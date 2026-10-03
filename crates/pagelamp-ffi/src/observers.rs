//! Progress callbacks implemented in Swift, for the long-running facade calls: model runs
//! (`GenObserver`), batches of syllabus readings (`CalendarBatchObserver`) and the Codex
//! install and sign-in (`CodexInstallObserver`, `CodexLoginObserver`). `SyncObserver` (lib.rs)
//! is the older, direct pattern for syncs.
//!
//! A run never waits for its observer: events go through an unbounded channel to a forwarder
//! on the runtime's blocking pool, which calls the observer in order. An observer that is slow,
//! never returns or panics can't hold up or break the run; the call returns at most
//! `DRAIN_WAIT` after the run ends even if events are still undelivered. An observer that never
//! returns keeps its forwarder (a blocking-pool thread) for good, though. Stopping a run is the
//! caller's own id: `cancel_generation(id)` (or `cancel_codex_install` / `cancel_codex_login`).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Duration;

use pagelamp_app::CalendarBatchEvent;
use pagelamp_app::ai::{GenEvent, LoginEvent, RuntimeEvent};
use tokio::sync::mpsc;

use crate::RUNTIME;

/// How long a finished call waits for its observer to take the remaining events.
pub(crate) const DRAIN_WAIT: Duration = Duration::from_secs(2);

/// Receives the progress of `explain_week`, `generate_study_plan` and `read_course_calendar`.
#[uniffi::export(foreign)]
pub trait GenObserver: Send + Sync {
    /// Called in order on a PageLamp worker thread. The run doesn't wait for it.
    fn on_event(&self, event: GenEvent);
}

/// Receives the progress of `read_course_calendars` (one event per course and stage).
#[uniffi::export(foreign)]
pub trait CalendarBatchObserver: Send + Sync {
    fn on_event(&self, event: CalendarBatchEvent);
}

/// Receives the progress of `install_codex` (download, verify, install).
#[uniffi::export(foreign)]
pub trait CodexInstallObserver: Send + Sync {
    fn on_event(&self, event: RuntimeEvent);
}

/// Receives the progress of `codex_login` (the browser URL or the one-time code, then done).
#[uniffi::export(foreign)]
pub trait CodexLoginObserver: Send + Sync {
    fn on_event(&self, event: LoginEvent);
}

/// The sending half a run calls, and the forwarder that delivers to the observer.
pub(crate) struct Forward<E> {
    sender: mpsc::UnboundedSender<E>,
    forwarder: tokio::task::JoinHandle<()>,
}

impl<E: Send + 'static> Forward<E> {
    /// Start delivering to `deliver` (the observer's `on_event`), in order, off the run.
    pub(crate) fn to(deliver: impl Fn(E) + Send + 'static) -> Self {
        let (sender, mut receiver) = mpsc::unbounded_channel::<E>();
        let forwarder = RUNTIME.spawn_blocking(move || {
            while let Some(event) = receiver.blocking_recv() {
                // A panicking observer loses its events, never the run.
                if catch_unwind(AssertUnwindSafe(|| deliver(event))).is_err() {
                    break;
                }
            }
        });
        Forward { sender, forwarder }
    }

    /// What the facade call gets as `on_event`: never blocks, never fails.
    pub(crate) fn sink(&self) -> impl Fn(E) + Send + Sync + 'static {
        let sender = self.sender.clone();
        move |event| {
            let _ = sender.send(event);
        }
    }

    /// After the run: let the observer take what's left, for at most `DRAIN_WAIT`. The wait
    /// runs on PageLamp's runtime, like every call: the caller's executor needs no timer.
    pub(crate) async fn drain(self) {
        drop(self.sender);
        let forwarder = self.forwarder;
        let _ = RUNTIME
            .spawn(async move {
                let _ = tokio::time::timeout(DRAIN_WAIT, forwarder).await;
            })
            .await;
    }
}

pub(crate) fn gen_events(observer: Arc<dyn GenObserver>) -> Forward<GenEvent> {
    Forward::to(move |event| observer.on_event(event))
}

pub(crate) fn batch_events(
    observer: Arc<dyn CalendarBatchObserver>,
) -> Forward<CalendarBatchEvent> {
    Forward::to(move |event| observer.on_event(event))
}

pub(crate) fn install_events(observer: Arc<dyn CodexInstallObserver>) -> Forward<RuntimeEvent> {
    Forward::to(move |event| observer.on_event(event))
}

pub(crate) fn login_events(observer: Arc<dyn CodexLoginObserver>) -> Forward<LoginEvent> {
    Forward::to(move |event| observer.on_event(event))
}
