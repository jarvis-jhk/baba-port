//! baba-port on Android: `libbabaport.so`, loaded by the platform's own
//! `android.app.NativeActivity` (no Java in the APK).
//!
//! `android_main` sets up logcat logging and the pure-Rust TLS provider,
//! opens the [`AndroidPlatform`](baba_platform::android::AndroidPlatform)
//! window, imports the player's game data if the app was opened with a zip
//! (`data`, a progress screen before the game starts), then runs the game's
//! own [`App`](baba_game::App) on that platform: the same title, menus, map
//! and levels as on desktop. Saves and settings live in the app's private
//! `files/save` (removed on uninstall).
//!
//! Input: the on-screen touch controls ([`screen::touch_quads`], shown
//! while playing) press the game's default keys; menus are tapped directly
//! (the controls are off while a menu is open); Back is the game's
//! Escape (pause in play, back in menus); gamepads use the game's pad
//! bindings, Select/Back of the pad acts as Escape too.
//!
//! Test switches (system properties, root `setprop`):
//! `debug.babaport.nettest 1` fetches the Warpzone server's public info over
//! HTTPS at start (TLS check); `debug.babaport.losectx 1` drops the GL
//! context whenever the window goes away (context-loss path).

pub mod crash;
pub mod screen;
pub mod zip_paths;

#[cfg(target_os = "android")]
mod data;
#[cfg(target_os = "android")]
mod logcat;

#[cfg(target_os = "android")]
mod entry {
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    use baba_game::{App, AppConfig};
    use baba_platform::android::{AndroidApp, AndroidPlatform, system_property};
    use baba_platform::{DeviceId, Event, Platform, WindowConfig};
    use baba_render::Renderer;

    use crate::{crash, data, logcat, screen};

    /// One HTTPS request to the Warpzone server's public info (no account,
    /// no identity file).
    fn net_check() {
        let t = Instant::now();
        let url = baba_net::warpzone_core::server_url();
        let api = baba_net::warpzone_core::api::Api::new(
            &url,
            std::sync::Arc::new(baba_net::warpzone_core::identity::Identity::generate()),
        );
        match api.info() {
            Ok(i) => log::info!(
                "nettest: HTTPS {url} OK in {} ms ({}): {} {} {}",
                t.elapsed().as_millis(),
                baba_net::tls::crypto_backend(),
                i.name,
                i.version,
                i.public_url
            ),
            Err(e) => log::error!("nettest: HTTPS {url} failed: {e}"),
        }
    }

    /// Imports the zip the app was opened with, showing progress.
    /// Returns the game directory, or None (failed / app closed).
    fn import(
        p: &mut AndroidPlatform,
        r: &mut Renderer,
        uri: String,
        file: std::fs::File,
        internal: PathBuf,
    ) -> Option<PathBuf> {
        let progress = data::Progress::default();
        let pr = progress.clone();
        let job = std::thread::spawn(move || data::import_zip(file, &internal, &pr));
        let mut events = Vec::new();
        let start = Instant::now();
        while !job.is_finished() {
            events.clear();
            p.poll_events(&mut events);
            if events.contains(&Event::Quit) {
                return None;
            }
            let (n, total) = (
                progress.done.load(Ordering::Relaxed),
                progress.total.load(Ordering::Relaxed).max(1),
            );
            screen::show_message(
                p,
                r,
                &format!("Importing the game data...\n\n{}%", n * 100 / total),
            );
            std::thread::sleep(Duration::from_millis(30));
        }
        match job.join() {
            Ok(Ok(dir)) => {
                log::info!(
                    "import of {uri} done in {:.1} s",
                    start.elapsed().as_secs_f32()
                );
                Some(dir)
            }
            Ok(Err(e)) => {
                log::error!("import of {uri} failed: {e}");
                None
            }
            Err(_) => {
                log::error!("import of {uri} panicked");
                None
            }
        }
    }

    #[unsafe(no_mangle)]
    fn android_main(app: AndroidApp) {
        logcat::init();
        log::info!(
            "baba-port {} (API {})",
            env!("CARGO_PKG_VERSION"),
            AndroidApp::sdk_version()
        );
        baba_net::tls::install_crypto_provider();
        if system_property("debug.babaport.nettest").as_deref() == Some("1") {
            std::thread::spawn(net_check);
        }
        let Some(dirs) = data::Dirs::of(&app) else {
            log::error!("no app data directory");
            return;
        };
        crash::init(
            dirs.internal.clone(),
            format!(
                "baba-port {} android {} (API {})",
                env!("CARGO_PKG_VERSION"),
                system_property("ro.product.model").unwrap_or_default(),
                AndroidApp::sdk_version()
            ),
        );
        let intent = data::intent_file(&app);
        let (mut p, gl) = match AndroidPlatform::new(app, &WindowConfig::default()) {
            Ok(x) => x,
            Err(e) => {
                log::error!("{e}");
                crash::report("Android start failed (window)", &e.to_string());
                return;
            }
        };
        let mut r = match Renderer::new(gl) {
            Ok(r) => r,
            Err(e) => {
                log::error!("renderer: {e}");
                crash::report("Android start failed (renderer)", &e.to_string());
                return;
            }
        };
        // Screens of our own until the App runs: no touch controls.
        if let Some(t) = p.touch_controls_mut() {
            t.set_enabled(false);
        }
        let mut game = dirs.find_game();
        if let Some((uri, file)) = intent {
            game = import(&mut p, &mut r, uri, file, dirs.internal.clone()).or(game);
        }
        let Some(game) = game else {
            log::warn!(
                "no game data: open a zip of the game with baba-port, or copy it to {}",
                dirs.external
                    .as_ref()
                    .map(|e| e.join("game").display().to_string())
                    .unwrap_or_default()
            );
            screen::message_loop(&mut p, &mut r, screen::NO_DATA);
            return;
        };
        log::info!("game data: {}", game.display());
        screen::show_message(&mut p, &mut r, "Loading...");
        let cfg = AppConfig {
            game_dir: game,
            data_dir: Some(dirs.internal.join("save")),
            skip_boot: false,
        };
        let mut app = match App::with_renderer(r, cfg) {
            Ok(a) => a,
            Err(e) => {
                log::error!("game: {e}");
                crash::report("Android start failed (game)", &e.to_string());
                return;
            }
        };
        for d in p.devices() {
            if matches!(d.id, DeviceId::Controller(_)) {
                app.handle_event(&Event::ControllerAdded(d));
            }
        }
        let (mut frames, mut last_log) = (0u64, Duration::ZERO);
        loop {
            let touch = app.wants_touch_controls();
            if let Some(t) = p.touch_controls_mut() {
                t.set_enabled(touch);
            }
            if !app.frame(&mut p) {
                break;
            }
            if let Some(t) = p.touch_controls()
                && t.visible()
            {
                let q = screen::touch_quads(&t.items());
                app.renderer().draw_ui(&q);
            }
            p.swap_buffers();
            frames += 1;
            let t = p.time();
            if t >= last_log + Duration::from_secs(10) {
                let (w, h) = p.drawable_size();
                log::info!("frame {frames}, {w}x{h}, {}", app.state());
                last_log = t;
            }
        }
        log::info!("android_main returns");
    }
}
