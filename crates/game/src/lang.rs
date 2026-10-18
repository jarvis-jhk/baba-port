//! Language files and text lookup (`docs/spec/ui.md` §9): the current
//! language with English as fallback, `$s` / `__` values, level and map
//! names, pack names.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use baba_formats::Language;

/// One language file, indexed.
pub struct Lang {
    pub code: String,
    pub name: String,
    /// Uses the TrueType font (`customfont=1`).
    pub ttf: bool,
    texts: HashMap<String, String>,
}

impl Lang {
    fn load(path: &Path) -> Option<Lang> {
        let l = Language::load(path).ok()?;
        let texts = l
            .texts()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Some(Lang {
            code: l.code.clone(),
            name: l.name().to_string(),
            ttf: l.custom_font(),
            texts,
        })
    }

    fn raw(&self, key: &str) -> Option<&str> {
        self.texts
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }
}

/// Every language of the install, the current one and English.
pub struct Texts {
    pub dir: PathBuf,
    /// `(code, display name)` of every language file, sorted by code.
    pub available: Vec<(String, String)>,
    pub current: Lang,
    english: Option<Lang>,
}

impl Texts {
    /// Loads `Data/Languages`, selecting `code` (falls back to English).
    pub fn load(data_dir: &Path, code: &str) -> Texts {
        let dir = data_dir.join("Languages");
        let mut available: Vec<(String, String)> = baba_formats::world::list_files(&dir, "txt")
            .iter()
            .filter_map(|p| Lang::load(p).map(|l| (l.code.clone(), l.name.clone())))
            .collect();
        available.sort();
        let english = Lang::load(&dir.join("lang_en.txt"));
        let current = Lang::load(&dir.join(format!("lang_{code}.txt")))
            .or_else(|| Lang::load(&dir.join("lang_en.txt")))
            .unwrap_or(Lang {
                code: "en".into(),
                name: "english".into(),
                ttf: false,
                texts: HashMap::new(),
            });
        Texts {
            dir,
            available,
            current,
            english,
        }
    }

    /// Switches language; false if the file is missing.
    pub fn switch(&mut self, code: &str) -> bool {
        match Lang::load(&self.dir.join(format!("lang_{code}.txt"))) {
            Some(l) => {
                self.current = l;
                true
            }
            None => false,
        }
    }

    pub fn is_english(&self) -> bool {
        self.current.code == "en"
    }

    /// §9.2 lookup: current language, then English, then the key itself
    /// marked as missing. `$s` = the key, `__` = empty.
    pub fn get(&self, key: &str) -> String {
        self.opt(key).unwrap_or_else(|| format!("({key})"))
    }

    /// Like [`Texts::get`] but `None` when missing everywhere.
    pub fn opt(&self, key: &str) -> Option<String> {
        let v = self
            .current
            .raw(key)
            .or_else(|| self.english.as_ref().and_then(|e| e.raw(key)))?;
        Some(match v {
            "$s" => key.to_string(),
            "__" => String::new(),
            v => v.to_string(),
        })
    }

    /// Only the current language (no English fallback).
    pub fn own(&self, key: &str) -> Option<String> {
        self.current.raw(key).map(str::to_string)
    }

    /// Displayed level name (`progression.md` §2.2): the `.ld` name unless
    /// a non-English language has a text for it.
    pub fn level_name(&self, world: &str, stem: &str, ld_name: &str) -> String {
        if !self.is_english() {
            let key = if world == "baba" {
                stem.to_string()
            } else {
                format!("{world}_{stem}")
            };
            if let Some(v) = self.own(&key) {
                return v;
            }
        }
        ld_name.to_string()
    }

    /// Map id override (`<stem>_mapid` / `<world>_<stem>_mapid`), any
    /// language.
    pub fn map_id(&self, world: &str, stem: &str) -> Option<String> {
        let key = if world == "baba" {
            format!("{stem}_mapid")
        } else {
            format!("{world}_{stem}_mapid")
        };
        self.opt(&key)
    }

    /// Pack display name (`ui.md` §3.3).
    pub fn pack_name(&self, folder: &str, data_name: &str) -> String {
        if let Some(n) = self.opt(&format!("world_{folder}")) {
            return n;
        }
        if !data_name.is_empty() {
            return data_name.to_string();
        }
        self.opt("customlevels_unnamed")
            .unwrap_or_else(|| "unnamed levelpack".into())
    }
}
