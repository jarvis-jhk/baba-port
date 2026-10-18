//! Turns: every action that changes a game instance (sim level, level
//! tree, save progress) as a small value with a byte encoding. Local play
//! applies them through [`crate::play::Play::apply`]; online play (M7)
//! sends the same bytes through `baba-net` streams (`docs/net.md`, "What
//! the game must provide": ≤ 64 bytes per turn).
//!
//! Presentation (transitions, title cards, banners, menus, the victory
//! delay) never changes an instance; it only decides *when* the local
//! machine submits a turn. For example the victory banner runs, then the
//! closing transition, and at full cover the presenter submits
//! [`Turn::Proceed`].

/// One turn of one instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Turn {
    /// A direction 0–3 (right, up, left, down) or 4 = wait, driving YOU
    /// (`you` = 1) or YOU2 (`you` = 2, falling back to YOU).
    Move {
        dir: u8,
        you: u8,
    },
    Undo,
    /// Reload the current level (also maps).
    Restart,
    /// Enter `target` from the map icon `unit` (`progression.md` §7.1).
    Enter {
        target: String,
        unit: i32,
    },
    /// Return to the parent map without winning (§7.3).
    Leave,
    /// Carry out what a win, a level conversion or an ending left pending
    /// (load the parent map with the unlock animation, convert, credits).
    Proceed,
    /// Shared save online (mode 2): read the map's icon and path states
    /// again from the save, which other players' progress changed
    /// (`Play::refresh_map`). Made by the player's own machine so every
    /// machine refreshes at the same point of the player's stream.
    Refresh,
    /// The AUTO timer fired (LEVEL IS AUTO, `turn.md` §3): one auto turn
    /// with the directions stored since the last one. The presenter submits
    /// it every `rhythm` + 1 frames while the level's AUTO holds (measured).
    Auto,
}

const MOVE: u8 = 1;
const UNDO: u8 = 2;
const RESTART: u8 = 3;
const ENTER: u8 = 4;
const LEAVE: u8 = 5;
const PROCEED: u8 = 6;
const REFRESH: u8 = 7;
const AUTO: u8 = 8;

/// Longest level stem an `Enter` may carry (the encoding stays ≤ 64 bytes).
pub const MAX_TARGET: usize = 58;

impl Turn {
    /// Plain wait for player 1.
    pub fn wait() -> Turn {
        Turn::Move { dir: 4, you: 1 }
    }

    /// The bytes sent over the network / kept in replays.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Turn::Move { dir, you } => vec![MOVE, (*dir & 7) | ((*you & 3) << 4)],
            Turn::Undo => vec![UNDO],
            Turn::Restart => vec![RESTART],
            Turn::Enter { target, unit } => {
                let t = &target.as_bytes()[..target.len().min(MAX_TARGET)];
                let mut v = Vec::with_capacity(6 + t.len());
                v.push(ENTER);
                v.extend_from_slice(&unit.to_le_bytes());
                v.push(t.len() as u8);
                v.extend_from_slice(t);
                v
            }
            Turn::Leave => vec![LEAVE],
            Turn::Proceed => vec![PROCEED],
            Turn::Refresh => vec![REFRESH],
            Turn::Auto => vec![AUTO],
        }
    }

    /// Inverse of [`Turn::encode`]; `None` for malformed input.
    pub fn decode(b: &[u8]) -> Option<Turn> {
        let (&tag, rest) = b.split_first()?;
        Some(match (tag, rest) {
            (MOVE, [x]) => {
                let dir = x & 7;
                let you = (x >> 4) & 3;
                if dir > 4 || !(1..=2).contains(&you) {
                    return None;
                }
                Turn::Move { dir, you }
            }
            (UNDO, []) => Turn::Undo,
            (RESTART, []) => Turn::Restart,
            (ENTER, r) if r.len() >= 5 => {
                let unit = i32::from_le_bytes(r[..4].try_into().ok()?);
                let n = r[4] as usize;
                let t = r.get(5..5 + n)?;
                if r.len() != 5 + n {
                    return None;
                }
                Turn::Enter {
                    target: String::from_utf8(t.to_vec()).ok()?,
                    unit,
                }
            }
            (LEAVE, []) => Turn::Leave,
            (PROCEED, []) => Turn::Proceed,
            (REFRESH, []) => Turn::Refresh,
            (AUTO, []) => Turn::Auto,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let all = [
            Turn::Move { dir: 0, you: 1 },
            Turn::Move { dir: 4, you: 2 },
            Turn::Undo,
            Turn::Restart,
            Turn::Enter {
                target: "106level".into(),
                unit: -7,
            },
            Turn::Leave,
            Turn::Proceed,
            Turn::Refresh,
            Turn::Auto,
        ];
        for t in all {
            let b = t.encode();
            assert!(b.len() <= 64);
            assert_eq!(Turn::decode(&b), Some(t));
        }
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(Turn::decode(&[]), None);
        assert_eq!(Turn::decode(&[MOVE, 5 | (1 << 4)]), None);
        assert_eq!(Turn::decode(&[MOVE, 1]), None);
        assert_eq!(Turn::decode(&[UNDO, 0]), None);
        assert_eq!(Turn::decode(&[ENTER, 0, 0, 0, 0, 3, b'a']), None);
        assert_eq!(Turn::decode(&[99]), None);
    }
}
