//! On-screen keyboard for the text prompts outside the editor (slot names,
//! level codes, online room codes), so they can be filled in with a pad
//! or by touch (Android, PortMaster) as well as with a keyboard. The keys
//! are ordinary menu items ([`Act::Osk`]): the menu grid moves the
//! selection, Confirm / a tap presses a key. A physical keyboard types
//! straight into the text; Enter submits.

use crate::menu::{Act, Item, Menu};

/// What a prompt accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Charset {
    /// Level and room codes: `A`–`Z`, digits and `-`, upper case.
    Code,
    /// Slot names: letters, digits and spaces, with a shift key.
    Name,
}

/// One key of the on-screen keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OskKey {
    Char(char),
    Delete,
    Shift,
    Done,
    Cancel,
}

const ROWS: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl-", "zxcvbnm"];

impl Charset {
    /// Whether `c` may go into the text (after [`Charset::fold`]).
    pub fn accepts(self, c: char) -> bool {
        match self {
            Charset::Code => c.is_ascii_alphanumeric() || c == '-',
            Charset::Name => c.is_alphanumeric() || c == ' ',
        }
    }

    /// The character as stored: codes are upper case.
    pub fn fold(self, c: char) -> char {
        match self {
            Charset::Code => c.to_ascii_uppercase(),
            Charset::Name => c,
        }
    }
}

/// Applies one typed character (`'\u{8}'` = backspace) to `text`; other
/// characters only if the charset takes them and `max` is not reached.
pub fn type_char(text: &mut String, max: usize, cs: Charset, c: char) {
    if c == '\u{8}' {
        text.pop();
        return;
    }
    let c = cs.fold(c);
    if cs.accepts(c) && text.chars().count() < max {
        text.push(c);
    }
}

/// Appends the keyboard rows to `m`: digits and letters (upper case with
/// `shift` or for codes), then Shift/Space (names), Delete, OK and Cancel.
pub fn add_rows(m: &mut Menu, cs: Charset, shift: bool, ok: &str, cancel: &str) {
    // Rows of 10 keys 552 wide: room for the cursor on 4:3 screens.
    m.min_width = 270.0;
    for row in ROWS {
        let keys = row
            .chars()
            .filter(|&c| cs.accepts(c))
            .map(|c| {
                let c = if shift || cs == Charset::Code {
                    c.to_ascii_uppercase()
                } else {
                    c
                };
                Item::new(c.to_string(), Act::Osk(OskKey::Char(c)))
            })
            .collect();
        m.row(keys);
    }
    let mut last = Vec::new();
    if cs == Charset::Name {
        last.push(Item::new("Shift", Act::Osk(OskKey::Shift)).on(shift));
        last.push(Item::new("Space", Act::Osk(OskKey::Char(' '))));
    }
    last.push(Item::new("Del", Act::Osk(OskKey::Delete)));
    last.push(Item::new(ok, Act::Osk(OskKey::Done)));
    last.push(Item::new(cancel, Act::Osk(OskKey::Cancel)));
    m.row(last);
    m.footer
        .push("Enter / Start: OK    Undo: delete    Escape / Select: cancel".into());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::Style;

    #[test]
    fn codes_are_upper_case_and_limited() {
        let mut t = String::new();
        for c in "38tc-1f2g!x".chars() {
            type_char(&mut t, 9, Charset::Code, c);
        }
        assert_eq!(t, "38TC-1F2G");
        type_char(&mut t, 9, Charset::Code, '\u{8}');
        assert_eq!(t, "38TC-1F2");
    }

    #[test]
    fn names_keep_case_and_spaces() {
        let mut t = String::new();
        for c in "Baba 2-x".chars() {
            type_char(&mut t, 16, Charset::Name, c);
        }
        assert_eq!(t, "Baba 2x");
    }

    #[test]
    fn rows_fit_the_charset() {
        let mut m = Menu::new(Style::List);
        add_rows(&mut m, Charset::Code, false, "OK", "Cancel");
        let labels: Vec<String> = m.rows.iter().flatten().map(|i| i.label.clone()).collect();
        assert!(labels.contains(&"Q".to_string()) && labels.contains(&"-".to_string()));
        assert!(!labels.contains(&"Space".to_string()));
        let mut m = Menu::new(Style::List);
        add_rows(&mut m, Charset::Name, false, "OK", "Cancel");
        let labels: Vec<String> = m.rows.iter().flatten().map(|i| i.label.clone()).collect();
        assert!(labels.contains(&"q".to_string()) && !labels.contains(&"-".to_string()));
        assert!(labels.contains(&"Space".to_string()));
    }
}
