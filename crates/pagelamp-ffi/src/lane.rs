//! The course lane beyond weeks and phases (v0.3): course calendars and reading the syllabus,
//! removing finished courses, reminders and the weekly digest, and a material's local file.

use std::sync::Arc;

use pagelamp_app::{
    CalendarRunOutcome, CourseCalendarView, CourseDatesInput, LocalFileUse, PurgeReport,
    ReadCalendarOptions, Reminder, ReminderSettings, RemovalPreview, RemovalReport, RemoveOptions,
    RemovedCourse, RestoreOutcome, SourceSyncResult, SyllabusOffer,
};
use pagelamp_core::calendar::candidates::CalendarCandidate;
use pagelamp_core::calendar::proposal::CalendarProposal;
use pagelamp_core::views::WeeklyDigest;

use crate::observers::{CalendarBatchObserver, GenObserver, batch_events, gen_events};
use crate::{PageLamp, Result, SyncObserver, Timestamp, blocking, forward_to, spawned};

#[uniffi::export]
impl PageLamp {
    // ----- course calendars ------------------------------------------------------------------------

    /// The calendar in force, the proposals waiting, the candidates to read and why reading
    /// is blocked, if it is.
    pub async fn course_calendar(&self, course: String) -> Result<CourseCalendarView> {
        let app = self.app.clone();
        blocking(move || app.course_calendar(&course)).await
    }

    /// The materials a syllabus reading would read, and why others are left out.
    pub async fn calendar_candidates(&self, course: String) -> Result<Vec<CalendarCandidate>> {
        let app = self.app.clone();
        blocking(move || app.calendar_candidates(&course)).await
    }

    /// The student's picks: always read `include`, never `exclude` (material ids).
    pub async fn set_calendar_sources(
        &self,
        course: String,
        include: Vec<String>,
        exclude: Vec<String>,
    ) -> Result<Vec<CalendarCandidate>> {
        let app = self.app.clone();
        blocking(move || app.set_calendar_sources(&course, include, exclude)).await
    }

    /// Downloads these materials' files (Canvas) so they can be read; UIs disclose first.
    pub async fn download_material_files(
        &self,
        course: String,
        material_ids: Vec<String>,
        observer: Arc<dyn SyncObserver>,
    ) -> Result<SourceSyncResult> {
        let app = self.app.clone();
        spawned(async move {
            app.download_material_files(&course, material_ids, forward_to(observer))
                .await
        })
        .await
    }

    /// The deterministic scan of the syllabus (no model); nil when it finds nothing new.
    pub async fn scan_course_calendar(&self, course: String) -> Result<Option<CalendarProposal>> {
        let app = self.app.clone();
        blocking(move || app.scan_course_calendar(&course)).await
    }

    /// The student's own dates (nil: clear them, "Undo").
    pub async fn set_course_dates(
        &self,
        course: String,
        dates: Option<CourseDatesInput>,
    ) -> Result<CourseCalendarView> {
        let app = self.app.clone();
        blocking(move || app.set_course_dates(&course, dates)).await
    }

    /// Accepts a proposal, optionally with the student's edits.
    pub async fn accept_calendar_proposal(
        &self,
        proposal_id: i64,
        edits: Option<CourseDatesInput>,
    ) -> Result<CourseCalendarView> {
        let app = self.app.clone();
        blocking(move || app.accept_calendar_proposal(proposal_id, edits)).await
    }

    /// Accepts proposals that have no conflicts ("Accept all that pass"); none when one
    /// doesn't pass (`Invalid`).
    pub async fn accept_passing_proposals(
        &self,
        proposal_ids: Vec<i64>,
    ) -> Result<Vec<CourseCalendarView>> {
        let app = self.app.clone();
        blocking(move || app.accept_passing_proposals(proposal_ids)).await
    }

    pub async fn dismiss_calendar_proposal(&self, proposal_id: i64) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.dismiss_calendar_proposal(proposal_id)).await
    }

    /// The courses "Read syllabi for N courses" offers; empty while "Not now" covers them.
    pub async fn syllabus_reading_offers(&self) -> Result<Vec<SyllabusOffer>> {
        let app = self.app.clone();
        blocking(move || app.syllabus_reading_offers()).await
    }

    /// "Not now" on the syllabus reading offers (`not_now_days()`).
    pub async fn snooze_calendar_offers(&self) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.snooze_calendar_offers()).await
    }

    /// Reads the syllabus with the chosen model into a proposal that changes nothing until
    /// accepted; `cancel_generation(generation_id)` stops it.
    pub async fn read_course_calendar(
        &self,
        course: String,
        generation_id: String,
        options: ReadCalendarOptions,
        observer: Arc<dyn GenObserver>,
    ) -> Result<CalendarProposal> {
        let app = self.app.clone();
        let events = gen_events(observer);
        let sink = events.sink();
        let result = spawned(async move {
            app.read_course_calendar(&course, &generation_id, options, sink)
                .await
        })
        .await;
        events.drain().await;
        result
    }

    /// Reads several courses' syllabi, one after another; `cancel_generation(batch_id)`
    /// stops the rest. A course that fails is an outcome, not an error.
    pub async fn read_course_calendars(
        &self,
        courses: Vec<String>,
        batch_id: String,
        options: ReadCalendarOptions,
        observer: Arc<dyn CalendarBatchObserver>,
    ) -> Result<Vec<CalendarRunOutcome>> {
        let app = self.app.clone();
        let events = batch_events(observer);
        let sink = events.sink();
        let result = spawned(async move {
            app.read_course_calendars(courses, &batch_id, options, sink)
                .await
        })
        .await;
        events.drain().await;
        result
    }

    // ----- removing finished courses --------------------------------------------------------------

    /// What removing `courses` would take away and keep.
    pub async fn removal_preview(&self, courses: Vec<String>) -> Result<RemovalPreview> {
        let app = self.app.clone();
        blocking(move || app.removal_preview(courses)).await
    }

    /// Removes the courses (stage 1: undoable for 7 days).
    pub async fn remove_courses(
        &self,
        courses: Vec<String>,
        options: RemoveOptions,
    ) -> Result<RemovalReport> {
        let app = self.app.clone();
        spawned(async move { app.remove_courses(courses, options).await }).await
    }

    pub async fn removed_courses(&self) -> Result<Vec<RemovedCourse>> {
        let app = self.app.clone();
        blocking(move || app.removed_courses()).await
    }

    pub async fn restore_course(&self, removed_id: String) -> Result<RestoreOutcome> {
        let app = self.app.clone();
        spawned(async move { app.restore_course(&removed_id).await }).await
    }

    /// Purges removed courses whose time is up (`removed_ids`: these, nil: every due one).
    /// `permanent_if_no_trash`: delete files for good where there is no Trash; callers ask
    /// first, it is never implied.
    pub async fn purge_removed_courses(
        &self,
        removed_ids: Option<Vec<String>>,
        permanent_if_no_trash: bool,
    ) -> Result<PurgeReport> {
        let app = self.app.clone();
        spawned(async move {
            app.purge_removed_courses(removed_ids, permanent_if_no_trash)
                .await
        })
        .await
    }

    /// Stops listing a removed course (its undo is gone).
    pub async fn forget_removed_course(&self, removed_id: String) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.forget_removed_course(&removed_id)).await
    }

    // ----- reminders and the weekly digest -----------------------------------------------------------

    pub async fn weekly_digest(&self) -> Result<WeeklyDigest> {
        let app = self.app.clone();
        blocking(move || app.weekly_digest()).await
    }

    pub async fn reminder_settings(&self) -> Result<ReminderSettings> {
        let app = self.app.clone();
        blocking(move || app.reminder_settings()).await
    }

    pub async fn set_reminder_settings(&self, settings: ReminderSettings) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.set_reminder_settings(&settings)).await
    }

    /// Every reminder from `from` to `to` (at most 62 days), for a schedule.
    pub async fn reminders(&self, from: Timestamp, to: Timestamp) -> Result<Vec<Reminder>> {
        let app = self.app.clone();
        blocking(move || app.reminders(from, to)).await
    }

    /// The reminders to show now (missed ones from the last 3 days included), not shown yet.
    pub async fn due_reminders(&self, now: Timestamp) -> Result<Vec<Reminder>> {
        let app = self.app.clone();
        blocking(move || app.due_reminders(now)).await
    }

    pub async fn mark_reminders_shown(&self, ids: Vec<String>) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.mark_reminders_shown(&ids)).await
    }

    // ----- files and syncs ----------------------------------------------------------------------------

    /// The material's file on this computer for `purpose` (open or reveal), nil when there
    /// is none; never a path outside the data folder or the course folder.
    pub async fn material_local_file(
        &self,
        material_id: String,
        purpose: LocalFileUse,
    ) -> Result<Option<String>> {
        let app = self.app.clone();
        blocking(move || app.material_local_file(&material_id, purpose)).await
    }

    /// Stops the running sync or download after its current step.
    pub async fn cancel_sync(&self) -> Result<()> {
        self.app.cancel_sync();
        Ok(())
    }
}
