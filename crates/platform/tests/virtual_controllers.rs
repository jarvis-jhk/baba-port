//! Hot-plug and multi-controller events through SDL virtual joysticks.
//! Needs a video driver for the window (the private Xvfb, or SDL's
//! `offscreen` driver); skips without one unless `BABA_REQUIRE_SDL=1`.

use baba_platform::sdl::SdlPlatform;
use baba_platform::{Button, DeviceId, Event, Platform, WindowConfig};

fn poll(p: &mut SdlPlatform) -> Vec<Event> {
    let mut all = Vec::new();
    for _ in 0..20 {
        let mut ev = Vec::new();
        p.poll_events(&mut ev);
        all.extend(ev);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    all
}

#[test]
fn hotplug_buttons_and_removal() {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("SDL_VIDEODRIVER").is_none() {
        // SAFETY: single-threaded at this point of the test.
        unsafe { std::env::set_var("SDL_VIDEODRIVER", "offscreen") };
    }
    let cfg = WindowConfig {
        width: 64,
        height: 64,
        audio: false,
        ..Default::default()
    };
    let (mut p, _gl) = match SdlPlatform::new(&cfg) {
        Ok(v) => v,
        Err(e) => {
            assert!(std::env::var("BABA_REQUIRE_SDL").is_err(), "{e}");
            eprintln!("skipping: {e}");
            return;
        }
    };
    let before = p.devices().len();
    let a = p.attach_virtual_controller().unwrap();
    let _b = p.attach_virtual_controller().unwrap();
    let ev = poll(&mut p);
    let added: Vec<DeviceId> = ev
        .iter()
        .filter_map(|e| match e {
            Event::ControllerAdded(i) => Some(i.id),
            _ => None,
        })
        .collect();
    assert_eq!(added.len(), 2, "{ev:?}");
    assert_eq!(p.devices().len(), before + 2);
    p.set_virtual_button(added[1], 0, true).unwrap();
    let ev = poll(&mut p);
    assert!(
        ev.contains(&Event::Button {
            device: added[1],
            button: Button::South,
            down: true
        }),
        "{ev:?}"
    );
    p.detach_virtual_controller(a).unwrap();
    let ev = poll(&mut p);
    assert!(ev.contains(&Event::ControllerRemoved(added[0])), "{ev:?}");
    assert_eq!(p.devices().len(), before + 1);
}
