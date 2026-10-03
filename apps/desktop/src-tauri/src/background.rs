//! Running in the background (v0.3 M3; design §5.3), opt-in only: the student answers "Remind
//! me (keep PageLamp in the tray and start it at login)" in onboarding or Settings, stored as
//! `ReminderSettings.run_in_background`. On, PageLamp gets a tray icon, a login item that starts
//! it with `--hidden` (in the tray, no window), and the close button hides the window. Off (the
//! default), there is no tray and no login item, and closing the window quits, as before.
//!
//! The webview gets no autostart permission: it changes the setting through our commands, which
//! do the rest here. Login items: a LaunchAgent on macOS (never the AppleScript launcher, which
//! asks for the Automation permission), the per-user Run key on Windows, an XDG autostart entry
//! on Linux. One PageLamp runs at a time: opening it again shows the running one's window
//! (tauri-plugin-single-instance hands a second process over; on macOS the system doesn't start
//! one and sends the running app Reopen instead, see lib.rs).

use std::panic::AssertUnwindSafe;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use pagelamp_app::diagnostics::expect_panics;
use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem};
use tauri::plugin::TauriPlugin;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime, WebviewWindow, WindowEvent};
use tauri_plugin_autostart::AutoLaunchManager;

/// Given by the login item: start in the tray without opening the window.
pub const HIDDEN_ARG: &str = "--hidden";
const TRAY_ID: &str = "main";
const MENU_OPEN: &str = "open";
const MENU_QUIT: &str = "quit";

/// Whether this launch came from the login item.
pub fn started_hidden(mut args: impl Iterator<Item = String>) -> bool {
    args.any(|arg| arg == HIDDEN_ARG)
}

/// The autostart plugin, set up for our login item (enabled only through `apply`). On macOS the
/// LaunchAgent is named after the bundle `identifier` (reverse-DNS, as launchd expects), so a
/// build with another identifier (a test build) never touches the real one.
pub fn autostart_plugin<R: Runtime>(identifier: &str) -> TauriPlugin<R> {
    let builder = tauri_plugin_autostart::Builder::new().arg(HIDDEN_ARG);
    #[cfg(target_os = "macos")]
    let builder = builder
        .macos_launcher(tauri_plugin_autostart::MacosLauncher::LaunchAgent)
        .app_name(identifier);
    #[cfg(not(target_os = "macos"))]
    let builder = {
        let _ = identifier;
        builder.app_name(pagelamp_core::brand::PRODUCT_NAME)
    };
    builder.build()
}

/// One PageLamp at a time: a second launch shows the running one's window and quits, and a
/// second login launch (`--hidden`) just quits. `None` on Linux without a D-Bus session bus,
/// where the plugin can't work and would stop the app from starting.
pub fn single_instance_plugin<R: Runtime>() -> Option<TauriPlugin<R>> {
    #[cfg(target_os = "linux")]
    if !session_bus_available() {
        tracing::warn!(target: "pagelamp::background", "no D-Bus session bus: several PageLamps may run");
        return None;
    }
    Some(tauri_plugin_single_instance::init(|app, args, _cwd| {
        if !started_hidden(args.into_iter()) {
            show_main(app);
        }
    }))
}

/// Where zbus looks for the session bus: `DBUS_SESSION_BUS_ADDRESS`, else `$XDG_RUNTIME_DIR/bus`.
#[cfg(target_os = "linux")]
fn session_bus_available() -> bool {
    std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|a| !a.is_empty())
        || std::env::var_os("XDG_RUNTIME_DIR")
            .is_some_and(|dir| std::path::Path::new(&dir).join("bus").exists())
}

/// Just before an update installs: the next launch shows its window. Both ways back after an
/// update pass on this launch's arguments (`AppHandle::restart`, and the Windows installer's
/// relaunch with `/ARGS`), so after a login launch (`--hidden`) the updated app would otherwise
/// come back in the tray only, though the student just chose "Install and restart" in its window.
pub fn show_window_at_next_launch<R: Runtime>(app: &AppHandle<R>) {
    let marker = show_window_marker(&app.config().identifier);
    if let Err(error) = std::fs::write(&marker, b"") {
        tracing::warn!(target: "pagelamp::background", %error, "show-window marker");
    }
}

/// Whether this launch stays in the tray: `--hidden` (a login launch), unless an update asked for
/// the window a moment ago. The marker is removed either way.
pub fn launch_hidden(args: impl Iterator<Item = String>, identifier: &str) -> bool {
    let marker = show_window_marker(identifier);
    let asked = std::fs::metadata(&marker)
        .and_then(|meta| meta.modified())
        .is_ok_and(|at| at.elapsed().is_ok_and(|age| age < SHOW_WINDOW_FOR));
    let _ = std::fs::remove_file(&marker);
    started_hidden(args) && !asked
}

/// How long an update's "show the window" holds: installing and relaunching take seconds; a
/// login hours later stays in the tray.
const SHOW_WINDOW_FOR: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Per user and per build (the bundle identifier). The temp folder is the user's own on macOS and
/// Windows; on Linux it's the shared /tmp, where another user's marker couldn't be removed, so the
/// user's runtime folder comes first there.
fn show_window_marker(identifier: &str) -> std::path::PathBuf {
    #[cfg(target_os = "linux")]
    let folder = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(std::env::temp_dir);
    #[cfg(not(target_os = "linux"))]
    let folder = std::env::temp_dir();
    folder.join(format!("{identifier}.show-window"))
}

/// The tray menu's words, in the student's language (the page sends them; English until then).
#[derive(Clone, Debug, Deserialize)]
pub struct TrayLabels {
    pub open: String,
    pub quit: String,
}

impl Default for TrayLabels {
    fn default() -> Self {
        TrayLabels {
            open: format!("Open {}", pagelamp_core::brand::PRODUCT_NAME),
            quit: format!("Quit {}", pagelamp_core::brand::PRODUCT_NAME),
        }
    }
}

/// Managed state.
#[derive(Default)]
pub struct Background {
    /// `run_in_background` as last applied: the close button hides the window.
    on: AtomicBool,
    /// `on` follows the stored setting (read at launch, or once the core opened later).
    known: AtomicBool,
    /// The tray icon is shown (kept here: the tray list is the main thread's).
    tray_shown: AtomicBool,
    /// Launched by the login item and not shown yet: page loads keep the window hidden.
    hidden: AtomicBool,
    labels: Mutex<TrayLabels>,
    /// The tray couldn't be created (Linux without an AppIndicator library).
    tray_failed: AtomicBool,
    /// Saving the reminder settings and applying them happen one save at a time.
    pub(crate) settings_saves: tauri::async_runtime::Mutex<()>,
    /// Building and removing the tray happen one at a time, each deciding from `on` and
    /// `tray_shown` as they are then. Never taken on the main thread after launch: a build
    /// holding it waits for the main thread.
    tray_ops: Mutex<()>,
}

impl Background {
    pub fn new(started_hidden: bool) -> Self {
        let background = Background::default();
        background.hidden.store(started_hidden, Ordering::SeqCst);
        background
    }

    /// Whether the student said "Remind me" (as last applied): notifications may be shown.
    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::SeqCst)
    }

    /// Whether a page load may show the window.
    pub fn may_show(&self) -> bool {
        !self.hidden.load(Ordering::SeqCst)
    }
}

/// What the Settings row shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BackgroundStatus {
    pub run_in_background: bool,
    /// The tray icon is there (false while off, and where no tray can be shown).
    pub tray: bool,
    /// On, but this system can't show a tray icon.
    pub tray_unavailable: bool,
    /// The login item exists (the student may have removed it in the system's settings).
    pub login_item: bool,
}

/// Turns running in the background on or off after a save: the tray, the login item and the
/// close button. `was` is the stored setting before the save. The login item changes only when
/// the setting did: saving another reminder setting never brings back a login item the student
/// turned off in the system's settings (Windows would mark it enabled again). An error from the
/// login item is logged and shows in the status; turning the setting off and on tries again.
pub fn apply<R: Runtime>(app: &AppHandle<R>, was: bool, on: bool) -> BackgroundStatus {
    follow(app, after_save(was, on));
    status(app)
}

/// Once the stored setting can be read after a launch that couldn't (the core opened later):
/// `on` and the tray follow it, never the login item. Skipped while a save is being applied,
/// which sets it anyway.
pub fn sync_stored<R: Runtime>(app: &AppHandle<R>, stored_on: bool) {
    let state = app.state::<Background>();
    let Ok(_no_save_running) = state.settings_saves.try_lock() else {
        return;
    };
    if let Some(change) = after_read(state.known.load(Ordering::SeqCst), stored_on) {
        follow(app, change);
    }
}

/// What following the setting changes: `on` (the close button, notifications), the tray, and
/// the login item when `login_item` says so.
#[derive(Debug, PartialEq, Eq)]
struct Change {
    on: bool,
    login_item: Option<bool>,
}

/// After a save (`was` stored before it): the login item changes only with the setting.
fn after_save(was: bool, on: bool) -> Change {
    Change {
        on,
        login_item: (was != on).then_some(on),
    }
}

/// After the first read of a launch that couldn't read it at start: never the login item.
fn after_read(known: bool, stored_on: bool) -> Option<Change> {
    (!known).then_some(Change {
        on: stored_on,
        login_item: None,
    })
}

fn follow<R: Runtime>(app: &AppHandle<R>, change: Change) {
    let state = app.state::<Background>();
    state.on.store(change.on, Ordering::SeqCst);
    state.known.store(true, Ordering::SeqCst);
    let login_item = app.try_state::<AutoLaunchManager>();
    update_login_item(
        login_item.as_deref().map(|item| item as &dyn LoginItem),
        change.login_item,
    );
    if change.on {
        ensure_tray(app);
    } else {
        remove_tray(app);
    }
}

/// The login item as `apply` uses it: the autostart plugin's, or a fake in tests.
trait LoginItem {
    fn set(&self, on: bool) -> Result<(), String>;
}

impl LoginItem for AutoLaunchManager {
    fn set(&self, on: bool) -> Result<(), String> {
        if on { self.enable() } else { self.disable() }.map_err(|error| error.to_string())
    }
}

/// Turns the login item on or off when `turn` says so.
fn update_login_item(login_item: Option<&dyn LoginItem>, turn: Option<bool>) {
    if let (Some(login_item), Some(on)) = (login_item, turn)
        && let Err(error) = login_item.set(on)
    {
        tracing::warn!(target: "pagelamp::background", %error, on, "login item");
    }
}

/// At launch, before the page loads: follow the stored setting. Off, a login item left over from
/// an earlier setting is removed, and a launch it caused ends here. Unknown (the core can't
/// open), nothing is changed and a login launch ends quietly: the window would only show the
/// error, and the next login tries again.
pub fn start<R: Runtime>(app: &AppHandle<R>, stored_on: Option<bool>) {
    // The tray menu's handler, once for the app's lifetime (a handler given to each tray built
    // would stay registered after its tray goes, and run again for every later one).
    app.on_menu_event(|app, event| match event.id.as_ref() {
        MENU_OPEN => show_main(app),
        MENU_QUIT => app.exit(0),
        _ => {}
    });
    let state = app.state::<Background>();
    state.known.store(stored_on.is_some(), Ordering::SeqCst);
    if stored_on == Some(true) {
        state.on.store(true, Ordering::SeqCst);
        ensure_tray(app);
        // Started in the tray: no Dock icon until the window shows.
        #[cfg(target_os = "macos")]
        if !state.may_show() {
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
        }
        return;
    }
    if stored_on == Some(false)
        && let Some(login_item) = app.try_state::<AutoLaunchManager>()
        && login_item.is_enabled().unwrap_or(false)
    {
        let _ = login_item.disable();
    }
    if !state.may_show() {
        tracing::info!(target: "pagelamp::background", "started by a login item that is off: quitting");
        app.exit(0);
    }
}

pub fn status<R: Runtime>(app: &AppHandle<R>) -> BackgroundStatus {
    let state = app.state::<Background>();
    let on = state.on.load(Ordering::SeqCst);
    BackgroundStatus {
        run_in_background: on,
        tray: state.tray_shown.load(Ordering::SeqCst),
        tray_unavailable: on && state.tray_failed.load(Ordering::SeqCst),
        login_item: app
            .try_state::<AutoLaunchManager>()
            .is_some_and(|login_item| login_item.is_enabled().unwrap_or(false)),
    }
}

/// New tray menu words; the tray, if shown, gets them at once.
pub fn set_labels<R: Runtime>(app: &AppHandle<R>, labels: TrayLabels) {
    *app.state::<Background>()
        .labels
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = labels;
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        match tray_menu(app) {
            Ok(menu) => {
                let _ = tray.set_menu(Some(menu));
            }
            Err(error) => tracing::warn!(target: "pagelamp::background", %error, "tray menu"),
        }
    }
}

/// Debug builds only, to test that the app comes back exactly once after "Install and restart":
/// with `PAGELAMP_DEBUG_RESTART=<file>` and that file present, the app deletes the file and two
/// seconds after starting restarts the way an update does (off the main thread, window marker).
#[cfg(debug_assertions)]
pub fn debug_restart_once<R: Runtime>(app: &AppHandle<R>) {
    let Some(marker) = std::env::var_os("PAGELAMP_DEBUG_RESTART") else {
        return;
    };
    if std::fs::remove_file(&marker).is_err() {
        return;
    }
    tracing::info!(target: "pagelamp::background", "debug restart in 2 s");
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        show_window_at_next_launch(&app);
        crate::updates::restart(&app);
    });
}

/// Settings → Reminders: the tray and the login item as they are now.
#[tauri::command]
pub fn background_status<R: Runtime>(app: AppHandle<R>) -> BackgroundStatus {
    status(&app)
}

/// The tray menu in the student's language (sent by the page at start and on a language change).
#[tauri::command]
pub fn set_tray_labels<R: Runtime>(app: AppHandle<R>, labels: TrayLabels) {
    set_labels(&app, labels);
}

/// The main window's close button: hides it while running in the background.
pub fn watch_close<R: Runtime>(window: &WebviewWindow<R>) {
    let app = window.app_handle().clone();
    let target = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event
            && app.state::<Background>().on.load(Ordering::SeqCst)
        {
            api.prevent_close();
            hide_main(&target);
        }
    });
}

/// Shows and focuses the main window (tray, a second launch, macOS Reopen).
pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    app.state::<Background>()
        .hidden
        .store(false, Ordering::SeqCst);
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn hide_main<R: Runtime>(window: &WebviewWindow<R>) {
    let _ = window.hide();
    // No Dock icon while only the tray is there.
    #[cfg(target_os = "macos")]
    let _ = window
        .app_handle()
        .set_activation_policy(tauri::ActivationPolicy::Accessory);
}

/// Builds the tray if running in the background and it isn't there yet.
fn ensure_tray<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<Background>();
    let _one_at_a_time = state.tray_ops.lock().unwrap_or_else(|e| e.into_inner());
    if !state.is_on() || state.tray_shown.load(Ordering::SeqCst) {
        return;
    }
    if !tray_library_available() {
        tracing::info!(target: "pagelamp::background", "no AppIndicator library: no tray");
        state.tray_failed.store(true, Ordering::SeqCst);
        return;
    }
    // A backstop only (the probe above is the check): release builds unwind, never abort. It
    // catches a panic only where the tray is built on this thread: at launch (setup runs on the
    // main thread). From a command, Tauri builds it on the main thread and the probe is all.
    let built = expect_panics(|| std::panic::catch_unwind(AssertUnwindSafe(|| build_tray(app))));
    let failed = match built {
        Ok(Ok(())) => false,
        Ok(Err(error)) => {
            tracing::warn!(target: "pagelamp::background", %error, "tray");
            true
        }
        Err(_) => {
            tracing::warn!(target: "pagelamp::background", "no tray on this system");
            true
        }
    };
    state.tray_failed.store(failed, Ordering::SeqCst);
    state.tray_shown.store(!failed, Ordering::SeqCst);
}

/// Removes the tray icon on the main thread: its platform teardown (DestroyWindow, the status
/// bar item) must run there, and the tray list is the main thread's. Doesn't wait.
fn remove_tray<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<Background>();
    let _one_at_a_time = state.tray_ops.lock().unwrap_or_else(|e| e.into_inner());
    if !state.tray_shown.swap(false, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let _ = handle.remove_tray_by_id(TRAY_ID);
    });
}

/// The Windows installer is about to take over: remove the tray icon now, on the main thread,
/// and wait for it (briefly), so no dead icon stays behind. Nothing else is torn down: if the
/// installer can't start, PageLamp goes on as it was (`restore_tray`).
pub fn remove_tray_before_exit<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<Background>();
    let _one_at_a_time = state.tray_ops.lock().unwrap_or_else(|e| e.into_inner());
    if !state.tray_shown.swap(false, Ordering::SeqCst) {
        return;
    }
    let (done, wait) = std::sync::mpsc::channel();
    let handle = app.clone();
    let queued = app.run_on_main_thread(move || {
        let _ = handle.remove_tray_by_id(TRAY_ID);
        let _ = done.send(());
    });
    if queued.is_ok() {
        let _ = wait.recv_timeout(std::time::Duration::from_secs(2));
    }
}

/// After an install that didn't take over: the tray comes back if running in the background.
pub fn restore_tray<R: Runtime>(app: &AppHandle<R>) {
    ensure_tray(app);
}

/// Linux shows the tray through an AppIndicator library that the tray loads when it is built,
/// panicking without one: probe the names it tries, in its order.
#[cfg(target_os = "linux")]
fn tray_library_available() -> bool {
    const NAMES: [&str; 4] = [
        "libayatana-appindicator3.so.1",
        "libayatana-appindicator3.so",
        "libappindicator3.so.1",
        "libappindicator3.so",
    ];
    // SAFETY: the same libraries the tray loads next; loading runs only their initialisers.
    NAMES
        .iter()
        .any(|name| unsafe { libloading::Library::new(name) }.is_ok())
}

#[cfg(not(target_os = "linux"))]
fn tray_library_available() -> bool {
    true
}

fn build_tray<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(pagelamp_core::brand::PRODUCT_NAME)
        .menu(&tray_menu(app)?)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let tray = builder.build(app)?;
    // The tray list keeps it; this handle's (non-atomic) count goes down on the main thread.
    let _ = app.run_on_main_thread(move || drop(tray));
    Ok(())
}

fn tray_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let labels = app
        .state::<Background>()
        .labels
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let open = MenuItem::with_id(app, MENU_OPEN, labels.open, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, labels.quit, true, None::<&str>)?;
    Menu::with_items(app, &[&open, &quit])
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use std::cell::RefCell;

    use super::{
        Change, HIDDEN_ARG, LoginItem, after_read, after_save, launch_hidden, show_window_marker,
        started_hidden, update_login_item,
    };

    /// Records what `update_login_item` asked of the login item.
    #[derive(Default)]
    struct FakeLoginItem(RefCell<Vec<bool>>);

    impl LoginItem for FakeLoginItem {
        fn set(&self, on: bool) -> Result<(), String> {
            self.0.borrow_mut().push(on);
            Ok(())
        }
    }

    #[test]
    fn the_login_item_changes_only_with_the_stored_setting() {
        let item = FakeLoginItem::default();
        // Another reminder setting saved while stored on (or off): left as the system has it.
        update_login_item(Some(&item), after_save(true, true).login_item);
        update_login_item(Some(&item), after_save(false, false).login_item);
        assert!(item.0.borrow().is_empty());
        update_login_item(Some(&item), after_save(false, true).login_item);
        update_login_item(Some(&item), after_save(true, false).login_item);
        assert_eq!(*item.0.borrow(), vec![true, false]);
    }

    #[test]
    fn reading_the_setting_late_never_touches_the_login_item() {
        let item = FakeLoginItem::default();
        for stored in [true, false] {
            let change = after_read(false, stored).expect("not known yet");
            assert_eq!(
                change,
                Change {
                    on: stored,
                    login_item: None
                }
            );
            update_login_item(Some(&item), change.login_item);
        }
        assert!(item.0.borrow().is_empty());
        // Known already (read at launch, or a save applied): nothing to follow.
        assert_eq!(after_read(true, true), None);
    }

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn only_the_login_item_starts_hidden() {
        assert!(started_hidden(args(&[
            "/Applications/PageLamp",
            HIDDEN_ARG
        ])));
        assert!(!started_hidden(args(&["/Applications/PageLamp"])));
        assert!(!started_hidden(args(&[
            "/Applications/PageLamp",
            "--hidden=no"
        ])));
    }

    #[test]
    fn an_update_shows_the_window_of_the_next_login_launch_only() {
        // Its own marker name: tests may run while a real PageLamp uses the temp folder.
        let identifier = format!("dev.pagelamp.test-launch-hidden-{}", std::process::id());
        let marker = show_window_marker(&identifier);
        let login = || args(&["PageLamp", HIDDEN_ARG]);

        assert!(launch_hidden(login(), &identifier), "no marker: the tray");

        std::fs::write(&marker, b"").unwrap();
        assert!(
            !launch_hidden(login(), &identifier),
            "just updated: the window"
        );
        assert!(!marker.exists(), "used once");
        assert!(
            launch_hidden(login(), &identifier),
            "the next login: the tray again"
        );

        let file = std::fs::File::create(&marker).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(11 * 60))
            .unwrap();
        assert!(
            launch_hidden(login(), &identifier),
            "an old marker doesn't count"
        );
        assert!(!marker.exists(), "and is removed");

        std::fs::write(&marker, b"").unwrap();
        assert!(!launch_hidden(args(&["PageLamp"]), &identifier));
        assert!(!marker.exists(), "a normal launch removes it too");
    }
}
