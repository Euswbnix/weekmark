//! Canvas sync orchestration over any `CanvasTransport`.
//!
//! Per course: fetch structure (tabs, modules, files, pages, assignments, announcements),
//! then write metadata in ONE short transaction on a fresh `Store` inside `spawn_blocking`,
//! then index page/announcement/syllabus HTML and (only when asked) download + index files —
//! each outside any transaction. No `Store` is ever held across `.await`.
//!
//! Safety rules (a sync must never destroy what it merely failed to see):
//! - Materials are pruned per kind, and only when every listing that kind depends on was read
//!   completely (all pages, every item understood). A hidden tab, a 403, a failed fetch or a
//!   cut-off pagination keeps the existing materials of that kind. Announcements older than
//!   the synced window are kept.
//! - Courses are NEVER deleted by a Canvas sync: one that leaves the active list (term ended,
//!   enrollment concluded) keeps all its data and is marked `enrollment_active = false`.
//!   Removing old courses is the student's decision.
//! - Events are replaced only for what was re-read completely: a course's assignments, and
//!   planner items of the synced courses (plus personal notes).
//! - A file's new version date is recorded only after its new text was indexed, so an
//!   interrupted download is retried next time; previously indexed copies keep their text.
//! - Only an expired/revoked token, persistent throttling, a network failure on an API call
//!   or a local database error aborts the sync; a failed file download is a warning.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeDelta, Utc};
use pagelamp_core::ingest::{self, IndexOutcome};
use pagelamp_core::model::{
    CourseUpsert, DownloadBlock, Event, Material, MaterialKind, MaterialUpsert, Module, TextStatus,
};
use pagelamp_core::source::{CourseSyncSummary, ProgressFn, SourceError, SyncProgress, SyncStage};
use pagelamp_core::store::Store;

use crate::api::{Api, Listing};
use crate::endpoint::Endpoint;
use crate::json::{self, CanvasId};
use crate::map::{self, Ids, Placement};
use crate::transport::{CanvasError, CanvasTransport};
use crate::{SyncOptions, SyncReport};

/// How far back announcements are synced, and the planner window.
const ANNOUNCEMENT_DAYS: i64 = 120;
const PLANNER_DAYS_BACK: i64 = 7;
const PLANNER_DAYS_AHEAD: i64 = 120;
/// Longest local file name we create (bytes; file systems allow 255).
const MAX_FILE_NAME_BYTES: usize = 150;

/// A failure that stops the whole sync.
pub(crate) fn fatal(err: &CanvasError) -> Option<SourceError> {
    Some(match err {
        CanvasError::Unauthorized => SourceError::auth(
            "Canvas rejected the access token (it has expired or was revoked). Create a new token \
             in Canvas and update it.",
        ),
        CanvasError::RateLimited => SourceError::rate_limited(
            "Canvas kept throttling requests. Wait a few minutes and sync again.",
        ),
        CanvasError::Network(detail) => {
            SourceError::network(format!("Could not reach Canvas ({detail})."))
        }
        // The local database failing would repeat for every course.
        CanvasError::Store(detail) => {
            SourceError::other(format!("Could not save Canvas data ({detail})."))
        }
        CanvasError::Cancelled => SourceError::cancelled(),
        _ => return None,
    })
}

/// The first error of a call that must succeed (e.g. `/users/self`, the course list).
pub(crate) fn required(err: CanvasError, what: &str) -> SourceError {
    fatal(&err).unwrap_or_else(|| match err {
        CanvasError::NotFound => no_canvas_here(),
        other => SourceError::other(format!("Could not read {what} from Canvas: {other}.")),
    })
}

/// The Canvas address answered, but not with the Canvas API.
pub(crate) fn no_canvas_here() -> SourceError {
    SourceError::not_found("No Canvas API was found at this address — check the Canvas URL.")
}

/// Run a store job on the blocking pool with a fresh read-write `Store`.
async fn with_store<T: Send + 'static>(
    db: &Path,
    job: impl FnOnce(&Store) -> Result<T, pagelamp_core::Error> + Send + 'static,
) -> Result<T, CanvasError> {
    let db = db.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let store = Store::open(&db)?;
        job(&store)
    })
    .await
    .map_err(|err| CanvasError::Store(format!("sync task crashed: {err}")))?
    .map_err(|err| match err {
        pagelamp_core::Error::Cancelled => CanvasError::Cancelled,
        other => CanvasError::Store(other.to_string()),
    })
}

pub(crate) struct Syncer<'a, T> {
    pub api: &'a Api<T>,
    pub db: &'a Path,
    pub source_id: &'a str,
    pub options: &'a SyncOptions,
    pub progress: ProgressFn<'a>,
    pub now: DateTime<Utc>,
}

/// What one course contributed.
#[derive(Default)]
struct CourseResult {
    modules: usize,
    materials: usize,
    pages: usize,
    files: usize,
    files_downloaded: usize,
    /// Downloaded files plus pages/announcements whose text was (re)indexed.
    files_indexed: usize,
    /// Events from the assignments endpoint, if it was read completely.
    events: Option<Vec<Event>>,
}

/// Which material kinds this sync may prune (their listings were read completely).
#[derive(Clone, Copy)]
struct Complete {
    files: bool,
    pages: bool,
    links: bool,
    announcements: bool,
}

impl Complete {
    fn kind(&self, kind: MaterialKind) -> bool {
        match kind {
            MaterialKind::File => self.files,
            MaterialKind::Page => self.pages,
            MaterialKind::ExternalLink => self.links,
            MaterialKind::Announcement => self.announcements,
            MaterialKind::Syllabus => true,
        }
    }

    fn all(&self) -> bool {
        self.files && self.pages && self.links && self.announcements
    }
}

/// Page/announcement/syllabus HTML to index after the metadata write.
struct HtmlJob {
    material_id: String,
    html: String,
}

/// A file to download after the metadata write. `material` carries the NEW version date,
/// written only after the new text was indexed.
struct DownloadJob {
    material: MaterialUpsert,
    url: url::Url,
    dest: PathBuf,
    /// Whether an older indexed copy exists (then a failure keeps it).
    had_copy: bool,
}

/// A cached copy (same version) to read again because the text reader could not read it last
/// time for a reason that may have gone away (`ingest::needs_retry`). Not downloaded again:
/// every download counts as a view.
struct ReindexJob {
    material_id: String,
    title: String,
    path: PathBuf,
    mime: Option<String>,
}

impl<T: CanvasTransport> Syncer<'_, T> {
    fn ids(&self) -> Ids<'_> {
        Ids {
            source: self.source_id,
        }
    }

    /// Stop here if the student stopped the sync (between courses, requests and files).
    fn check_cancelled(&self) -> Result<(), CanvasError> {
        if self.options.extractor.is_cancelled() {
            Err(CanvasError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn warn(&self, report: &mut SyncReport, message: String) {
        (self.progress)(SyncProgress::Warning(message.clone()));
        report.warnings.push(message);
    }

    /// After indexing one file: warn about it if it could not be read (and, once per sync,
    /// when the text reader could not start). True when its text is indexed.
    fn record_index(
        &self,
        report: &mut SyncReport,
        label: &str,
        title: &str,
        outcome: IndexOutcome,
    ) -> bool {
        if let Some(message) = self.options.extractor.take_warning() {
            self.warn(report, message);
        }
        match outcome {
            IndexOutcome::Indexed { .. } | IndexOutcome::Unchanged | IndexOutcome::Empty => true,
            IndexOutcome::Failed(message) => {
                self.warn(report, format!("{label}: {title}: {message}"));
                false
            }
            IndexOutcome::Skipped(kind) => {
                let message = ingest::worker_failure_message(kind);
                self.warn(report, format!("{label}: {title}: {message}"));
                false
            }
            // Tried again next sync; the extractor's warning covers every such file.
            IndexOutcome::Unsupported | IndexOutcome::Deferred(_) => false,
        }
    }

    fn step(
        &self,
        stage: SyncStage,
        course: Option<&str>,
        message: String,
        current: Option<usize>,
        total: Option<usize>,
    ) {
        (self.progress)(SyncProgress::Step {
            message,
            current: current.map(to_u32),
            total: total.map(to_u32),
            stage: Some(stage),
            course: course.map(str::to_string),
        });
    }

    pub(crate) async fn run(&self) -> Result<SyncReport, SourceError> {
        // Same reading as when the source was added: a moved or redirecting Canvas says so.
        self.step(
            SyncStage::CheckingAccess,
            None,
            "Checking the Canvas token".into(),
            None,
            None,
        );
        let _user: json::User = self
            .api
            .get_one(Endpoint::UsersSelf)
            .await
            .map_err(crate::probe_error)?;
        self.run_inner().await.map_err(|err| {
            fatal(&err).unwrap_or_else(|| SourceError::other(format!("Canvas sync failed: {err}.")))
        })
    }

    async fn run_inner(&self) -> Result<SyncReport, CanvasError> {
        let mut report = SyncReport::default();

        self.step(
            SyncStage::ListingCourses,
            None,
            "Listing courses".into(),
            None,
            None,
        );
        let listing = self.api.get_all::<json::Course>(Endpoint::Courses).await?;
        if !listing.complete() {
            self.warn(
                &mut report,
                "The course list could not be read completely; no course was removed".into(),
            );
        }
        let active: Vec<(&json::Course, CourseUpsert)> = listing
            .items
            .iter()
            .filter_map(|c| Some((c, map::course(self.ids(), &self.api.base, c)?)))
            .collect();
        // Removed courses (a pending or purged tombstone) aren't synced; a restore is.
        let source_id = self.source_id.to_string();
        let tombstones =
            with_store(self.db, move |store| store.tombstone_states(&source_id)).await?;
        let removed = |upsert: &CourseUpsert| {
            tombstones
                .get(&upsert.external_id)
                .is_some_and(|state| state.skipped_by_sync())
        };
        for (_, upsert) in &active {
            if removed(upsert) && !self.options.only_courses.is_empty() && self.wanted(upsert) {
                self.warn(
                    &mut report,
                    format!(
                        "{}: removed from PageLamp; restore it first",
                        upsert.code.as_deref().unwrap_or(&upsert.name)
                    ),
                );
            }
        }
        let selected: Vec<&(&json::Course, CourseUpsert)> = active
            .iter()
            .filter(|(_, upsert)| self.wanted(upsert) && !removed(upsert))
            .collect();
        let selected_ids: HashSet<String> = selected.iter().map(|(_, u)| u.id.clone()).collect();

        let source_id = self.source_id.to_string();
        let existing_events: Vec<Event> = with_store(self.db, move |store| {
            let all =
                store.list_events(DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC, None)?;
            Ok(all
                .into_iter()
                .filter(|e| e.source_id == source_id)
                .collect())
        })
        .await?;

        let mut new_events: Vec<Event> = Vec::new();
        let mut refreshed_courses: HashSet<String> = HashSet::new();
        for (index, (canvas, upsert)) in selected.iter().enumerate() {
            self.check_cancelled()?;
            let label = upsert.code.clone().unwrap_or_else(|| upsert.name.clone());
            self.step(
                SyncStage::ReadingCourse,
                Some(&label),
                format!("{label}: reading"),
                Some(index + 1),
                Some(selected.len()),
            );
            let warnings_before = report.warnings.len();
            match self.sync_course(canvas, upsert, &label, &mut report).await {
                Ok(result) => {
                    report.course_summaries.push(CourseSyncSummary {
                        course: label.clone(),
                        modules: to_u32(result.modules),
                        pages: to_u32(result.pages),
                        files: to_u32(result.files),
                        events: to_u32(result.events.as_ref().map_or(0, Vec::len)),
                        warnings: to_u32(report.warnings.len() - warnings_before),
                    });
                    report.courses += 1;
                    report.modules += result.modules;
                    report.materials += result.materials;
                    report.files_downloaded += result.files_downloaded;
                    report.files_indexed += result.files_indexed;
                    if let Some(events) = result.events {
                        refreshed_courses.insert(upsert.id.clone());
                        new_events.extend(events);
                    }
                }
                Err(err) if fatal(&err).is_some() => return Err(err),
                Err(err) => self.warn(&mut report, format!("{label}: skipped ({err})")),
            }
        }

        // Planner items (personal notes, calendar events, ungraded to-dos) of the synced
        // courses; items of other courses are left alone.
        let course_map: HashMap<String, String> = selected
            .iter()
            .map(|(canvas, upsert)| (canvas.id.0.clone(), upsert.id.clone()))
            .collect();
        let today = self.now.date_naive();
        let planner = self
            .api
            .get_all::<json::PlannerItem>(Endpoint::PlannerItems {
                start: today - TimeDelta::days(PLANNER_DAYS_BACK),
                end: today + TimeDelta::days(PLANNER_DAYS_AHEAD),
            })
            .await;
        let planner_refreshed = match planner {
            Ok(listing) if listing.complete() => {
                new_events.extend(listing.items.iter().filter_map(|item| {
                    map::planner_item(
                        self.ids(),
                        &self.api.base,
                        item,
                        |id| course_map.get(&id.0).cloned(),
                        self.now,
                    )
                }));
                true
            }
            Ok(_) => {
                self.warn(
                    &mut report,
                    "Planner items could not be read completely".into(),
                );
                false
            }
            Err(err) if fatal(&err).is_some() => return Err(err),
            Err(err) => {
                self.warn(&mut report, format!("Planner items not updated ({err})"));
                false
            }
        };

        // Courses are never deleted here (a course that left Canvas's active list — term
        // ended, enrollment concluded — is what a student needs during exams). After a
        // complete course list, record which ones are still active; `listed` includes
        // date-restricted stubs, which are still enrolled.
        let listed: Vec<String> = listing
            .items
            .iter()
            .map(|c| self.ids().course(&c.id))
            .collect();
        let mark_enrollment = listing.complete();
        // Listed but restricted by date: never upserted (`map::course`), so only the flag of
        // an existing row is recorded (calendar design §5 S2: such a course can't be synced
        // again once removed).
        let restricted: Vec<String> = listing
            .items
            .iter()
            .filter(|c| c.access_restricted_by_date == Some(true))
            .map(|c| self.ids().course(&c.id))
            .collect();
        let planner_prefix = format!("{}/planner/", self.source_id);
        let kept = existing_events.into_iter().filter(|event| {
            if event.id.starts_with(&planner_prefix) {
                let refreshed_here = match &event.course_id {
                    None => true,
                    Some(course) => selected_ids.contains(course),
                };
                return !(planner_refreshed && refreshed_here);
            }
            event
                .course_id
                .as_ref()
                .is_none_or(|course| !refreshed_courses.contains(course))
        });
        let mut events: BTreeMap<String, Event> = kept.map(|e| (e.id.clone(), e)).collect();
        for event in new_events {
            events.insert(event.id.clone(), event);
        }
        let events: Vec<Event> = events.into_values().collect();
        report.events = events.len();

        let source_id = self.source_id.to_string();
        with_store(self.db, move |store| {
            store.replace_events(&source_id, &events)?;
            if mark_enrollment {
                store.mark_enrollment_active(&source_id, &listed)?;
            }
            for course in &restricted {
                match store.set_course_access_restricted(course, true) {
                    // A course never seen with its full fields has no row: nothing to mark.
                    Ok(()) | Err(pagelamp_core::Error::NotFound(_)) => {}
                    Err(err) => return Err(err),
                }
            }
            Ok(())
        })
        .await?;
        report.requests = self.api.transport.requests_made();
        Ok(report)
    }

    /// `only_courses` matches our id, the Canvas id, or a code prefix (case/space-insensitive).
    fn wanted(&self, course: &CourseUpsert) -> bool {
        if self.options.only_courses.is_empty() {
            return true;
        }
        let code = course.code.as_deref().map(squash).unwrap_or_default();
        self.options.only_courses.iter().any(|want| {
            want == &course.id
                || want == &course.external_id
                || (!code.is_empty() && !want.trim().is_empty() && code.starts_with(&squash(want)))
        })
    }

    /// A per-course problem: fatal ones propagate, others become a warning.
    fn soft(
        &self,
        report: &mut SyncReport,
        label: &str,
        what: &str,
        err: CanvasError,
    ) -> Result<(), CanvasError> {
        if fatal(&err).is_some() {
            return Err(err);
        }
        self.warn(report, format!("{label}: {what} not available ({err})"));
        Ok(())
    }

    /// A listing that loses items must not be used for pruning: warn and report incomplete.
    fn check_listing<I>(
        &self,
        report: &mut SyncReport,
        label: &str,
        what: &str,
        listing: &Listing<I>,
    ) -> bool {
        if !listing.complete() {
            self.warn(
                report,
                format!("{label}: {what} could not be read completely; nothing of it was removed"),
            );
        }
        listing.complete()
    }

    async fn sync_course(
        &self,
        canvas: &json::Course,
        upsert: &CourseUpsert,
        label: &str,
        report: &mut SyncReport,
    ) -> Result<CourseResult, CanvasError> {
        let api = self.api;
        let cid = &canvas.id;
        let course_id = upsert.id.clone();
        let ids = self.ids();

        // Tabs tell which areas the student can see (hidden tabs are usually omitted).
        let tabs = match api
            .get_all::<json::Tab>(Endpoint::Tabs { course: cid })
            .await
        {
            Ok(listing) => Some(listing.items),
            Err(err) => {
                self.soft(report, label, "tabs", err)?;
                None
            }
        };
        let visible = |tab: &str| {
            tabs.as_ref()
                .is_none_or(|tabs| tabs.iter().any(|t| t.id == tab && t.hidden != Some(true)))
        };

        // ---- modules + items ------------------------------------------------------------------
        let mut modules_ok = true;
        let mut modules: Option<Vec<Module>> = None;
        let mut module_items: Vec<(Placement, json::ModuleItem)> = Vec::new();
        match api
            .get_all::<json::Module>(Endpoint::Modules { course: cid })
            .await
        {
            Ok(listing) => {
                modules_ok &= self.check_listing(report, label, "modules", &listing);
                let mut list = Vec::new();
                for module in &listing.items {
                    let mapped = map::module(ids, &course_id, module);
                    let placement = Placement {
                        module_id: Some(mapped.id.clone()),
                        module_week: mapped.week_hint,
                    };
                    let items: Vec<json::ModuleItem> = match &module.items {
                        Some(values) => {
                            let parsed: Vec<json::ModuleItem> = values
                                .iter()
                                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                                .collect();
                            modules_ok &= parsed.len() == values.len();
                            parsed
                        }
                        None if module.items_count.unwrap_or(1) > 0 => {
                            let endpoint = Endpoint::ModuleItems {
                                course: cid,
                                module: &module.id,
                            };
                            match api.get_all::<json::ModuleItem>(endpoint).await {
                                Ok(items) => {
                                    modules_ok &=
                                        self.check_listing(report, label, "module items", &items);
                                    items.items
                                }
                                Err(err) => {
                                    modules_ok = false;
                                    self.soft(report, label, "module items", err)?;
                                    Vec::new()
                                }
                            }
                        }
                        None => Vec::new(),
                    };
                    module_items.extend(items.into_iter().map(|item| (placement.clone(), item)));
                    list.push(mapped);
                }
                modules = Some(list);
            }
            Err(err) => {
                modules_ok = false;
                self.soft(report, label, "modules", err)?;
            }
        }

        // ---- files (only when the Files tab is visible; module items still bring files) --------
        let mut files_ok = modules_ok;
        let mut files: HashMap<CanvasId, json::File> = HashMap::new();
        if visible("files") {
            match api
                .get_all::<json::File>(Endpoint::Files { course: cid })
                .await
            {
                Ok(listing) => {
                    files_ok &= self.check_listing(report, label, "the files list", &listing);
                    files.extend(listing.items.into_iter().map(|f| (f.id.clone(), f)));
                }
                Err(err) => {
                    files_ok = false;
                    self.soft(report, label, "files list", err)?;
                }
            }
        } else {
            files_ok = false;
            self.warn(
                report,
                format!("{label}: Files tab hidden, used module items only"),
            );
        }

        // ---- pages ------------------------------------------------------------------------------
        let mut pages_ok = modules_ok;
        let mut pages: HashMap<String, json::Page> = HashMap::new(); // by slug
        if visible("pages") {
            match api
                .get_all::<json::Page>(Endpoint::Pages { course: cid })
                .await
            {
                Ok(listing) => {
                    pages_ok &= self.check_listing(report, label, "the pages list", &listing);
                    for page in listing.items {
                        match page.url.clone() {
                            Some(slug) => {
                                pages.insert(slug, page);
                            }
                            None => pages_ok = false,
                        }
                    }
                }
                Err(err) => {
                    pages_ok = false;
                    self.soft(report, label, "pages", err)?;
                }
            }
        } else {
            pages_ok = false;
        }

        // ---- assignments → due dates only ----------------------------------------------------
        let events = match api
            .get_all::<json::Assignment>(Endpoint::Assignments { course: cid })
            .await
        {
            Ok(listing) => {
                let complete = self.check_listing(report, label, "assignments", &listing);
                let events: Vec<Event> = listing
                    .items
                    .iter()
                    .filter_map(|a| map::assignment(ids, &api.base, &course_id, a, self.now))
                    .collect();
                complete.then_some(events)
            }
            Err(err) => {
                self.soft(report, label, "assignments", err)?;
                None
            }
        };

        // ---- announcements of the window ----------------------------------------------------------
        let today = self.now.date_naive();
        let window_start = today - TimeDelta::days(ANNOUNCEMENT_DAYS);
        let mut announcements_ok = true;
        let announcements = match api
            .get_all::<json::Announcement>(Endpoint::Announcements {
                course: cid,
                start: window_start,
                end: today + TimeDelta::days(1),
            })
            .await
        {
            Ok(listing) => {
                announcements_ok &= self.check_listing(report, label, "announcements", &listing);
                listing.items
            }
            Err(err) => {
                announcements_ok = false;
                self.soft(report, label, "announcements", err)?;
                Vec::new()
            }
        };

        // ---- build materials -----------------------------------------------------------------------
        let existing: HashMap<String, Material> = {
            let course_id = course_id.clone();
            with_store(self.db, move |store| {
                Ok(store
                    .list_materials(&course_id)?
                    .into_iter()
                    .map(|m| (m.id.clone(), m))
                    .collect())
            })
            .await?
        };
        let mut materials: BTreeMap<String, MaterialUpsert> = BTreeMap::new();
        let mut file_objects: HashMap<String, json::File> = HashMap::new();
        let mut html_jobs: Vec<HtmlJob> = Vec::new();
        let mut placements: HashMap<String, Placement> = HashMap::new();

        for file in files.values() {
            let material = map::file(
                ids,
                &api.base,
                cid,
                &course_id,
                file,
                None,
                &Placement::default(),
            );
            file_objects.insert(material.id.clone(), clone_file(file));
            materials.insert(material.id.clone(), material);
        }
        for (placement, item) in &module_items {
            match item.kind.as_deref() {
                Some("File") => {
                    let Some(file_id) = &item.content_id else {
                        files_ok = false;
                        continue;
                    };
                    let file = match files.get(file_id) {
                        Some(file) => Some(clone_file(file)),
                        None => match api
                            .get_one::<json::File>(Endpoint::File {
                                course: cid,
                                file: file_id,
                            })
                            .await
                        {
                            Ok(file) => Some(file),
                            Err(err) => {
                                files_ok = false;
                                self.soft(report, label, "a module file", err)?;
                                None
                            }
                        },
                    };
                    if let Some(file) = file {
                        let material = map::file(
                            ids,
                            &api.base,
                            cid,
                            &course_id,
                            &file,
                            item.title.as_deref(),
                            placement,
                        );
                        file_objects.insert(material.id.clone(), file);
                        materials.insert(material.id.clone(), material);
                    }
                }
                Some("Page") => match &item.page_url {
                    Some(slug) => {
                        placements.insert(slug.clone(), placement.clone());
                        pages.entry(slug.clone()).or_default();
                    }
                    None => pages_ok = false,
                },
                Some("ExternalUrl") => {
                    if let Some(material) = map::link(ids, &course_id, item, placement) {
                        materials.insert(material.id.clone(), material);
                    }
                }
                _ => {} // assignments, quizzes, discussions, headers: not course material
            }
        }
        for (slug, listed) in &pages {
            let placement = placements.get(slug).cloned().unwrap_or_default();
            let unchanged = listed.page_id.as_ref().is_some_and(|id| {
                existing.get(&ids.page(id)).is_some_and(|old| {
                    old.text_status == TextStatus::Ok
                        && listed.updated_at.is_some()
                        && old.published_at == listed.updated_at
                })
            });
            let page = if unchanged || listed.locked_for_user == Some(true) {
                None
            } else {
                match api
                    .get_one::<json::Page>(Endpoint::Page {
                        course: cid,
                        url_or_id: slug,
                    })
                    .await
                {
                    Ok(page) => Some(page),
                    Err(err) => {
                        pages_ok = false;
                        self.soft(report, label, "a page", err)?;
                        None
                    }
                }
            };
            let source = page.as_ref().unwrap_or(listed);
            let Some(page_id) = source.page_id.clone().or_else(|| listed.page_id.clone()) else {
                pages_ok = false;
                continue;
            };
            let material = map::page(ids, &api.base, &course_id, &page_id, source, &placement);
            if let Some(body) = page.and_then(|p| p.body) {
                html_jobs.push(HtmlJob {
                    material_id: material.id.clone(),
                    html: body,
                });
            }
            materials.insert(material.id.clone(), material);
        }
        for announcement in &announcements {
            let material = map::announcement(ids, &api.base, &course_id, announcement);
            if let Some(message) = &announcement.message {
                html_jobs.push(HtmlJob {
                    material_id: material.id.clone(),
                    html: message.clone(),
                });
            }
            materials.insert(material.id.clone(), material);
        }
        if let Some(body) = canvas
            .syllabus_body
            .as_deref()
            .filter(|b| !b.trim().is_empty())
        {
            let material = map::syllabus(ids, &api.base, cid, &course_id);
            html_jobs.push(HtmlJob {
                material_id: material.id.clone(),
                html: body.to_string(),
            });
            materials.insert(material.id.clone(), material);
        }

        // ---- files: download (when asked), keep earlier copies, or mark not downloaded -----------
        let course_dir = crate::course_files_dir(
            &self.options.files_dir,
            upsert.code.as_deref(),
            &upsert.external_id,
        );
        let mut downloads: Vec<DownloadJob> = Vec::new();
        let mut reindex: Vec<ReindexJob> = Vec::new();
        let mut not_downloaded: Vec<String> = Vec::new();
        // Why a not-downloaded file can't be downloaded on request; other files are cleared.
        let mut blocked: HashMap<String, DownloadBlock> = HashMap::new();
        for (id, material) in materials.iter_mut() {
            let Some(file) = file_objects.get(id) else {
                continue;
            };
            let old = existing.get(id);
            // An earlier sync downloaded (and processed) this file and the copy is still there.
            let copy = old.filter(|m| {
                !matches!(
                    m.text_status,
                    TextStatus::Pending | TextStatus::NotDownloaded
                ) && m
                    .local_path
                    .as_deref()
                    .is_some_and(|p| Path::new(p).is_file())
            });
            let same_version = copy.is_some_and(|m| m.published_at == material.published_at);
            if same_version {
                material.local_path = copy.and_then(|m| m.local_path.clone());
                if let Some(old) = copy.filter(|m| ingest::needs_retry(m))
                    && let Some(path) = &old.local_path
                {
                    reindex.push(ReindexJob {
                        material_id: id.clone(),
                        title: material.title.clone(),
                        path: PathBuf::from(path),
                        mime: material.mime.clone(),
                    });
                }
                continue;
            }
            let downloadable = self.options.download_files
                && file.locked_for_user != Some(true)
                && file.size.is_none_or(|s| s <= self.options.max_file_bytes);
            let link = file.url.as_deref().and_then(|u| url::Url::parse(u).ok());
            let has_link = link.is_some();
            match (downloadable, link) {
                (true, Some(url)) => {
                    let dest = course_dir.join(file_name(file));
                    let job = DownloadJob {
                        material: {
                            let mut new = material.clone();
                            new.local_path = Some(dest.to_string_lossy().to_string());
                            new
                        },
                        url,
                        dest,
                        had_copy: copy.is_some(),
                    };
                    // Until the new text is indexed, the stored row keeps the OLD version date
                    // (and copy), so an interrupted download is retried next time.
                    material.published_at = old.and_then(|m| m.published_at);
                    material.local_path = copy.and_then(|m| m.local_path.clone());
                    if copy.is_none() {
                        not_downloaded.push(id.clone());
                    }
                    downloads.push(job);
                }
                _ => {
                    if self.options.download_files
                        && file.size.is_some_and(|s| s > self.options.max_file_bytes)
                    {
                        self.warn(
                            report,
                            format!(
                                "{label}: {} skipped (larger than the download limit)",
                                material.title
                            ),
                        );
                    }
                    match copy {
                        // Keep the earlier text (and its version date, so a later download
                        // sync still sees that the file changed).
                        Some(copy) => {
                            material.local_path = copy.local_path.clone();
                            material.published_at = copy.published_at;
                        }
                        None => {
                            not_downloaded.push(id.clone());
                            let max = self.options.max_file_bytes;
                            let block = if file.locked_for_user == Some(true) || !has_link {
                                // Canvas gives no (usable) download link to this student.
                                Some(DownloadBlock::Locked)
                            } else if let Some(size) = file.size {
                                (size > max).then_some(DownloadBlock::TooLarge)
                            } else {
                                // Size not listed: keep what an earlier download found out.
                                old.and_then(|m| m.download_blocked)
                                    .filter(|b| *b == DownloadBlock::TooLarge)
                            };
                            if let Some(block) = block {
                                blocked.insert(id.clone(), block);
                            }
                        }
                    }
                }
            }
        }
        let links: Vec<String> = materials
            .values()
            .filter(|m| m.kind == MaterialKind::ExternalLink)
            .map(|m| m.id.clone())
            .collect();

        // ---- write metadata in one short transaction -----------------------------------------------
        let complete = Complete {
            files: files_ok,
            pages: pages_ok,
            links: modules_ok,
            announcements: announcements_ok,
        };
        let window_start_instant = window_start
            .and_hms_opt(0, 0, 0)
            .expect("midnight exists")
            .and_utc();
        let mut keep: Vec<String> = materials.keys().cloned().collect();
        keep.extend(
            existing
                .values()
                .filter(|m| {
                    !complete.kind(m.kind)
                        || (m.kind == MaterialKind::Announcement
                            && m.published_at.is_none_or(|p| p < window_start_instant))
                })
                .map(|m| m.id.clone()),
        );
        let result_modules = modules.as_ref().map_or(0, Vec::len);
        let result_materials = materials.len();
        let result_pages = materials
            .values()
            .filter(|m| m.kind == MaterialKind::Page)
            .count();
        let result_files = materials
            .values()
            .filter(|m| m.kind == MaterialKind::File)
            .count();
        {
            let upsert = upsert.clone();
            let course_id = course_id.clone();
            let materials: Vec<MaterialUpsert> = materials.into_values().collect();
            let modules = modules.clone();
            with_store(self.db, move |store| {
                store.in_transaction(|store| {
                    store.upsert_course(&upsert)?;
                    let module_ids: HashSet<String> = match &modules {
                        Some(modules) => {
                            store.replace_modules(&course_id, modules)?;
                            modules.iter().map(|m| m.id.clone()).collect()
                        }
                        None => store
                            .list_modules(&course_id)?
                            .into_iter()
                            .map(|m| m.id)
                            .collect(),
                    };
                    for material in &materials {
                        let mut material = material.clone();
                        if material
                            .module_id
                            .as_ref()
                            .is_some_and(|m| !module_ids.contains(m))
                        {
                            material.module_id = None;
                        }
                        store.upsert_material(&material)?;
                        if material.kind == MaterialKind::File {
                            let block = blocked.get(&material.id).copied();
                            store.set_download_blocked(&material.id, block)?;
                        }
                    }
                    for id in &not_downloaded {
                        store.set_text_state(id, TextStatus::NotDownloaded, None, None)?;
                    }
                    for id in &links {
                        store.set_text_state(id, TextStatus::Unsupported, None, None)?;
                    }
                    store.prune_materials(&course_id, &keep)?;
                    Ok(())
                })
            })
            .await?;
        }

        // ---- index HTML (pages, announcements, syllabus) ------------------------------------------
        let indexed_html = with_store(self.db, move |store| {
            let mut indexed = 0;
            for job in html_jobs {
                if let IndexOutcome::Indexed { .. } | IndexOutcome::Empty =
                    ingest::index_html(store, &job.material_id, &job.html)?
                {
                    indexed += 1;
                }
            }
            Ok(indexed)
        })
        .await?;

        // ---- download + index files (only when asked) ---------------------------------------------
        let mut files_downloaded = 0;
        let mut files_indexed = 0;
        let total = downloads.len();
        for (index, job) in downloads.into_iter().enumerate() {
            self.check_cancelled()?;
            self.step(
                SyncStage::DownloadingFiles,
                Some(label),
                format!("{label}: downloading files"),
                Some(index + 1),
                Some(total),
            );
            let title = job.material.title.clone();
            match api
                .transport
                .download(
                    job.url.clone(),
                    job.dest.clone(),
                    self.options.max_file_bytes,
                )
                .await
            {
                Ok(_) => {
                    files_downloaded += 1;
                    let (material, dest) = (job.material, job.dest);
                    let extractor = self.options.extractor.clone();
                    let outcome = with_store(self.db, move |store| {
                        // Point the row at the new copy, index it, and only then record the
                        // new version date (upsert keeps the text state it just got).
                        let mut pending = material.clone();
                        pending.published_at = store
                            .get_material(&material.id)?
                            .and_then(|m| m.published_at);
                        store.upsert_material(&pending)?;
                        // Record the new version unless the copy couldn't even be read (then
                        // retry next time). A file whose text can't be extracted is not
                        // downloaded again and again: each download counts as a view.
                        let outcome = match ingest::index_file_using(
                            store,
                            &material.id,
                            &dest,
                            material.mime.as_deref(),
                            &extractor,
                        ) {
                            Ok(outcome) => {
                                store.upsert_material(&material)?;
                                outcome
                            }
                            Err(pagelamp_core::Error::Io(err)) => {
                                IndexOutcome::Failed(err.to_string())
                            }
                            Err(other) => return Err(other),
                        };
                        Ok(outcome)
                    })
                    .await?;
                    if self.record_index(report, label, &title, outcome) {
                        files_indexed += 1;
                    }
                }
                // An expired token aborts; anything else about ONE file is a warning.
                Err(CanvasError::Unauthorized) => return Err(CanvasError::Unauthorized),
                Err(err) => {
                    // Larger than its listed size said: asking again won't help.
                    if matches!(err, CanvasError::TooLarge) && !job.had_copy {
                        let id = job.material.id.clone();
                        with_store(self.db, move |store| {
                            store.set_download_blocked(&id, Some(DownloadBlock::TooLarge))
                        })
                        .await?;
                    }
                    let note = if job.had_copy {
                        " (kept the earlier copy)"
                    } else {
                        ""
                    };
                    self.warn(
                        report,
                        format!("{label}: {title} could not be downloaded ({err}){note}"),
                    );
                }
            }
        }

        // ---- read cached copies again that the text reader couldn't read last time --------------
        for job in reindex {
            self.check_cancelled()?;
            let extractor = self.options.extractor.clone();
            let ReindexJob {
                material_id,
                title,
                path,
                mime,
            } = job;
            let outcome = with_store(self.db, move |store| {
                match ingest::index_file_using(
                    store,
                    &material_id,
                    &path,
                    mime.as_deref(),
                    &extractor,
                ) {
                    Ok(outcome) => Ok(outcome),
                    Err(pagelamp_core::Error::Io(err)) => Ok(IndexOutcome::Failed(err.to_string())),
                    Err(other) => Err(other),
                }
            })
            .await?;
            if self.record_index(report, label, &title, outcome) {
                files_indexed += 1;
            }
        }

        // ---- remove local copies nothing refers to any more --------------------------------------
        if complete.files && course_dir.is_dir() {
            let course_id = course_id.clone();
            let referenced: HashSet<PathBuf> = with_store(self.db, move |store| {
                Ok(store
                    .list_materials(&course_id)?
                    .into_iter()
                    .filter_map(|m| m.local_path.map(PathBuf::from))
                    .collect())
            })
            .await?;
            if let Ok(mut entries) = tokio::fs::read_dir(&course_dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let path = entry.path();
                    if entry.file_type().await.is_ok_and(|t| t.is_file())
                        && !referenced.contains(&path)
                    {
                        let _ = tokio::fs::remove_file(path).await;
                    }
                }
            }
        }
        if !complete.all() {
            tracing::debug!(course = %label, "partial listing: kept existing materials of incomplete kinds");
        }

        Ok(CourseResult {
            modules: result_modules,
            materials: result_materials,
            pages: result_pages,
            files: result_files,
            files_downloaded,
            files_indexed: files_indexed + indexed_html,
            events,
        })
    }
}

/// `json::File` is not `Clone` (it never needs to be, except here).
fn clone_file(file: &json::File) -> json::File {
    json::File {
        id: file.id.clone(),
        display_name: file.display_name.clone(),
        filename: file.filename.clone(),
        content_type: file.content_type.clone(),
        size: file.size,
        url: file.url.clone(),
        created_at: file.created_at,
        updated_at: file.updated_at,
        locked_for_user: file.locked_for_user,
    }
}

/// `<CODE>-<canvas id>` with only safe characters (a directory name under `files_dir`).
pub(crate) fn course_dir_name(code: Option<&str>, external_id: &str) -> String {
    format!(
        "{}-{}",
        safe_name(code.unwrap_or("course"), 40),
        safe_name(external_id, 40)
    )
}

/// `<file id>-<sanitised name>`, at most `MAX_FILE_NAME_BYTES` bytes with the extension kept:
/// unique per file, never a path traversal, valid on every file system.
fn file_name(file: &json::File) -> String {
    let name = file
        .display_name
        .as_deref()
        .or(file.filename.as_deref())
        .unwrap_or("file");
    let name = safe_name(name, 200);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ext.len() <= 10 => {
            (stem.to_string(), format!(".{ext}"))
        }
        _ => (name.clone(), String::new()),
    };
    let prefix = format!("{}-", safe_name(&file.id.0, 40));
    let room = MAX_FILE_NAME_BYTES.saturating_sub(prefix.len() + ext.len());
    format!("{prefix}{}{ext}", truncate_bytes(&stem, room))
}

/// Longest prefix of `text` with at most `max` bytes, cut at a character boundary.
fn truncate_bytes(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Keep letters, digits, `.`, `-`, `_` and spaces; everything else (separators, `..` runs,
/// control characters) becomes `_`; no leading dot; at most `max` characters.
pub(crate) fn safe_name(name: &str, max: usize) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.replace("..", "_");
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    let cleaned: String = cleaned.chars().take(max).collect();
    if cleaned.is_empty() {
        "_".into()
    } else {
        cleaned
    }
}

fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe() {
        assert_eq!(safe_name("../../etc/passwd", 120), "____etc_passwd");
        assert_eq!(safe_name(".hidden", 120), "hidden");
        assert_eq!(safe_name("Week 3 slides.pdf", 120), "Week 3 slides.pdf");
        assert_eq!(safe_name("a\u{0}b/c\\d:e", 120), "a_b_c_d_e");
        assert_eq!(safe_name("", 120), "_");
        assert_eq!(safe_name(&"x".repeat(500), 120).len(), 120);
        assert!(!safe_name("....", 10).contains(".."));
    }

    #[test]
    fn local_file_names_fit_every_file_system_and_keep_the_extension() {
        let file = |name: &str| json::File {
            id: CanvasId("501".into()),
            display_name: Some(name.into()),
            filename: None,
            content_type: None,
            size: None,
            url: None,
            created_at: None,
            updated_at: None,
            locked_for_user: None,
        };
        let cjk = file_name(&file(&("講".repeat(100) + ".pdf")));
        assert!(cjk.len() <= MAX_FILE_NAME_BYTES, "{} bytes", cjk.len());
        assert!(cjk.starts_with("501-講") && cjk.ends_with(".pdf"));
        assert_eq!(file_name(&file("notes.part")), "501-notes.part");
        assert_eq!(file_name(&file("../x")), "501-__x");
        assert_eq!(
            course_dir_name(Some("DEMO101 F/LEC"), "101"),
            "DEMO101 F_LEC-101"
        );
    }
}
