//! Keyboard key-cap images of the original (engine art, `ui.md` §6.4),
//! identified on captures: a rounded cap with the key's label, 24×24 with
//! the hot spot at the centre. Used by tutorial control hints and the
//! undo/restart hint.

/// The cap image for an SDL key name (`crate::input::key_name`).
pub fn image(key: &str) -> Option<u32> {
    let k = key.to_ascii_lowercase();
    Some(match k.as_str() {
        "left" => 843,
        "right" => 844,
        "up" => 845,
        "down" => 846,
        "return" | "enter" => 863,
        "backspace" => 864,
        "escape" | "esc" => 859,
        "left shift" | "right shift" | "shift" => 860,
        "left ctrl" | "right ctrl" | "ctrl" | "control" => 857,
        "r" => 1091,
        "space" => 1158,
        s if s.len() == 1 => {
            let c = s.chars().next()?;
            const LETTERS: [(char, u32); 25] = [
                ('b', 808),
                ('a', 809),
                ('c', 810),
                ('e', 811),
                ('d', 812),
                ('f', 813),
                ('h', 814),
                ('g', 815),
                ('i', 816),
                ('j', 817),
                ('l', 818),
                ('k', 819),
                ('n', 820),
                ('m', 821),
                ('o', 822),
                ('p', 823),
                ('w', 824),
                ('q', 825),
                ('s', 826),
                ('t', 827),
                ('u', 828),
                ('v', 829),
                ('x', 830),
                ('z', 831),
                ('y', 832),
            ];
            const DIGITS: [(char, u32); 10] = [
                ('1', 833),
                ('2', 834),
                ('3', 835),
                ('5', 836),
                ('4', 837),
                ('8', 838),
                ('6', 839),
                ('7', 840),
                ('0', 841),
                ('9', 842),
            ];
            LETTERS
                .iter()
                .chain(DIGITS.iter())
                .find(|(ch, _)| *ch == c)
                .map(|(_, n)| *n)?
        }
        _ => return None,
    })
}
