//! Sound effects by name. The effects are Ogg files inside `Assets.dat`,
//! which stores them by index only; the names live in the game's
//! executable (Linux `bin64/Chowdren`, Windows `Baba Is You.exe`, macOS
//! `Contents/MacOS/Chowdren`) as a string hash table with a parallel index
//! array (`baba_formats::exe`). Both files belong to the player's install
//! and are read at run time, nothing is copied into the port. When the
//! names cannot be recovered (no executable), effects stay silent.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use baba_formats::{AssetKind, AssetsDat};
use baba_platform::audio::{SoundData, decode_ogg};

/// Loaded sound effects, decoded on first use.
pub struct SoundBank {
    assets: Option<Arc<AssetsDat>>,
    /// Sound entry index (among sounds) by name.
    index: HashMap<String, usize>,
    decoded: HashMap<usize, Option<Arc<SoundData>>>,
}

impl SoundBank {
    pub fn empty() -> Self {
        Self {
            assets: None,
            index: HashMap::new(),
            decoded: HashMap::new(),
        }
    }

    pub fn load(game_dir: &Path) -> Self {
        let mut bank = Self::empty();
        let assets = match AssetsDat::load(&game_dir.join("Assets.dat")) {
            Ok(a) => a,
            Err(e) => {
                log::warn!("no sound effects: {e}");
                return bank;
            }
        };
        let count = assets.of_kind(AssetKind::Sound).count();
        let exes = baba_formats::install::executables_in(game_dir);
        let named = exes.iter().find_map(|exe| {
            let bytes = std::fs::read(exe)
                .inspect_err(|e| log::warn!("{}: {e}", exe.display()))
                .ok()?;
            let names = baba_formats::exe::sound_names(&bytes, count);
            if names.is_none() {
                log::warn!("sound names not found in {}", exe.display());
            }
            Some((exe, names?))
        });
        match named {
            Some((exe, names)) => {
                for (i, n) in names.into_iter().enumerate() {
                    bank.index.insert(n, i);
                }
                log::info!("{count} sound effects named from {}", exe.display());
            }
            None => log::warn!(
                "no game executable with sound names in {} (sound effects stay silent)",
                game_dir.display()
            ),
        }
        bank.assets = Some(Arc::new(assets));
        bank
    }

    /// The index among `Assets.dat`'s sounds a name maps to (tests).
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.index.get(name).copied()
    }

    /// The opened `Assets.dat` (shared with the engine-art loader).
    pub fn assets(&self) -> Option<Arc<AssetsDat>> {
        self.assets.clone()
    }

    pub fn has(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// The decoded sound, if it exists.
    pub fn get(&mut self, name: &str) -> Option<Arc<SoundData>> {
        let &i = self.index.get(name)?;
        if let Some(d) = self.decoded.get(&i) {
            return d.clone();
        }
        let assets = self.assets.as_ref()?;
        let entry = assets.of_kind(AssetKind::Sound).nth(i)?;
        let data = assets
            .sound(entry)
            .ok()
            .and_then(|s| decode_ogg(std::io::Cursor::new(s.data.to_vec())).ok())
            .map(Arc::new);
        if data.is_none() {
            log::warn!("sound {name}: decode failed");
        }
        self.decoded.insert(i, data.clone());
        data
    }
}
