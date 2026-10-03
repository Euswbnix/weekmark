//! Stored calendars in the views (schema 4; docs/design/v0.3-course-calendar.md §6.4, §7.8,
//! §7.10): the calendar in force is the resolver's first anchor, with its provenance, and the
//! Timeline tab's status says whether it is proposed, accepted or stale. Synthetic data only.

use chrono::{DateTime, NaiveDate, Utc};
use pagelamp_core::ai_gate::ManifestEntry;
use pagelamp_core::calendar::assemble::{DateKind, ProposedDate};
use pagelamp_core::calendar::legacy_calendar;
use pagelamp_core::calendar::validate::DateEvidence;
use pagelamp_core::model::*;
use pagelamp_core::store::{CalendarChecks, CalendarProvenance, NewCalendarRow, Store};
use pagelamp_core::views::{self, AsOf};
use serde_json::json;

const COURSE: &str = "canvas:lms.example.edu/course/101";
const SYLLABUS: &str = "canvas:lms.example.edu/syllabus/101";

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").unwrap()
}

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&format!("{text}T12:00:00Z"))
        .unwrap()
        .with_timezone(&Utc)
}

/// Thursday of teaching week 3 when classes begin on 2026-09-08.
fn as_of() -> AsOf {
    AsOf {
        now: at("2026-09-24"),
        today: date("2026-09-24"),
        tz: None,
    }
}

fn syllabus(store: &Store, hash: &str, text: &str) {
    store
        .upsert_material(&MaterialUpsert {
            id: SYLLABUS.into(),
            course_id: COURSE.into(),
            module_id: None,
            kind: MaterialKind::Syllabus,
            title: "Syllabus".into(),
            url: None,
            local_path: None,
            mime: None,
            published_at: None,
            week_hint: None,
        })
        .unwrap();
    store
        .set_text_state(SYLLABUS, TextStatus::Ok, None, Some(hash))
        .unwrap();
    store
        .replace_chunks(
            SYLLABUS,
            &[Chunk {
                material_id: SYLLABUS.into(),
                ord: 0,
                locator: Some("p. 1".into()),
                text: text.into(),
            }],
        )
        .unwrap();
}

fn demo_store() -> (Store, Course) {
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_source(&SourceRecord {
            id: "canvas:lms.example.edu".into(),
            kind: SourceKind::Canvas,
            label: "Demo LMS".into(),
            config: json!({}),
            last_synced_at: None,
            last_error: None,
            last_error_kind: None,
        })
        .unwrap();
    store
        .upsert_course(&CourseUpsert {
            id: COURSE.into(),
            source_id: "canvas:lms.example.edu".into(),
            external_id: "101".into(),
            code: Some("DEMO101".into()),
            name: "Intro to Demo Studies".into(),
            term_start: None,
            term_end: None,
            url: None,
            syllabus_text: None,
            lms: Default::default(),
        })
        .unwrap();
    syllabus(&store, "h1", "Classes begin Tuesday, September 8, 2026.");
    let course = store.get_course(COURSE).unwrap().unwrap();
    (store, course)
}

/// An AI proposal quoting the syllabus for its first class.
fn ai_proposal() -> NewCalendarRow {
    NewCalendarRow {
        course_id: COURSE.into(),
        origin: CalendarOrigin::Ai,
        calendar: legacy_calendar(date("2026-09-08"), Some(date("2026-12-08"))),
        dates: vec![ProposedDate {
            kind: DateKind::FirstClass,
            segment: 0,
            date: date("2026-09-08"),
            end: None,
            label: "Classes begin".into(),
            evidence: vec![DateEvidence {
                material_id: SYLLABUS.into(),
                title: "Syllabus".into(),
                locator: Some("p. 1".into()),
                quote: Some("Classes begin Tuesday, September 8, 2026.".into()),
                url: None,
                derived: false,
            }],
            alternatives: Vec::new(),
            week: None,
            break_kind: None,
            numbered: None,
        }],
        checks: CalendarChecks {
            passing: true,
            ..CalendarChecks::default()
        },
        manifest: vec![ManifestEntry {
            material_id: SYLLABUS.into(),
            content_hash: Some("h1".into()),
            chunk_ords: vec![0],
        }],
        fingerprint: "f1".into(),
        provenance: Some(CalendarProvenance {
            generation_id: None,
            backend_label: "ChatGPT plan (through OpenAI Codex)".into(),
            model: "gpt-6-luna".into(),
            prompt_version: 1,
            on_device: false,
        }),
    }
}

#[test]
fn a_proposal_changes_nothing_until_it_is_accepted() {
    let (store, course) = demo_store();
    let timeline = views::course_timeline(&store, &course, as_of()).unwrap();
    assert_eq!(timeline.calendar, CalendarStatus::NoCalendar);
    assert_eq!(timeline.current_week, None, "no dates at all");

    let id = store
        .insert_calendar_proposal(&ai_proposal(), at("2026-09-23"))
        .unwrap();
    let timeline = views::course_timeline(&store, &course, as_of()).unwrap();
    assert_eq!(timeline.calendar, CalendarStatus::Proposed);
    assert_eq!(timeline.current_week, None, "a proposal has no effect");

    store
        .accept_calendar_proposal(id, None, at("2026-09-24"))
        .unwrap();
    let timeline = views::course_timeline(&store, &course, as_of()).unwrap();
    assert_eq!(timeline.calendar, CalendarStatus::Accepted);
    assert_eq!(timeline.term.anchor, TermAnchorSource::StudentConfirmed);
    assert_eq!(timeline.term.anchor_origin, Some(CalendarOrigin::Ai));
    assert_eq!(timeline.term.anchor_confidence, Confidence::High);
    let label = timeline.term.ai_label.expect("AI provenance");
    assert_eq!(
        (label.backend_label.as_str(), label.model.as_str()),
        ("ChatGPT plan (through OpenAI Codex)", "gpt-6-luna")
    );
    assert!(!label.on_device);
    assert_eq!(timeline.current_week, Some(3));
}

#[test]
fn an_on_device_reading_says_so_in_its_label() {
    let (store, course) = demo_store();
    let mut proposal = ai_proposal();
    let provenance = proposal.provenance.as_mut().unwrap();
    provenance.backend_label = "Ollama (this computer)".into();
    provenance.on_device = true;
    let id = store
        .insert_calendar_proposal(&proposal, at("2026-09-23"))
        .unwrap();
    assert!(
        store
            .calendar_row(id)
            .unwrap()
            .unwrap()
            .provenance
            .unwrap()
            .on_device
    );
    store
        .accept_calendar_proposal(id, None, at("2026-09-24"))
        .unwrap();
    let timeline = views::course_timeline(&store, &course, as_of()).unwrap();
    assert!(timeline.term.ai_label.unwrap().on_device);
}

#[test]
fn a_lost_quote_makes_the_calendar_stale_but_keeps_it_in_force() {
    let (store, course) = demo_store();
    let id = store
        .insert_calendar_proposal(&ai_proposal(), at("2026-09-23"))
        .unwrap();
    store
        .accept_calendar_proposal(id, None, at("2026-09-24"))
        .unwrap();
    // Edited, dates unchanged: still Accepted.
    syllabus(
        &store,
        "h2",
        "Welcome! Classes begin Tuesday, September 8, 2026.",
    );
    let timeline = views::course_timeline(&store, &course, as_of()).unwrap();
    assert_eq!(timeline.calendar, CalendarStatus::Accepted);
    // The words are gone: stale, Medium, and the weeks still count from it.
    syllabus(
        &store,
        "h3",
        "Welcome! Classes begin Wednesday, September 9, 2026.",
    );
    let timeline = views::course_timeline(&store, &course, as_of()).unwrap();
    assert_eq!(timeline.calendar, CalendarStatus::AcceptedStale);
    assert_eq!(timeline.term.anchor, TermAnchorSource::StudentConfirmed);
    assert_eq!(timeline.term.anchor_confidence, Confidence::Medium);
    assert_eq!(timeline.current_week, Some(3));
}

#[test]
fn the_students_own_dates_are_in_force_until_undone() {
    let (store, course) = demo_store();
    let calendar = legacy_calendar(date("2026-09-14"), Some(date("2026-12-11")));
    store
        .set_student_calendar(COURSE, Some(&calendar), at("2026-09-20"))
        .unwrap();
    let timeline = views::course_timeline(&store, &course, as_of()).unwrap();
    assert_eq!(timeline.term.anchor_origin, Some(CalendarOrigin::User));
    assert_eq!(timeline.current_week, Some(2));
    store
        .set_student_calendar(COURSE, None, at("2026-09-21"))
        .unwrap();
    let timeline = views::course_timeline(&store, &course, as_of()).unwrap();
    assert_eq!(timeline.calendar, CalendarStatus::NoCalendar);
    assert_eq!(timeline.term.anchor_origin, None);
}

#[test]
fn a_migrated_override_still_resolves_through_the_students_term_dates() {
    let (store, course) = demo_store();
    // What the v4 data step leaves for a 0.1 start-only override.
    store
        .set_course_term(COURSE, Some(date("2026-09-08")), None)
        .unwrap();
    store
        .conn()
        .execute(
            "INSERT INTO course_calendars
                 (course_id, origin, state, calendar_json, evidence_json, checks_json,
                  manifest_json, fingerprint, created_at, decided_at)
             VALUES (?1, 'legacy', 'accepted', ?2, '[]', '{}', '[]', ?3, ?4, ?4)",
            rusqlite::params![
                COURSE,
                serde_json::to_string(&legacy_calendar(date("2026-09-08"), None)).unwrap(),
                pagelamp_core::store::MIGRATED_FINGERPRINT,
                "2026-09-01T00:00:00Z"
            ],
        )
        .unwrap();
    let timeline = views::course_timeline(&store, &course, as_of()).unwrap();
    assert_eq!(timeline.calendar, CalendarStatus::Accepted);
    assert_eq!(timeline.term.anchor_origin, Some(CalendarOrigin::Legacy));
    assert_eq!(timeline.current_week, Some(3));
}

#[test]
fn the_sharing_reminder_is_claimed_once_per_course() {
    let (store, _) = demo_store();
    assert!(
        store
            .claim_sharing_reminder(COURSE, at("2026-09-24"))
            .unwrap()
    );
    assert!(
        !store
            .claim_sharing_reminder(COURSE, at("2026-09-25"))
            .unwrap()
    );
    assert!(
        store
            .claim_sharing_reminder("canvas:lms.example.edu/course/202", at("2026-09-25"))
            .unwrap()
    );
    // Removing all AI data starts over.
    store.remove_all_ai_data().unwrap();
    assert!(
        store
            .claim_sharing_reminder(COURSE, at("2026-09-26"))
            .unwrap()
    );
}
