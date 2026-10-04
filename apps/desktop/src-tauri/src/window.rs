//! The main window. `tauri.conf.json` describes it with `"create": false`, and it is built here
//! so its backdrop can depend on the Windows build (docs/design/macos-shell.md §8): Windows 11
//! 22H2 (build 22621) and later get a transparent window with Mica; Windows 10, Windows 11 21H2,
//! Linux and macOS get an opaque one, as before. Tauri ignores whether an effect took, so the
//! gate is ours. The page learns the result through an initialization script
//! (`window.__PAGELAMP_WINDOW__`, read by `src/lib/appearance.ts`), and paints its own paper
//! everywhere Mica isn't shown.
//!
//! The window starts hidden (`"visible": false`) and is shown once the page has loaded: by then
//! main.tsx has applied the student's theme, transparency and contrast, so the first frame is
//! never the system theme or a see-through Mica the student turned off.

use tauri::webview::PageLoadEvent;
use tauri::{App, Manager, Runtime, WebviewWindowBuilder};

/// What the page is told the window was built with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backdrop {
    None,
    Mica,
}

impl Backdrop {
    fn name(self) -> &'static str {
        match self {
            Backdrop::None => "none",
            Backdrop::Mica => "mica",
        }
    }
}

/// Runs in the page before any of its scripts. `hidden`: the window was started without being
/// shown to the student (a login start), so the page must not take its launch for the student
/// opening PageLamp (an automatic sync at such a start is never "attended").
pub fn init_script(backdrop: Backdrop, hidden: bool) -> String {
    format!(
        "window.__PAGELAMP_WINDOW__ = Object.freeze({{ backdrop: \"{}\", hidden: {hidden} }});",
        backdrop.name()
    )
}

/// Builds the "main" window from its `tauri.conf.json` entry.
pub fn create_main<R: Runtime>(app: &App<R>) -> tauri::Result<()> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .cloned()
        .expect("tauri.conf.json describes the main window");
    let builder = WebviewWindowBuilder::from_config(app.handle(), &config)?;
    let (builder, backdrop, os_build) = with_backdrop(builder);
    builder
        // This build has no hidden start: the window is always shown once the page has loaded.
        .initialization_script(init_script(backdrop, false))
        // Also after a failed load (Finished comes anyway), so the window never stays hidden.
        .on_page_load(|window, payload| {
            if payload.event() == PageLoadEvent::Finished
                && let Err(error) = window.show()
            {
                tracing::warn!(target: "pagelamp::window", %error, "show main window");
            }
        })
        .build()?;
    // The Windows build tells a tester's "no Mica" apart: gated (Windows 10, 21H2) or DWM's own
    // solid fallback (Battery Saver, transparency off, an inactive window).
    tracing::info!(
        target: "pagelamp::window",
        backdrop = backdrop.name(),
        os_build = ?os_build,
        "main window"
    );
    Ok(())
}

/// The builder with its backdrop, which backdrop that is, and the Windows build it depends on.
type WithBackdrop<'a, R, M> = (WebviewWindowBuilder<'a, R, M>, Backdrop, Option<u32>);

#[cfg(windows)]
fn with_backdrop<'a, R: Runtime, M: Manager<R>>(
    builder: WebviewWindowBuilder<'a, R, M>,
) -> WithBackdrop<'a, R, M> {
    use tauri::window::{Effect, EffectsBuilder};
    let build = windows_version::OsVersion::current().build;
    // Mica needs Windows 11 22H2 (build 22621); older builds would get a see-through window.
    if build >= 22_621 {
        let effects = EffectsBuilder::new().effect(Effect::Mica).build();
        (
            builder.transparent(true).effects(effects),
            Backdrop::Mica,
            Some(build),
        )
    } else {
        (builder, Backdrop::None, Some(build))
    }
}

#[cfg(not(windows))]
fn with_backdrop<'a, R: Runtime, M: Manager<R>>(
    builder: WebviewWindowBuilder<'a, R, M>,
) -> WithBackdrop<'a, R, M> {
    (builder, Backdrop::None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_is_told_the_backdrop() {
        assert_eq!(
            init_script(Backdrop::Mica, false),
            r#"window.__PAGELAMP_WINDOW__ = Object.freeze({ backdrop: "mica", hidden: false });"#
        );
        assert!(init_script(Backdrop::None, false).contains(r#"backdrop: "none""#));
        assert!(init_script(Backdrop::None, true).contains("hidden: true"));
    }

    fn shipped_config() -> serde_json::Value {
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json")
    }

    /// `create_main` expects exactly one "main" entry, and Tauri must not create it too (a lost
    /// `"create": false` would make two "main" windows and fail at startup). Transparency and
    /// effects are window.rs's decision: in the config they would bypass the Windows-build gate
    /// (a see-through window on Windows 10 and Linux). It starts hidden; the page shows it.
    #[test]
    fn the_shipped_config_leaves_the_main_window_to_us() {
        let config = shipped_config();
        let windows = config["app"]["windows"].as_array().expect("app.windows");
        let main: Vec<_> = windows.iter().filter(|w| w["label"] == "main").collect();
        assert_eq!(main.len(), 1, "one main window");
        assert_eq!(main[0]["create"], false, "built in window.rs, not by Tauri");
        assert_eq!(main[0]["visible"], false, "shown once the page has loaded");
        for key in ["transparent", "windowEffects"] {
            assert!(main[0].get(key).is_none(), "{key} is decided in window.rs");
        }
    }

    /// A JSON merge patch replaces arrays whole: `app.windows` in the rehearsal overlay would
    /// drop `"create": false` (and the rest) from the main window.
    #[test]
    fn the_rehearsal_overlay_keeps_the_windows() {
        let overlay: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.rehearsal.conf.json"))
                .expect("tauri.rehearsal.conf.json");
        assert!(overlay.pointer("/app/windows").is_none());
    }

    /// Runs `create_main` itself (mock runtime): the config entry, the builder and the
    /// initialization script together make exactly one "main" window.
    #[test]
    fn create_main_builds_the_one_main_window() {
        let windows: Vec<tauri::utils::config::WindowConfig> =
            serde_json::from_value(shipped_config()["app"]["windows"].clone())
                .expect("window configs");
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().app.windows = windows;
        let app = tauri::test::mock_builder()
            .build(context)
            .expect("mock app");
        create_main(&app).expect("create_main");
        assert_eq!(app.webview_windows().len(), 1);
        assert!(app.get_webview_window("main").is_some());
    }
}
