//! In-app updates (v0.3 M0.4). Rust only: the webview has no `updater:*` permission, so it can
//! neither install nor redirect an update; it calls the three commands below.
//!
//! - The channel (stable/beta) and whether a check is due come from the facade
//!   (`App::effective_update_channel`, `startup_tasks`); every check is recorded there (codes
//!   only) for the diagnostic report.
//! - Endpoints are static URLs from `tauri.conf.json` `plugins.pagelamp-updates` (no
//!   `{{target}}`/`{{arch}}`/`{{current_version}}` templating), and requests carry a minimal
//!   `User-Agent: PageLamp/<version>`. So GitHub sees the IP address and the app version, as
//!   with any download, and nothing else. Each channel asks raw.githubusercontent.com first
//!   (the `gh-pages` branch itself), then GitHub Pages: the updater moves to the next endpoint
//!   on a network error or an error status, but stops at an answer that isn't JSON, so the
//!   endpoint that depends on no domain comes first. The rehearsal overlay
//!   (`tauri.rehearsal.conf.json`) swaps the endpoints at build time; nothing can change them
//!   at runtime.
//! - Versions are compared with the real crate version (`CARGO_PKG_VERSION`, e.g.
//!   `0.3.0-beta.2`), not `tauri.conf.json`'s numeric one, so betas are offered the release.
//! - deb/rpm installs never auto-install: they read the AppImage entry only to learn the new
//!   version and get a link to the release page (the manifest has no deb/rpm keys, by design).

use std::future::Future;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use pagelamp_app::{
    Activity, ActivityKind, AppError, AppErrorKind, UpdateChannel, UpdateCheckOutcome,
    UpdateCheckRecord,
};
use pagelamp_core::brand;
use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::utils::config::BundleType;
use tauri::{AppHandle, Manager, Runtime, State, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::backend::{Backend, InstallGate, internal};

type CmdResult<T> = Result<T, AppError>;

/// The manifest entry deb/rpm installs read to learn the new version (never installed).
const DOWNLOAD_ONLY_TARGET: &str = "linux-x86_64-appimage";
/// A check or download that hangs shouldn't hold the UI forever.
const CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// The real version of this build.
pub fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate version is valid semver")
}

/// The channel when the facade can't say (it can't open, e.g. a database written by a newer
/// PageLamp, which is exactly when an update is needed): the facade's own default rule (D3),
/// beta for a pre-release build, else stable.
pub fn default_channel(version: &Version) -> UpdateChannel {
    if version.pre.is_empty() {
        UpdateChannel::Stable
    } else {
        UpdateChannel::Beta
    }
}

/// Whether `remote` is newer than `current` by full semver, pre-releases included:
/// `0.3.0-beta.2 < 0.3.0`, and an equal version is not an update.
pub fn is_newer(current: &Version, remote: &Version) -> bool {
    remote > current
}

/// How this build updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallMode {
    InApp,
    /// deb/rpm: installing would need the package manager (and a password), so the app only
    /// links to the release page.
    DownloadOnly,
}

pub fn install_mode() -> InstallMode {
    match tauri::utils::platform::bundle_type() {
        Some(BundleType::Deb | BundleType::Rpm) => InstallMode::DownloadOnly,
        _ => InstallMode::InApp,
    }
}

#[derive(Debug, Serialize)]
pub struct UpdaterStatus {
    pub current_version: String,
    pub install: InstallMode,
    pub platform: &'static str,
}

#[derive(Debug, Serialize)]
pub struct AvailableUpdate {
    pub version: String,
    pub date: Option<String>,
    pub notes: Option<String>,
    /// The release page, for download-only installs.
    pub download_url: Option<String>,
}

/// Progress of "Install and restart", streamed to the UI.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UpdateEvent {
    DownloadStarted {
        total_bytes: Option<u64>,
    },
    Progress {
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
    },
    Installing,
    Restarting,
}

/// The update the last check found (what "Install and restart" installs), and its package once
/// downloaded: kept when work that started during the download held the install back, so trying
/// again installs without downloading again.
#[derive(Default)]
pub struct PendingUpdate {
    update: Mutex<Option<Update>>,
    package: Mutex<Option<Package>>,
}

/// A downloaded package; `Update::download` has checked its signature.
struct Package {
    /// The update it belongs to (`package_key`).
    key: String,
    bytes: Vec<u8>,
}

fn package_key(update: &Update) -> String {
    format!("{} {}", update.version, update.download_url)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `tauri.conf.json` → `plugins.pagelamp-updates`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChannelsConfig {
    stable: Vec<Url>,
    beta: Vec<Url>,
    /// `{version}` is replaced by the new version.
    release_page: String,
}

fn channels_config<R: Runtime>(app: &AppHandle<R>) -> CmdResult<ChannelsConfig> {
    let value = app
        .config()
        .plugins
        .0
        .get("pagelamp-updates")
        .cloned()
        .ok_or_else(|| internal("this build has no update channels configured"))?;
    serde_json::from_value(value).map_err(|err| internal(format!("update channels: {err}")))
}

fn endpoints(config: ChannelsConfig, channel: &UpdateChannel) -> Vec<Url> {
    match channel {
        UpdateChannel::Stable => config.stable,
        UpdateChannel::Beta => config.beta,
    }
}

/// A short code for the diagnostic report (never a message or URL).
fn error_code(err: &tauri_plugin_updater::Error) -> &'static str {
    use tauri_plugin_updater::Error as E;
    match err {
        E::Reqwest(_) | E::Network(_) | E::Http(_) => "network",
        E::Minisign(_)
        | E::Base64(_)
        | E::SignatureUtf8(_)
        | E::SignedVersionMismatch { .. }
        | E::MissingSignedVersion => "signature",
        E::ReleaseNotFound
        | E::TargetNotFound(_)
        | E::TargetsNotFound(_)
        | E::Serialization(_)
        | E::Semver(_)
        | E::UrlParse(_)
        | E::EmptyEndpoints => "manifest",
        E::Io(_)
        | E::BinaryNotFoundInArchive
        | E::InvalidUpdaterFormat
        | E::TempDirNotFound
        | E::TempDirNotOnSameMountPoint
        | E::FailedToDetermineExtractPath => "install",
        _ => "other",
    }
}

fn app_error(err: &tauri_plugin_updater::Error) -> AppError {
    let kind = match error_code(err) {
        "network" => AppErrorKind::Network,
        _ => AppErrorKind::Internal,
    };
    AppError::new(kind, err.to_string())
}

/// A failed check, as the window hears it. For a test version on Stable, `ReleaseNotFound` (every
/// address answered, none with update information) means no stable release has any yet. The
/// window says that in its own words, so it gets its own kind; the diagnostic report keeps the
/// "manifest" code.
///
/// Only for a pre-release build: a stable build was itself installed from a stable release, so
/// for it the same answer is a real failure (an outage, a rate limit, a proxy in the way).
fn check_error(
    err: &tauri_plugin_updater::Error,
    channel: UpdateChannel,
    version: &Version,
) -> AppError {
    match (err, channel) {
        (tauri_plugin_updater::Error::ReleaseNotFound, UpdateChannel::Stable)
            if !version.pre.is_empty() =>
        {
            AppError::new(AppErrorKind::NotFound, err.to_string())
        }
        _ => app_error(err),
    }
}

#[tauri::command]
pub fn updates_status() -> UpdaterStatus {
    UpdaterStatus {
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        install: install_mode(),
        platform: std::env::consts::OS,
    }
}

/// Checks the effective channel and records the outcome through the facade.
#[tauri::command]
pub async fn updates_check<R: Runtime>(
    app: AppHandle<R>,
    backend: State<'_, Backend>,
    pending: State<'_, PendingUpdate>,
) -> CmdResult<Option<AvailableUpdate>> {
    let channel = match backend
        .blocking(|facade| facade.effective_update_channel())
        .await
    {
        Ok(channel) => channel,
        Err(err) => {
            tracing::info!(target: "pagelamp::updates", "no stored channel ({}); using the default", err.message);
            default_channel(&current_version())
        }
    };
    let config = channels_config(&app)?;
    let release_page = config.release_page.clone();
    let mode = install_mode();
    let user_agent = format!("{}/{}", brand::PRODUCT_NAME, env!("CARGO_PKG_VERSION"));

    let mut builder = app
        .updater_builder()
        .endpoints(endpoints(config, &channel))
        .map_err(|err| app_error(&err))?
        .header("User-Agent", user_agent)
        .map_err(|err| app_error(&err))?
        .timeout(CHECK_TIMEOUT)
        // Windows: the installer takes over from here (Install is refused during syncs).
        .on_before_exit(
            || tracing::info!(target: "pagelamp::updates", "exiting for the installer"),
        );
    if mode == InstallMode::DownloadOnly {
        builder = builder.target(DOWNLOAD_ONLY_TARGET);
    }
    let result = match builder.build() {
        Ok(updater) => updater.check().await,
        Err(err) => Err(err),
    };

    let outcome = match &result {
        Ok(Some(update)) => UpdateCheckOutcome::Available {
            version: update.version.clone(),
        },
        Ok(None) => UpdateCheckOutcome::UpToDate,
        Err(err) => {
            tracing::warn!(target: "pagelamp::updates", code = error_code(err), "update check failed: {err}");
            UpdateCheckOutcome::Error {
                code: error_code(err).to_string(),
            }
        }
    };
    let record = UpdateCheckRecord {
        at: chrono::Utc::now(),
        channel,
        outcome,
    };
    if let Err(err) = backend
        .blocking(move |facade| facade.record_update_check(record))
        .await
    {
        tracing::warn!(target: "pagelamp::updates", "couldn't record the update check: {}", err.message);
    }

    let found = result.map_err(|err| check_error(&err, channel, &current_version()))?;
    let mut slot = lock(&pending.update);
    // A package kept for the same update stays; any other is dropped.
    let key = found.as_ref().map(package_key);
    lock(&pending.package).take_if(|package| Some(&package.key) != key.as_ref());
    let Some(update) = found else {
        *slot = None;
        return Ok(None);
    };
    let available = AvailableUpdate {
        version: update.version.clone(),
        date: update
            .raw_json
            .get("pub_date")
            .and_then(|date| date.as_str())
            .map(str::to_string),
        notes: update.body.clone(),
        download_url: (mode == InstallMode::DownloadOnly)
            .then(|| release_page.replace("{version}", &update.version)),
    };
    *slot = (mode == InstallMode::InApp).then_some(update);
    Ok(Some(available))
}

/// Downloads, verifies and installs the update the last check found, then restarts. Only
/// called after the student confirmed; refused while anything runs that the restart would kill.
#[tauri::command]
pub async fn updates_install<R: Runtime>(
    app: AppHandle<R>,
    backend: State<'_, Backend>,
    pending: State<'_, PendingUpdate>,
    on_event: Channel<UpdateEvent>,
) -> CmdResult<()> {
    if install_mode() == InstallMode::DownloadOnly {
        return Err(AppError::new(
            AppErrorKind::Invalid,
            "This install updates by downloading the new package.",
        ));
    }
    let update = lock(&pending.update)
        .clone()
        .ok_or_else(|| AppError::new(AppErrorKind::NotFound, "Check for updates first."))?;

    let mut downloaded: u64 = 0;
    let mut started = false;
    let progress = on_event.clone();
    let installing = on_event.clone();
    let download = update.download(
        move |chunk, total| {
            if !started {
                started = true;
                let _ = progress.send(UpdateEvent::DownloadStarted { total_bytes: total });
            }
            downloaded += chunk as u64;
            let _ = progress.send(UpdateEvent::Progress {
                downloaded_bytes: downloaded,
                total_bytes: total,
            });
        },
        || {},
    );
    let install = |bytes: &[u8]| {
        let _ = installing.send(UpdateEvent::Installing);
        update.install(bytes)
    };
    let result = fetch_and_install(
        &backend,
        &pending.package,
        package_key(&update),
        async { download.await.map_err(Failure::Updater) },
        |bytes| install(bytes).map_err(Failure::Updater),
    )
    .await;
    // On Windows the installer has already exited the process by now. Elsewhere the gate stays
    // closed until the restart.
    let _gate = match result {
        Ok(gate) => gate,
        Err(Failure::Refused(err)) => return Err(err),
        Err(Failure::Updater(err)) => {
            tracing::warn!(target: "pagelamp::updates", code = error_code(&err), "update install failed: {err}");
            return Err(app_error(&err));
        }
    };
    tracing::info!(target: "pagelamp::updates", version = %update.version, "update installed; restarting");
    let _ = on_event.send(UpdateEvent::Restarting);
    app.restart()
}

/// Why `fetch_and_install` stopped.
enum Failure<E> {
    /// Something runs that the restart would kill, or another install is under way.
    Refused(AppError),
    /// The download, its signature check or the install failed.
    Updater(E),
}

/// "Install and restart" apart from the updater (tests pass fakes): hold new work back, check
/// nothing runs, download unless the package was kept, check again, install. Work can start
/// during the download: another process's sync, or one of ours that got past the gate as it
/// closed. Then the package is kept and the install refused; trying again installs it. The gate
/// is returned so it stays closed until the restart; every error drops it.
async fn fetch_and_install<'a, E>(
    backend: &'a Backend,
    kept: &Mutex<Option<Package>>,
    key: String,
    download: impl Future<Output = Result<Vec<u8>, Failure<E>>>,
    install: impl FnOnce(&[u8]) -> Result<(), Failure<E>>,
) -> Result<InstallGate<'a>, Failure<E>> {
    let gate = backend.hold_work_for_install().ok_or_else(|| {
        Failure::Refused(AppError::new(
            AppErrorKind::Busy,
            "The update is already being installed.",
        ))
    })?;
    refuse_while_busy(backend).map_err(Failure::Refused)?;
    let kept_bytes = lock(kept)
        .take_if(|package| package.key == key)
        .map(|package| package.bytes);
    let bytes = match kept_bytes {
        Some(bytes) => bytes,
        None => download.await?,
    };
    if let Err(busy) = refuse_while_busy(backend) {
        *lock(kept) = Some(Package { key, bytes });
        return Err(Failure::Refused(busy));
    }
    install(&bytes)?;
    Ok(gate)
}

/// Busy while this app runs anything (a sync, a download, a Codex install, a model run) or
/// another process syncs. Without an open core (e.g. its database is from a newer PageLamp)
/// nothing of ours can run, and installing the update is the way out.
fn refuse_while_busy(backend: &Backend) -> CmdResult<()> {
    // Work that got past `spawn_work`'s gate check before it closed but hasn't registered in
    // activity() yet (a retry with a kept package has no download to wait it out).
    if backend.work_in_flight() > 0 {
        return Err(AppError::new(
            AppErrorKind::Busy,
            "A sync, a download or an AI reading is starting. Install the update when it finishes.",
        ));
    }
    let Ok(facade) = backend.app() else {
        return Ok(());
    };
    match busy_message(&facade.activity()) {
        None => Ok(()),
        Some(message) => Err(AppError::new(AppErrorKind::Busy, message)),
    }
}

/// What holds an install back, a sync first; `None` when nothing does.
fn busy_message(activity: &Activity) -> Option<&'static str> {
    let running = |kind| activity.items.iter().any(|item| item.kind == kind);
    if activity.other_process_syncing
        || running(ActivityKind::Sync)
        || running(ActivityKind::Download)
    {
        Some("A sync is running. Install the update when it finishes.")
    } else if running(ActivityKind::Generation) {
        Some(
            "The AI is still working (reading a syllabus or writing a study plan). \
             Install the update when it finishes.",
        )
    } else if running(ActivityKind::CodexInstall) {
        Some("Codex is still downloading. Install the update when it finishes.")
    } else {
        None
    }
}

/// Windows: the NSIS pre-install hook (windows/hooks.nsh) renames a running `pagelamp.exe` (an
/// AI app's MCP server keeps it locked) to `pagelamp.exe.old` (or `.old2`). Once nothing runs it
/// any more, delete it.
pub fn remove_old_sidecar() {
    let binary = crate::backend::pagelamp_binary();
    let Some(name) = binary.file_name() else {
        return;
    };
    for suffix in [".old", ".old2"] {
        let mut old = name.to_os_string();
        old.push(suffix);
        let old = binary.with_file_name(old);
        if old.exists()
            && let Err(err) = std::fs::remove_file(&old)
        {
            // Still in use (an AI app hasn't been restarted yet): try again next launch.
            tracing::info!(target: "pagelamp::updates", "leftover sidecar not removed yet: {err}");
        }
    }
}

/// The updater plugin, comparing versions with the real crate version.
pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R, tauri_plugin_updater::Config> {
    tauri_plugin_updater::Builder::new()
        .default_version_comparator(|_, remote| is_newer(&current_version(), &remote.version))
        .build()
}

/// Registers what the updater commands need.
pub fn manage<R: Runtime>(app: &AppHandle<R>) {
    app.manage(PendingUpdate::default());
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::{current_version, is_newer};

    fn v(s: &str) -> Version {
        Version::parse(s).expect("test version")
    }

    #[test]
    fn a_beta_is_offered_its_final_release() {
        assert!(is_newer(&v("0.3.0-beta.2"), &v("0.3.0")));
        assert!(is_newer(&v("0.3.0-alpha.1"), &v("0.3.0-alpha.2")));
        assert!(is_newer(&v("0.3.0-alpha.9"), &v("0.3.0-beta.1")));
        assert!(is_newer(&v("0.3.0"), &v("0.3.1")));
    }

    #[test]
    fn stable_without_a_release_is_not_a_failed_check() {
        use pagelamp_app::{AppErrorKind, UpdateChannel};
        use tauri_plugin_updater::Error;

        use super::check_error;
        let test_version = v("0.3.0-alpha.1");
        assert_eq!(
            check_error(
                &Error::ReleaseNotFound,
                UpdateChannel::Stable,
                &test_version
            )
            .kind,
            AppErrorKind::NotFound
        );
        // Beta always has a release (this build came from it): a missing one is a real failure.
        assert_eq!(
            check_error(&Error::ReleaseNotFound, UpdateChannel::Beta, &test_version).kind,
            AppErrorKind::Internal
        );
        assert_eq!(
            check_error(&Error::EmptyEndpoints, UpdateChannel::Stable, &test_version).kind,
            AppErrorKind::Internal
        );
        // So does Stable for a stable build: it came from a stable release.
        assert_eq!(
            check_error(&Error::ReleaseNotFound, UpdateChannel::Stable, &v("0.3.0")).kind,
            AppErrorKind::Internal
        );
    }

    #[test]
    fn without_the_facade_a_pre_release_checks_beta() {
        use super::default_channel;
        use pagelamp_app::UpdateChannel;
        assert!(matches!(
            default_channel(&v("0.3.0-alpha.1")),
            UpdateChannel::Beta
        ));
        assert!(matches!(
            default_channel(&v("0.3.0")),
            UpdateChannel::Stable
        ));
    }

    #[test]
    fn the_same_or_an_older_version_is_not_an_update() {
        assert!(!is_newer(&v("0.3.0"), &v("0.3.0")));
        assert!(!is_newer(&v("0.3.0"), &v("0.3.0-beta.2")));
        assert!(!is_newer(&v("0.3.1"), &v("0.3.0")));
    }

    #[test]
    fn the_shipped_update_settings_are_valid_static_https_urls() {
        use super::ChannelsConfig;
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        let plugins = &config["plugins"];
        // What the plugin itself parses at startup (a bad section would stop the app).
        let _: tauri_plugin_updater::Config =
            serde_json::from_value(plugins["updater"].clone()).expect("plugins.updater");
        let channels: ChannelsConfig =
            serde_json::from_value(plugins["pagelamp-updates"].clone()).expect("channels");
        let urls: Vec<_> = channels.stable.iter().chain(&channels.beta).collect();
        assert!(!channels.stable.is_empty() && !channels.beta.is_empty());
        for url in &urls {
            assert_eq!(url.scheme(), "https", "{url}");
            // Static: no {{target}}/{{arch}}/{{current_version}} sent to the server.
            assert!(
                !url.as_str().contains("%7B%7B") && !url.as_str().contains("{{"),
                "{url}"
            );
        }
        assert!(channels.release_page.contains("{version}"));
        // The first endpoint of each channel is the gh-pages branch on raw.githubusercontent.com,
        // which depends on no domain of ours (see the module docs).
        let raw = "https://raw.githubusercontent.com/Euswbnix/pagelamp/gh-pages/updates/";
        for list in [&channels.stable, &channels.beta] {
            assert!(list[0].as_str().starts_with(raw), "{list:?}");
        }

        // The rehearsal overlay only swaps the endpoints, and only to the test manifest.
        let overlay: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.rehearsal.conf.json")).expect("overlay");
        let test = &overlay["plugins"]["pagelamp-updates"];
        for channel in ["stable", "beta"] {
            let list: Vec<tauri::Url> =
                serde_json::from_value(test[channel].clone()).expect("urls");
            assert!(
                list.iter()
                    .all(|u| u.as_str().ends_with("/updates/test.json")),
                "{list:?}"
            );
            assert!(list[0].as_str().starts_with(raw), "{list:?}");
        }
    }

    #[test]
    fn the_running_version_is_the_crate_version() {
        assert_eq!(current_version().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn an_install_says_what_holds_it_back() {
        use pagelamp_app::{Activity, ActivityItem, ActivityKind};

        use super::busy_message;

        let item = |kind| ActivityItem {
            kind,
            source_id: None,
            generation_id: None,
            started_at: chrono::Utc::now(),
        };
        let activity = |kinds: &[ActivityKind], other_process_syncing| Activity {
            items: kinds.iter().copied().map(item).collect(),
            other_process_syncing,
        };
        assert_eq!(busy_message(&activity(&[], false)), None);
        let sync = Some("A sync is running. Install the update when it finishes.");
        assert_eq!(busy_message(&activity(&[], true)), sync);
        assert_eq!(
            busy_message(&activity(&[ActivityKind::Download], false)),
            sync
        );
        // A sync is named first: it can be stopped from the dialog.
        let both = activity(&[ActivityKind::Generation, ActivityKind::Sync], false);
        assert_eq!(busy_message(&both), sync);
        assert!(
            busy_message(&activity(&[ActivityKind::Generation], false))
                .is_some_and(|m| m.starts_with("The AI is still working"))
        );
        assert!(
            busy_message(&activity(&[ActivityKind::CodexInstall], false))
                .is_some_and(|m| m.starts_with("Codex is still downloading"))
        );
    }

    mod install {
        use std::cell::Cell;
        use std::fs::File;
        use std::path::Path;
        use std::sync::Mutex;

        use pagelamp_app::{App, AppErrorKind};
        use pagelamp_core::paths;
        use tauri::async_runtime::block_on;

        use crate::backend::Backend;
        use crate::updates::{Failure, Package, fetch_and_install};

        const KEY: &str = "0.3.1 https://example.invalid/PageLamp.app.tar.gz";

        fn backend() -> (tempfile::TempDir, Backend) {
            let dir = tempfile::tempdir().expect("temp dir");
            let app = App::open_at(dir.path().to_path_buf()).expect("open App in a temp dir");
            (dir, Backend::from_app(app))
        }

        /// Another process (the CLI) syncing: it holds `sync.lock` until the file drops.
        fn other_process_syncs(data_dir: &Path) -> File {
            let file = File::options()
                .create(true)
                .truncate(false)
                .write(true)
                .open(paths::sync_lock_path_in(data_dir))
                .expect("sync.lock");
            file.lock().expect("lock sync.lock");
            file
        }

        fn kept(key: &str, bytes: &[u8]) -> Mutex<Option<Package>> {
            Mutex::new(Some(Package {
                key: key.to_string(),
                bytes: bytes.to_vec(),
            }))
        }

        fn refused<T>(result: Result<T, Failure<&'static str>>) -> AppErrorKind {
            match result {
                Err(Failure::Refused(err)) => err.kind,
                Err(Failure::Updater(err)) => panic!("refused expected, updater failed: {err}"),
                Ok(_) => panic!("refused expected, installed"),
            }
        }

        #[test]
        fn work_not_yet_in_activity_holds_a_kept_package_back() {
            use std::future::Future;
            use std::task::{Context, Waker};

            let (_dir, backend) = backend();
            let package = kept(KEY, b"package");
            // A sync got past spawn_work's gate check before the install closed it, and hasn't
            // registered in activity() yet (this one never does: it waits to be let go).
            let (release, wait) = std::sync::mpsc::channel::<()>();
            let mut sync = Box::pin(backend.spawn_work(move |_| async move {
                let _ = wait.recv();
                Ok(())
            }));
            let mut cx = Context::from_waker(Waker::noop());
            assert!(sync.as_mut().poll(&mut cx).is_pending());

            let installed = Cell::new(false);
            let result = block_on(fetch_and_install::<&'static str>(
                &backend,
                &package,
                KEY.to_string(),
                async { panic!("the kept package needs no download") },
                |_| {
                    installed.set(true);
                    Ok(())
                },
            ));
            assert!(matches!(refused(result), AppErrorKind::Busy));
            assert!(!installed.get(), "nothing installed while the sync starts");
            assert!(
                package.lock().unwrap().is_some(),
                "the package is still kept"
            );

            release.send(()).expect("let the sync go");
            block_on(sync).expect("the sync ends");
            let gate = block_on(fetch_and_install::<&'static str>(
                &backend,
                &package,
                KEY.to_string(),
                async { panic!("the kept package needs no download") },
                |_| {
                    installed.set(true);
                    Ok(())
                },
            ));
            assert!(
                gate.is_ok() && installed.get(),
                "then the retry installs it"
            );
        }

        #[test]
        fn new_work_is_refused_while_an_update_installs() {
            let (_dir, backend) = backend();
            let gate = backend.hold_work_for_install().expect("gate");
            assert!(
                backend.hold_work_for_install().is_none(),
                "one install at a time"
            );
            let work = block_on(backend.spawn_work(|_| async { Ok(()) }));
            assert!(matches!(work.unwrap_err().kind, AppErrorKind::Busy));
            assert_eq!(backend.work_in_flight(), 0, "refused work isn't counted");
            drop(gate);
            block_on(backend.spawn_work(|_| async { Ok(()) })).expect("work runs again");
        }

        #[test]
        fn a_sync_that_starts_during_the_download_holds_the_install_back() {
            let (dir, backend) = backend();
            let package = Mutex::new(None);
            let lock = Cell::new(None);
            let installed = Cell::new(false);
            let result = block_on(fetch_and_install(
                &backend,
                &package,
                KEY.to_string(),
                async {
                    lock.set(Some(other_process_syncs(dir.path())));
                    Ok(b"package".to_vec())
                },
                |_| {
                    installed.set(true);
                    Ok(())
                },
            ));
            assert!(matches!(refused(result), AppErrorKind::Busy));
            assert!(!installed.get(), "nothing installed while a sync runs");
            let kept = package.lock().unwrap();
            let kept = kept.as_ref().expect("the package is kept");
            assert_eq!(
                (kept.key.as_str(), kept.bytes.as_slice()),
                (KEY, &b"package"[..])
            );
            assert!(
                backend.hold_work_for_install().is_some(),
                "the gate is open again"
            );
        }

        #[test]
        fn trying_again_installs_the_kept_package_without_downloading() {
            let (_dir, backend) = backend();
            let package = kept(KEY, b"package");
            let installed = Cell::new(None);
            let gate = block_on(fetch_and_install::<&'static str>(
                &backend,
                &package,
                KEY.to_string(),
                async { panic!("the kept package must not be downloaded again") },
                |bytes| {
                    installed.set(Some(bytes.to_vec()));
                    Ok(())
                },
            ));
            let gate = gate.unwrap_or_else(|_| panic!("installed"));
            assert_eq!(installed.take().as_deref(), Some(&b"package"[..]));
            assert!(package.lock().unwrap().is_none());
            assert!(
                backend.hold_work_for_install().is_none(),
                "the gate stays closed until the restart"
            );
            drop(gate);
        }

        #[test]
        fn a_package_for_another_update_is_not_installed() {
            let (_dir, backend) = backend();
            let package = kept("0.3.0 https://example.invalid/old.tar.gz", b"old");
            let installed = Cell::new(None);
            let gate = block_on(fetch_and_install::<&'static str>(
                &backend,
                &package,
                KEY.to_string(),
                async { Ok(b"new".to_vec()) },
                |bytes| {
                    installed.set(Some(bytes.to_vec()));
                    Ok(())
                },
            ));
            assert!(gate.is_ok());
            assert_eq!(installed.take().as_deref(), Some(&b"new"[..]));
        }

        #[test]
        fn nothing_is_downloaded_while_a_sync_runs() {
            let (dir, backend) = backend();
            let _sync = other_process_syncs(dir.path());
            let result = block_on(fetch_and_install::<&'static str>(
                &backend,
                &Mutex::new(None),
                KEY.to_string(),
                async { panic!("no download while a sync runs") },
                |_| panic!("no install while a sync runs"),
            ));
            assert!(matches!(refused(result), AppErrorKind::Busy));
            assert!(
                backend.hold_work_for_install().is_some(),
                "the gate is open again"
            );
        }

        #[test]
        fn the_gate_opens_again_after_a_failed_download_or_install() {
            let (_dir, backend) = backend();
            let package = Mutex::new(None);
            let result = block_on(fetch_and_install(
                &backend,
                &package,
                KEY.to_string(),
                async { Err(Failure::Updater("network")) },
                |_| panic!("nothing to install"),
            ));
            assert!(matches!(result, Err(Failure::Updater("network"))));
            assert!(
                backend.hold_work_for_install().is_some(),
                "open after a failed download"
            );

            let result = block_on(fetch_and_install(
                &backend,
                &package,
                KEY.to_string(),
                async { Ok(b"package".to_vec()) },
                |_| Err(Failure::Updater("install")),
            ));
            assert!(matches!(result, Err(Failure::Updater("install"))));
            assert!(
                backend.hold_work_for_install().is_some(),
                "open after a failed install"
            );
            assert!(
                package.lock().unwrap().is_none(),
                "a package that failed to install is downloaded again"
            );
        }

        #[test]
        fn a_cancelled_install_opens_the_gate() {
            use std::future::Future;
            use std::task::{Context, Waker};

            let (_dir, backend) = backend();
            let package = Mutex::new(None);
            // The command's future is dropped mid-download (e.g. the app quits).
            let mut install = Box::pin(fetch_and_install::<&'static str>(
                &backend,
                &package,
                KEY.to_string(),
                std::future::pending(),
                |_| panic!("nothing to install"),
            ));
            let mut cx = Context::from_waker(Waker::noop());
            assert!(
                install.as_mut().poll(&mut cx).is_pending(),
                "still downloading"
            );
            assert!(
                backend.hold_work_for_install().is_none(),
                "held while downloading"
            );
            drop(install);
            assert!(backend.hold_work_for_install().is_some());
        }
    }
}
