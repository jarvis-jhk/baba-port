//! SDL virtual controllers for scripted runs (`drive.rs`): SDL's
//! joystick and game-controller subsystems only (no window, no video
//! driver), virtual joysticks plugged in and out, buttons pressed, and
//! the resulting SDL events turned into platform events in the order SDL
//! delivers them, exactly like `baba_platform::sdl` does for real pads.

use std::collections::BTreeMap;

use baba_platform::{Axis, Button, DeviceId, DeviceInfo, Event};
use sdl2::controller::{Axis as SdlAxis, Button as SdlButton, GameController};
use sdl2::event::Event as SdlEvent;

/// Virtual-controller hub.
pub struct PadHub {
    _sdl: sdl2::Sdl,
    gc: sdl2::GameControllerSubsystem,
    pump: sdl2::EventPump,
    open: BTreeMap<u32, GameController>,
    /// Instance ids of our virtual pads in plug order (`None` = unplugged).
    pub virt: Vec<Option<u32>>,
}

fn map_button(b: SdlButton) -> Button {
    use SdlButton as B;
    match b {
        B::A => Button::South,
        B::B => Button::East,
        B::X => Button::West,
        B::Y => Button::North,
        B::Back => Button::Back,
        B::Guide => Button::Guide,
        B::Start => Button::Start,
        B::LeftStick => Button::LeftStick,
        B::RightStick => Button::RightStick,
        B::LeftShoulder => Button::LeftShoulder,
        B::RightShoulder => Button::RightShoulder,
        B::DPadUp => Button::DPadUp,
        B::DPadDown => Button::DPadDown,
        B::DPadLeft => Button::DPadLeft,
        B::DPadRight => Button::DPadRight,
        B::Misc1 => Button::Misc,
        other => Button::Other(other as u8),
    }
}

/// SDL game-controller button number by name (virtual joysticks map
/// button n to controller button n).
pub fn button_number(name: &str) -> Option<u8> {
    Some(match name {
        "a" => 0,
        "b" => 1,
        "x" => 2,
        "y" => 3,
        "back" | "select" => 4,
        "guide" => 5,
        "start" => 6,
        "ls" => 7,
        "rs" => 8,
        "lb" => 9,
        "rb" => 10,
        "up" => 11,
        "down" => 12,
        "left" => 13,
        "right" => 14,
        _ => return None,
    })
}

impl PadHub {
    pub fn new() -> Result<PadHub, String> {
        let sdl = sdl2::init()?;
        let gc = sdl.game_controller()?;
        let pump = sdl.event_pump()?;
        Ok(PadHub {
            _sdl: sdl,
            gc,
            pump,
            open: BTreeMap::new(),
            virt: Vec::new(),
        })
    }

    /// Plugs in a virtual controller (it arrives with the next `poll`).
    /// Virtual pads plugged in so far (unplugged ones included).
    pub fn count(&self) -> usize {
        self.virt.len()
    }

    pub fn attach(&mut self) -> Result<(), String> {
        use sdl2::sys::{SDL_JoystickAttachVirtual, SDL_JoystickType};
        // SAFETY: plain SDL call; the joystick subsystem is initialised.
        let i = unsafe {
            SDL_JoystickAttachVirtual(SDL_JoystickType::SDL_JOYSTICK_TYPE_GAMECONTROLLER, 6, 15, 0)
        };
        if i < 0 {
            return Err(sdl2::get_error());
        }
        Ok(())
    }

    /// Unplugs our virtual pad `k` (plug order).
    pub fn detach(&mut self, k: usize) -> Result<(), String> {
        let id = self
            .virt
            .get(k)
            .copied()
            .flatten()
            .ok_or_else(|| format!("no virtual pad {k}"))?;
        let n = self.gc.num_joysticks()?;
        for idx in 0..n {
            // SAFETY: plain SDL calls with a valid device index.
            unsafe {
                if sdl2::sys::SDL_JoystickGetDeviceInstanceID(idx as i32) == id as i32 {
                    if sdl2::sys::SDL_JoystickDetachVirtual(idx as i32) < 0 {
                        return Err(sdl2::get_error());
                    }
                    self.virt[k] = None;
                    return Ok(());
                }
            }
        }
        Err(format!("virtual pad {k} not found"))
    }

    /// Presses or releases a button of our virtual pad `k`.
    pub fn set(&mut self, k: usize, button: u8, down: bool) -> Result<(), String> {
        let id = self
            .virt
            .get(k)
            .copied()
            .flatten()
            .ok_or_else(|| format!("no virtual pad {k}"))?;
        self.set_by_id(id, button, down)
    }

    /// Presses or releases a button of the virtual pad with instance `id`.
    pub fn set_by_id(&mut self, id: u32, button: u8, down: bool) -> Result<(), String> {
        // SAFETY: the instance id belongs to a joystick we keep open.
        unsafe {
            let j = sdl2::sys::SDL_JoystickFromInstanceID(id as i32);
            if j.is_null()
                || sdl2::sys::SDL_JoystickSetVirtualButton(j, button as i32, down as u8) < 0
            {
                return Err(sdl2::get_error());
            }
        }
        Ok(())
    }

    /// Pumps SDL and returns the controller events.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        let events: Vec<SdlEvent> = self.pump.poll_iter().collect();
        for ev in events {
            match ev {
                SdlEvent::ControllerDeviceAdded { which, .. } => {
                    // SAFETY: plain SDL call with a device index from SDL.
                    let virt = unsafe { sdl2::sys::SDL_JoystickIsVirtual(which as i32) }
                        == sdl2::sys::SDL_bool::SDL_TRUE;
                    if let Ok(c) = self.gc.open(which) {
                        let id = c.instance_id();
                        out.push(Event::ControllerAdded(DeviceInfo {
                            id: DeviceId::Controller(id),
                            name: c.name(),
                            guid: String::new(),
                            mapped: true,
                        }));
                        self.open.insert(id, c);
                        if virt {
                            self.virt.push(Some(id));
                        }
                    }
                }
                SdlEvent::ControllerDeviceRemoved { which, .. } => {
                    if self.open.remove(&which).is_some() {
                        out.push(Event::ControllerRemoved(DeviceId::Controller(which)));
                    }
                }
                SdlEvent::ControllerButtonDown { which, button, .. } => out.push(Event::Button {
                    device: DeviceId::Controller(which),
                    button: map_button(button),
                    down: true,
                }),
                SdlEvent::ControllerButtonUp { which, button, .. } => out.push(Event::Button {
                    device: DeviceId::Controller(which),
                    button: map_button(button),
                    down: false,
                }),
                SdlEvent::ControllerAxisMotion {
                    which, axis, value, ..
                } => {
                    let axis = match axis {
                        SdlAxis::LeftX => Axis::LeftX,
                        SdlAxis::LeftY => Axis::LeftY,
                        SdlAxis::RightX => Axis::RightX,
                        SdlAxis::RightY => Axis::RightY,
                        SdlAxis::TriggerLeft => Axis::TriggerLeft,
                        SdlAxis::TriggerRight => Axis::TriggerRight,
                    };
                    out.push(Event::Axis {
                        device: DeviceId::Controller(which),
                        axis,
                        value: (value as f32 / 32767.0).clamp(-1.0, 1.0),
                    });
                }
                _ => {}
            }
        }
        out
    }
}
