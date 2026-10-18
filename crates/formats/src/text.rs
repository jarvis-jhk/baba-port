//! Themes (`Themes/<n>theme.txt`) and language files
//! (`Data/Languages/lang_<code>.txt`).

use std::path::Path;

use crate::error::Result;
use crate::ini::{Ini, parse_int};
use crate::objects::TileChanges;

/// An editor theme: a palette/music/particles preset plus slot overrides.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Theme {
    /// The raw INI.
    pub ini: Ini,
    /// Display name.
    pub name: String,
    /// Palette file name.
    pub palette: String,
    /// Palette in `Data/Palettes` (true) or the world's `Palettes/`.
    pub palette_root: bool,
    /// Music track.
    pub music: String,
    /// Particle effect.
    pub particles: String,
    /// Object slot overrides (`[tiles]`).
    pub tiles: TileChanges,
}

impl Theme {
    /// Parses theme text.
    pub fn parse(text: &str) -> Theme {
        let ini = Ini::parse(text);
        let s = |k: &str| ini.get("general", k).unwrap_or("").to_string();
        Theme {
            name: s("name"),
            palette: s("palette"),
            palette_root: ini
                .get("general", "paletteroot")
                .and_then(parse_int)
                .unwrap_or(1)
                != 0,
            music: s("music"),
            particles: s("particles"),
            tiles: TileChanges::from_ini(&ini),
            ini,
        }
    }

    /// Reads a theme file.
    pub fn load(path: &Path) -> Result<Theme> {
        Ok(Theme::parse(&crate::error::read_text(path)?))
    }
}

/// A language file: `[general]` (name, `customfont`) and `[texts]` (id → text).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Language {
    /// Code from the file name (`en` for `lang_en.txt`), if known.
    pub code: String,
    /// The raw INI.
    pub ini: Ini,
}

impl Language {
    /// Parses language text.
    pub fn parse(code: &str, text: &str) -> Language {
        Language {
            code: code.to_string(),
            ini: Ini::parse(text),
        }
    }

    /// Reads `lang_<code>.txt`; the code is taken from the file name.
    pub fn load(path: &Path) -> Result<Language> {
        let code = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix("lang_"))
            .unwrap_or("")
            .to_string();
        Ok(Language::parse(&code, &crate::error::read_text(path)?))
    }

    /// Display name of the language.
    pub fn name(&self) -> &str {
        self.ini.get("general", "name").unwrap_or(&self.code)
    }

    /// Whether the language needs its own font (`customfont`).
    pub fn custom_font(&self) -> bool {
        self.ini
            .get("general", "customfont")
            .and_then(parse_int)
            .unwrap_or(0)
            != 0
    }

    /// A text by id.
    pub fn text(&self, id: &str) -> Option<&str> {
        self.ini.get("texts", id)
    }

    /// All `(id, text)` pairs.
    pub fn texts(&self) -> Vec<(&str, &str)> {
        self.ini.entries("texts")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_and_lang() {
        let t = Theme::parse(
            "[general]\rname=Under\rpalette=ocean.png\rparticles=bubbles\r[tiles]\rchanged=object004,\robject004_name=baba\r",
        );
        assert_eq!(t.name, "Under");
        assert!(t.palette_root);
        assert_eq!(t.tiles.changes.len(), 1);
        let l = Language::parse(
            "de",
            "[general]\nname=Deutsch\ncustomfont=0\n[texts]\nmain_start=spiel starten\n",
        );
        assert_eq!(l.name(), "Deutsch");
        assert!(!l.custom_font());
        assert_eq!(l.text("main_start"), Some("spiel starten"));
        assert_eq!(l.texts().len(), 1);
    }
}
