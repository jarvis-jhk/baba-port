//! Where the player's game data lives on Android, and importing it from a
//! zip the player opens with (or shares to) baba-port. No Java: the intent
//! and the content URI are read through JNI (`getIntent`,
//! `ContentResolver.openFileDescriptor`), the zip is unpacked in Rust.
//! See `docs/platform-android.md` ("Game data").

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::fd::FromRawFd;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use baba_platform::android::AndroidApp;
use baba_platform::android::jni::{self, Arg};

use crate::zip_paths;

/// The app's own directories.
pub struct Dirs {
    /// `/data/user/0/<package>/files` (private; removed on uninstall).
    pub internal: PathBuf,
    /// `/storage/emulated/0/Android/data/<package>/files` (reachable over
    /// USB/adb; removed on uninstall).
    pub external: Option<PathBuf>,
}

impl Dirs {
    pub fn of(app: &AndroidApp) -> Option<Dirs> {
        Some(Dirs {
            internal: app.internal_data_path()?,
            external: app.external_data_path(),
        })
    }

    /// Imported data first, then a folder copied to the external files dir.
    pub fn find_game(&self) -> Option<PathBuf> {
        std::iter::once(self.internal.join("game"))
            .chain(self.external.iter().map(|e| e.join("game")))
            .find(|d| d.join("Data").join("values.lua").is_file())
    }
}

/// The file the activity was opened with (VIEW) or sent (SEND), as an open
/// file plus its URI for logs. Marks the intent as consumed.
pub fn intent_file(app: &AndroidApp) -> Option<(String, File)> {
    let act = app.activity_as_ptr();
    let (uri, fd) = jni::with_env(app.vm_as_ptr(), |e| {
        // The activity outlives this call.
        let act = unsafe { e.wrap(act) };
        let intent = e
            .call_obj(act, c"getIntent", c"()Landroid/content/Intent;", &[])
            .filter(|i| !i.is_null())?;
        let action = e
            .call_obj(intent, c"getAction", c"()Ljava/lang/String;", &[])
            .and_then(|s| e.string(s))
            .unwrap_or_default();
        let mut uri = e.call_obj(intent, c"getData", c"()Landroid/net/Uri;", &[])?;
        if uri.is_null() && action == "android.intent.action.SEND" {
            let key = e.new_string("android.intent.extra.STREAM")?;
            uri = e.call_obj(
                intent,
                c"getParcelableExtra",
                c"(Ljava/lang/String;)Landroid/os/Parcelable;",
                &[Arg::Obj(key)],
            )?;
        }
        if uri.is_null() {
            return None;
        }
        let text = e
            .call_obj(uri, c"toString", c"()Ljava/lang/String;", &[])
            .and_then(|s| e.string(s))
            .unwrap_or_default();
        log::info!("intent {action}: {text}");
        let resolver = e.call_obj(
            act,
            c"getContentResolver",
            c"()Landroid/content/ContentResolver;",
            &[],
        )?;
        let mode = e.new_string("r")?;
        let pfd = e
            .call_obj(
                resolver,
                c"openFileDescriptor",
                c"(Landroid/net/Uri;Ljava/lang/String;)Landroid/os/ParcelFileDescriptor;",
                &[Arg::Obj(uri), Arg::Obj(mode)],
            )
            .filter(|p| !p.is_null())?;
        let fd = e.call_int(pfd, c"detachFd", c"()I", &[])?;
        // Consumed: a re-created activity must not import again.
        let done = e.new_string("org.babaport.action.IMPORTED")?;
        e.call_obj(
            intent,
            c"setAction",
            c"(Ljava/lang/String;)Landroid/content/Intent;",
            &[Arg::Obj(done)],
        );
        Some((text, fd))
    })?;
    (fd >= 0).then(|| (uri, unsafe { File::from_raw_fd(fd) }))
}

/// What the Warpzone launcher asked for: the launch intent's string-array
/// extra `warpzone.args` (the desktop command line: `--warpzone-join`,
/// `--warpzone-host`, `--warpzone-code`), `warpzone.launched` and
/// `warpzone.server`. Removes `warpzone.args` from the intent, so a
/// re-created activity does not join again.
#[derive(Debug, Default)]
pub struct WarpzoneLaunch {
    pub args: Vec<String>,
    pub launched: bool,
    pub server: Option<String>,
}

pub fn warpzone_launch(app: &AndroidApp) -> WarpzoneLaunch {
    let act = app.activity_as_ptr();
    jni::with_env(app.vm_as_ptr(), |e| {
        let act = unsafe { e.wrap(act) };
        let intent = e
            .call_obj(act, c"getIntent", c"()Landroid/content/Intent;", &[])
            .filter(|i| !i.is_null())?;
        let key = e.new_string("warpzone.args")?;
        let args = e
            .call_obj(
                intent,
                c"getStringArrayExtra",
                c"(Ljava/lang/String;)[Ljava/lang/String;",
                &[Arg::Obj(key)],
            )
            .and_then(|a| e.string_array(a))
            .unwrap_or_default();
        if !args.is_empty() {
            e.call_void(
                intent,
                c"removeExtra",
                c"(Ljava/lang/String;)V",
                &[Arg::Obj(key)],
            );
        }
        let key = e.new_string("warpzone.launched")?;
        let launched = e
            .call_bool(
                intent,
                c"getBooleanExtra",
                c"(Ljava/lang/String;Z)Z",
                &[Arg::Obj(key), Arg::Bool(false)],
            )
            .unwrap_or(false);
        let key = e.new_string("warpzone.server")?;
        let server = e
            .call_obj(
                intent,
                c"getStringExtra",
                c"(Ljava/lang/String;)Ljava/lang/String;",
                &[Arg::Obj(key)],
            )
            .and_then(|s| e.string(s))
            .filter(|s| !s.is_empty());
        Some(WarpzoneLaunch {
            args,
            launched,
            server,
        })
    })
    .unwrap_or_default()
}

/// Progress of an import (entries written / total), for the UI.
#[derive(Clone, Default)]
pub struct Progress {
    pub done: Arc<AtomicUsize>,
    pub total: Arc<AtomicUsize>,
}

/// A relative path with no `..`, root or prefix components.
fn safe(rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    p.components()
        .all(|c| matches!(c, Component::Normal(_)))
        .then(|| p.to_path_buf())
}

/// Unpacks the game's `Data/` (+ `Assets.dat`, `gamecontrollerdb.txt` and
/// the executable for the sound names) from a zip of any build's install
/// (any folder depth; a macOS `.app` too, see `zip_paths`) into
/// `<internal>/game`.
pub fn import_zip(file: File, internal: &Path, progress: &Progress) -> Result<PathBuf, String> {
    let mut file = file;
    // Content providers may hand out pipes; zip needs to seek.
    let seekable = file.seek(SeekFrom::End(0)).is_ok() && file.rewind().is_ok();
    let tmp_zip = internal.join("import.zip");
    if !seekable {
        let mut out = File::create(&tmp_zip).map_err(|e| e.to_string())?;
        std::io::copy(&mut file, &mut out).map_err(|e| e.to_string())?;
        file = File::open(&tmp_zip).map_err(|e| e.to_string())?;
    }
    let result = unpack(file, internal, progress);
    let _ = std::fs::remove_file(&tmp_zip);
    result
}

fn unpack(file: File, internal: &Path, progress: &Progress) -> Result<PathBuf, String> {
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a zip: {e}"))?;
    let names: Vec<String> = zip
        .file_names()
        .filter_map(|n| n.ok().map(|n| n.into_owned()))
        .collect();
    let prefix = zip_paths::data_prefix(names.iter().map(String::as_str))
        .ok_or("the zip has no Data/values.lua: zip the game's folder (with Data inside)")?;
    log::info!("importing game data (folder {prefix:?} in the zip)");
    progress.total.store(zip.len(), Ordering::Relaxed);
    let tmp = internal.join("game.import");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let mut bytes = 0u64;
    for i in 0..zip.len() {
        progress.done.store(i + 1, Ordering::Relaxed);
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?.into_owned();
        let Some(rel) = zip_paths::target(&name, &prefix) else {
            continue;
        };
        let Some(rel) = safe(rel.trim_end_matches('/')) else {
            continue;
        };
        let dest = tmp.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = File::create(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
        bytes += std::io::copy(&mut (&mut entry as &mut dyn Read), &mut out)
            .map_err(|e| format!("{}: {e}", dest.display()))?;
    }
    if !tmp.join("Data").join("values.lua").is_file() {
        return Err("import incomplete (no Data/values.lua written)".into());
    }
    let game = internal.join("game");
    let _ = std::fs::remove_dir_all(&game);
    std::fs::rename(&tmp, &game).map_err(|e| e.to_string())?;
    log::info!(
        "imported {:.1} MB into {}",
        bytes as f64 / 1e6,
        game.display()
    );
    Ok(game)
}
