//! Text prompts outside the editor (slot names, level codes, online room
//! codes): typing on a keyboard and the on-screen keyboard ([`crate::osk`])
//! for pads and touch.

use super::App;
use super::extras::{ExtAct, ExtMenu};
use super::online::{OnAct, OnMenu};
use crate::input::Cmd;
use crate::menu::MenuId;
use crate::osk::{self, Charset, OskKey};

impl App {
    /// The open prompt's text, its length limit and charset.
    pub(super) fn prompt_text(&mut self) -> Option<(&mut String, usize, Charset)> {
        match self.menus.last_mut() {
            Some((MenuId::Rename { text, .. }, _)) => Some((text, 16, Charset::Name)),
            Some((MenuId::Ext(ExtMenu::EnterCode { text }), _)) => Some((text, 9, Charset::Code)),
            Some((MenuId::Online(OnMenu::Code { text }), _)) => Some((text, 12, Charset::Code)),
            _ => None,
        }
    }

    /// Typing, Enter / Start (submit), B (delete; cancel when empty),
    /// Escape / Select (cancel), and the on-screen keys through the menu
    /// grid. True if a prompt is open (it took the frame).
    pub(super) fn prompt_tick(&mut self, cmds: &[Cmd]) -> bool {
        if self.prompt_text().is_none() {
            return false;
        }
        for c in std::mem::take(&mut self.input.typed) {
            if c == '\n' {
                self.prompt_submit();
                return true;
            }
            if let Some((text, max, cs)) = self.prompt_text() {
                osk::type_char(text, max, cs, c);
            }
        }
        for &c in cmds {
            match c {
                Cmd::Back => self.osk_key(OskKey::Cancel),
                Cmd::Undo => {
                    let empty = self.prompt_text().is_some_and(|(t, ..)| t.is_empty());
                    self.osk_key(if empty {
                        OskKey::Cancel
                    } else {
                        OskKey::Delete
                    });
                }
                Cmd::Pause => self.osk_key(OskKey::Done),
                Cmd::Restart => self.osk_key(OskKey::Shift),
                Cmd::Dir(_) | Cmd::Confirm => self.menu_cmd(c),
                _ => {}
            }
            if self.prompt_text().is_none() {
                break;
            }
        }
        true
    }

    /// One key of the on-screen keyboard (or its pad shortcut).
    pub(super) fn osk_key(&mut self, k: OskKey) {
        match k {
            OskKey::Char(c) => {
                let shift = std::mem::take(&mut self.osk_shift);
                if let Some((text, max, cs)) = self.prompt_text() {
                    let c = if shift { c.to_ascii_uppercase() } else { c };
                    osk::type_char(text, max, cs, c);
                }
            }
            OskKey::Delete => {
                if let Some((text, ..)) = self.prompt_text() {
                    text.pop();
                }
            }
            OskKey::Shift => {
                if self
                    .prompt_text()
                    .is_some_and(|(.., cs)| cs == Charset::Name)
                {
                    self.osk_shift = !self.osk_shift;
                }
            }
            OskKey::Done => self.prompt_submit(),
            OskKey::Cancel => {
                self.osk_shift = false;
                self.menus.pop();
            }
        }
    }

    fn prompt_submit(&mut self) {
        self.osk_shift = false;
        match self.menus.last() {
            Some((MenuId::Rename { .. }, _)) => {
                if let Some((MenuId::Rename { slot, text }, _)) = self.menus.pop()
                    && !text.trim().is_empty()
                {
                    self.profile.settings.set_slot_name(slot, text.trim());
                    self.profile.save_settings();
                }
            }
            Some((MenuId::Ext(ExtMenu::EnterCode { .. }), _)) => {
                self.ext_activate(ExtAct::SubmitCode)
            }
            Some((MenuId::Online(OnMenu::Code { .. }), _)) => {
                self.online_activate(OnAct::SubmitCode)
            }
            _ => {}
        }
    }
}
