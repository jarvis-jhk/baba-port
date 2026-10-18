//! Which entries of a zip of the player's game are imported, and where
//! they go (platform-neutral, so it is tested on the host; the unpacking
//! itself is in `data.rs`). Any build's folder works:
//!
//! | Zipped | Folder holding `Data/` | Executable (sound names) |
//! |---|---|---|
//! | Linux / Steam Deck install | the install | `bin64/Chowdren` |
//! | Windows install | the install | `Baba Is You.exe` (any `*.exe` next to `Data/`) |
//! | macOS `Baba Is You.app` (or a folder with it) | `….app/Contents/Resources/` | `….app/Contents/MacOS/Chowdren` → `MacOS/Chowdren` |
//!
//! The imported folder keeps `Data/**`, `Assets.dat`, `gamecontrollerdb.txt`
//! and the executable (read only for the sound effects' names,
//! `baba_formats::exe`); `baba_formats::GameInstall::executables` looks in
//! those places.

/// A zip entry name with `/` separators (some Windows tools write `\`).
pub fn normalize(name: &str) -> String {
    name.replace('\\', "/")
}

/// The folder inside the zip that holds `Data/values.lua` (`""` for the
/// zip's root, else ending in `/`), the shallowest one if several.
pub fn data_prefix<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<String> {
    names
        .into_iter()
        .map(normalize)
        .filter_map(|n| n.strip_suffix("Data/values.lua").map(str::to_string))
        .filter(|p| p.is_empty() || p.ends_with('/'))
        .filter(|p| !p.starts_with("__MACOSX/"))
        .min_by_key(|p| p.len())
}

/// Where entry `name` goes in the imported game folder (relative path), or
/// `None` if it is not needed. `prefix` comes from [`data_prefix`].
pub fn target(name: &str, prefix: &str) -> Option<String> {
    let name = normalize(name);
    // macOS bundle: the executable sits beside Resources/.
    if let Some(contents) = prefix.strip_suffix("Resources/")
        && contents.ends_with("Contents/")
        && name == format!("{contents}MacOS/Chowdren")
    {
        return Some("MacOS/Chowdren".into());
    }
    let rel = name.strip_prefix(prefix)?;
    let lower = rel.to_ascii_lowercase();
    let windows_exe = !rel.contains('/') && lower.ends_with(".exe") && !lower.starts_with("unins");
    let keep = rel.starts_with("Data/")
        || rel == "Assets.dat"
        || rel == "gamecontrollerdb.txt"
        || rel == "bin64/Chowdren"
        || windows_exe;
    keep.then(|| rel.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_zip() {
        let names = [
            "Baba Is You/",
            "Baba Is You/Data/values.lua",
            "Baba Is You/Data/Worlds/baba/1level.l",
            "Baba Is You/Assets.dat",
            "Baba Is You/bin64/Chowdren",
            "Baba Is You/bin64/libsteam_api.so",
            "Baba Is You/run.sh",
        ];
        let p = data_prefix(names).unwrap();
        assert_eq!(p, "Baba Is You/");
        let kept: Vec<_> = names.iter().filter_map(|n| target(n, &p)).collect();
        assert_eq!(
            kept,
            [
                "Data/values.lua",
                "Data/Worlds/baba/1level.l",
                "Assets.dat",
                "bin64/Chowdren"
            ]
        );
    }

    #[test]
    fn windows_zip_with_backslashes() {
        let names = [
            "Data\\values.lua",
            "Data\\Music\\baba.ogg",
            "Assets.dat",
            "Baba Is You.exe",
            "dxvk.dll",
            "steam_api64.dll",
            "XAudio2_7.dll",
            "unins000.exe",
            "gamecontrollerdb.txt",
        ];
        let p = data_prefix(names).unwrap();
        assert_eq!(p, "");
        let kept: Vec<_> = names.iter().filter_map(|n| target(n, &p)).collect();
        assert_eq!(
            kept,
            [
                "Data/values.lua",
                "Data/Music/baba.ogg",
                "Assets.dat",
                "Baba Is You.exe",
                "gamecontrollerdb.txt"
            ]
        );
    }

    #[test]
    fn mac_app_zip() {
        let names = [
            "__MACOSX/Baba Is You.app/Contents/Resources/Data/._values.lua",
            "__MACOSX/Baba Is You.app/Contents/Resources/Data/values.lua",
            "Baba Is You.app/Contents/Info.plist",
            "Baba Is You.app/Contents/MacOS/Chowdren",
            "Baba Is You.app/Contents/MacOS/libsteam_api.dylib",
            "Baba Is You.app/Contents/Resources/Assets.dat",
            "Baba Is You.app/Contents/Resources/Data/values.lua",
            "Baba Is You.app/Contents/Resources/gamecontrollerdb.txt",
            "Baba Is You.app/Contents/Resources/icon.icns",
        ];
        let p = data_prefix(names).unwrap();
        assert_eq!(p, "Baba Is You.app/Contents/Resources/");
        let kept: Vec<_> = names.iter().filter_map(|n| target(n, &p)).collect();
        assert_eq!(
            kept,
            [
                "MacOS/Chowdren",
                "Assets.dat",
                "Data/values.lua",
                "gamecontrollerdb.txt"
            ]
        );
    }

    #[test]
    fn no_game() {
        assert_eq!(data_prefix(["readme.txt", "MyData/values.lua"]), None);
    }
}
