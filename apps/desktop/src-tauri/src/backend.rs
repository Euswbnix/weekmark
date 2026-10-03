//! Owns the one `pagelamp_app::App` instance and runs facade calls off the main thread.

use std::future::Future;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use pagelamp_app::diagnostics::{expect_panics, expect_panics_in};
use pagelamp_app::{App, AppError, AppErrorKind};
use pagelamp_core::brand;

/// Managed Tauri state. If the core fails to open (e.g. the data folder isn't writable yet),
/// the window still starts and every command returns that error, so the UI can explain it. A
/// failed open is retried on the next call, so the start screen's "Try again" can recover
/// without restarting the app; once opened, the facade is kept.
pub struct Backend {
    state: Mutex<Result<App, AppError>>,
    open: fn() -> Result<App, AppError>,
    /// Set while an update downloads and installs (`InstallGate`): the restart, or on Windows
    /// the installer closing the app, would kill any work started meanwhile.
    installing: AtomicBool,
    /// Work started through `spawn_work` that hasn't ended. Counted before the gate is checked,
    /// while an install closes the gate before it checks the count: one of the two always sees
    /// the other, even before the work registers in `App::activity`.
    in_flight: Arc<AtomicUsize>,
}

impl Backend {
    pub fn open() -> Self {
        Backend::open_with(open_app)
    }

    fn open_with(open: fn() -> Result<App, AppError>) -> Self {
        Backend {
            state: Mutex::new(open_guarded(open)),
            open,
            installing: AtomicBool::new(false),
            in_flight: Arc::default(),
        }
    }

    /// Wrap an already opened facade (tests use a temp data dir and in-memory secrets).
    pub fn from_app(app: App) -> Self {
        Backend {
            state: Mutex::new(Ok(app)),
            open: App::open,
            installing: AtomicBool::new(false),
            in_flight: Arc::default(),
        }
    }

    /// A handle to the facade (cheap: `App` only holds the data-dir path). Retries a failed open.
    pub fn app(&self) -> Result<App, AppError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.is_err() {
            *state = open_guarded(self.open);
        }
        state.clone()
    }

    /// Run a synchronous facade call (they open SQLite) on the blocking pool, never on the
    /// UI thread. Panics become `internal` errors (logged, but not recorded as a crash: the app
    /// keeps running).
    pub async fn blocking<T, F>(&self, f: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&App) -> Result<T, AppError> + Send + 'static,
    {
        let app = self.app()?;
        tauri::async_runtime::spawn_blocking(move || expect_panics(|| f(&app)))
            .await
            .map_err(|err| internal(format!("background task failed: {err}")))?
    }

    /// Diagnostics must work even when the facade can't open: a locked or damaged database is
    /// exactly when a tester needs a report. With an open facade they use its data dir,
    /// otherwise the default one (`pagelamp_app::diagnostics`, resolved like `App::open`).
    pub async fn diagnostics<T, F, G>(&self, with_app: F, without_app: G) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&App) -> Result<T, AppError> + Send + 'static,
        G: FnOnce() -> Result<T, AppError> + Send + 'static,
    {
        let app = self.app().ok();
        tauri::async_runtime::spawn_blocking(move || {
            expect_panics(|| match &app {
                Some(app) => with_app(app),
                None => without_app(),
            })
        })
        .await
        .map_err(|err| internal(format!("background task failed: {err}")))?
    }

    /// Run an async facade call (network: token validation, sync) as its own task. Panics
    /// become `internal` errors instead of a promise that never settles.
    pub async fn spawn<T, F, Fut>(&self, f: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(App) -> Fut,
        Fut: Future<Output = Result<T, AppError>> + Send + 'static,
    {
        let fut = f(self.app()?);
        tauri::async_runtime::spawn(expect_panics_in(AssertUnwindSafe(fut)))
            .await
            .map_err(|err| internal(format!("background task failed: {err}")))?
    }

    /// `spawn` for work that `App::activity` lists (a sync, a download, a model run, and a
    /// course removal, restore or purge, which hold `sync.lock`): refused while an update
    /// installs.
    pub async fn spawn_work<T, F, Fut>(&self, f: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(App) -> Fut,
        Fut: Future<Output = Result<T, AppError>> + Send + 'static,
    {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        // Dropped with the spawned task, not with this future: the work runs on regardless.
        let work = InFlight(Arc::clone(&self.in_flight));
        if self.installing.load(Ordering::SeqCst) {
            return Err(AppError::new(
                AppErrorKind::Busy,
                format!(
                    "{} is installing an update and restarts when it finishes.",
                    brand::PRODUCT_NAME
                ),
            ));
        }
        self.spawn(|app| {
            let fut = f(app);
            async move {
                let _work = work;
                fut.await
            }
        })
        .await
    }

    /// How much `spawn_work` work hasn't ended, including work not yet in `App::activity`.
    pub fn work_in_flight(&self) -> usize {
        self.in_flight.load(Ordering::SeqCst)
    }

    /// Hold new work back (`spawn_work`) until the gate drops, which every error or cancel path
    /// of an install does. `None` while another install holds it.
    pub fn hold_work_for_install(&self) -> Option<InstallGate<'_>> {
        self.installing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| InstallGate(&self.installing))
    }
}

/// An update is installing: new work is refused until this drops.
pub struct InstallGate<'a>(&'a AtomicBool);

/// One piece of `spawn_work` work; ending it (success, error, panic or drop) uncounts it.
struct InFlight(Arc<AtomicUsize>);

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Drop for InstallGate<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Opens the facade and points it at the bundled `pagelamp` sidecar, so syncs read files in
/// resource-limited `pagelamp extract-worker` processes (v0.3 M0.5). A retried open does the
/// same. If the sidecar can't run, the facade falls back as designed (small non-PDF files are
/// read directly, the rest wait) and `doctor` reports why.
fn open_app() -> Result<App, AppError> {
    let app = App::open()?;
    app.set_extract_worker(Some(pagelamp_binary()));
    Ok(app)
}

/// Open the facade; a panic inside the core must not kill the window, so it becomes an error.
fn open_guarded(open: fn() -> Result<App, AppError>) -> Result<App, AppError> {
    expect_panics(|| panic::catch_unwind(open)).unwrap_or_else(|payload| {
        Err(internal(format!(
            "{} core failed to start: {}",
            brand::PRODUCT_NAME,
            panic_message(&*payload)
        )))
    })
}

pub fn internal(message: impl Into<String>) -> AppError {
    AppError::new(AppErrorKind::Internal, message)
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".to_string())
}

/// The `pagelamp` binary that AI apps launch for MCP: the file next to this app's executable,
/// without a target-triple suffix. Release bundles ship it there as a Tauri sidecar
/// (`scripts/build-sidecar.mjs`, macOS: `PageLamp.app/Contents/MacOS/pagelamp`); in development
/// it is the workspace build in `target/debug` (`cargo build -p pagelamp-cli`).
pub fn pagelamp_binary() -> PathBuf {
    let name = format!("{}{}", brand::CLI_NAME, std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .map(|exe| exe.with_file_name(&name))
        .unwrap_or_else(|_| PathBuf::from(name))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use pagelamp_app::{App, AppError, AppErrorKind};

    use super::{Backend, brand, pagelamp_binary};

    #[test]
    fn a_failed_open_is_retried_and_success_is_kept() {
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        // A fresh directory per run: a fixed path in the shared temp dir could hold a database
        // left by another checkout (e.g. a newer schema), which made this test fail.
        static DIR: Mutex<Option<tempfile::TempDir>> = Mutex::new(None);
        fn flaky_open() -> Result<App, AppError> {
            match CALLS.fetch_add(1, Ordering::SeqCst) {
                0 => Err(AppError::new(
                    AppErrorKind::Internal,
                    "data folder not ready",
                )),
                1 => panic!("core crashed"),
                _ => {
                    let mut dir = DIR.lock().unwrap();
                    let dir = dir.get_or_insert_with(|| tempfile::tempdir().expect("temp dir"));
                    App::open_at(dir.path().to_path_buf())
                }
            }
        }
        let backend = Backend::open_with(flaky_open);
        assert_eq!(
            backend.app().unwrap_err().message,
            format!("{} core failed to start: core crashed", brand::PRODUCT_NAME)
        );
        assert!(backend.app().is_ok(), "third attempt opens");
        assert!(backend.app().is_ok());
        assert_eq!(
            CALLS.load(Ordering::SeqCst),
            3,
            "an opened facade is kept, not reopened"
        );
        drop(backend);
        DIR.lock().unwrap().take();
    }

    #[test]
    fn diagnostics_use_the_default_data_dir_only_when_the_core_cannot_open() {
        fn locked_open() -> Result<App, AppError> {
            Err(AppError::new(AppErrorKind::Internal, "database is locked"))
        }
        let backend = Backend::open_with(locked_open);
        let used = tauri::async_runtime::block_on(
            backend.diagnostics(|_| Ok("facade"), || Ok("default data dir")),
        );
        assert_eq!(used.unwrap(), "default data dir");

        // With an open facade, its own data dir: tests never touch the real default one.
        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::open_at(dir.path().to_path_buf()).expect("open App in a temp dir");
        let backend = Backend::from_app(app);
        let used = tauri::async_runtime::block_on(backend.diagnostics(
            |app| Ok(app.data_dir().to_path_buf()),
            || panic!("the open facade's data dir must be used"),
        ));
        assert_eq!(used.unwrap(), dir.path());
    }

    /// Waits (up to 5 s) for work the test let go of to end on the runtime.
    fn eventually(done: impl Fn() -> bool) {
        let started = std::time::Instant::now();
        while !done() {
            assert!(started.elapsed().as_secs() < 5, "timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn work_is_counted_until_it_ends_however_it_ends() {
        use std::future::Future;
        use std::task::{Context, Waker};

        use tauri::async_runtime::block_on;

        let dir = tempfile::tempdir().expect("temp dir");
        let app = App::open_at(dir.path().to_path_buf()).expect("open App in a temp dir");
        let backend = Backend::from_app(app);

        block_on(backend.spawn_work(|_| async { Ok(()) })).expect("success");
        assert_eq!(backend.work_in_flight(), 0, "after success");
        let failed: Result<(), AppError> =
            block_on(backend.spawn_work(|_| async {
                Err(AppError::new(AppErrorKind::Internal, "work failed"))
            }));
        assert!(failed.is_err());
        assert_eq!(backend.work_in_flight(), 0, "after an error");
        let crashed: Result<(), AppError> = block_on(backend.spawn_work(|_| async {
            let crash = true;
            if crash {
                panic!("work crashed");
            }
            Ok(())
        }));
        assert!(matches!(crashed.unwrap_err().kind, AppErrorKind::Internal));
        assert_eq!(backend.work_in_flight(), 0, "after a panic");

        // The command's future is dropped mid-work (e.g. the window reloads): the work runs on,
        // and stays counted until it ends.
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let mut command = Box::pin(backend.spawn_work(move |_| async move {
            let _ = wait.recv();
            Ok(())
        }));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(command.as_mut().poll(&mut cx).is_pending());
        drop(command);
        assert_eq!(backend.work_in_flight(), 1, "the work still runs");
        release.send(()).expect("release the work");
        eventually(|| backend.work_in_flight() == 0);

        // Refused before it starts: the facade can't open.
        fn locked_open() -> Result<App, AppError> {
            Err(AppError::new(AppErrorKind::Internal, "database is locked"))
        }
        let closed = Backend::open_with(locked_open);
        assert!(block_on(closed.spawn_work(|_| async { Ok(()) })).is_err());
        assert_eq!(closed.work_in_flight(), 0, "when the facade can't open");
    }

    #[test]
    fn the_mcp_binary_sits_next_to_the_app() {
        let exe = std::env::current_exe().expect("current exe");
        let bin = pagelamp_binary();
        assert_eq!(bin.parent(), exe.parent());
        let name = bin
            .file_name()
            .expect("file name")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            name,
            format!("{}{}", brand::CLI_NAME, std::env::consts::EXE_SUFFIX)
        );
    }
}
