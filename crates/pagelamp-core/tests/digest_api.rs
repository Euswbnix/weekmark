//! The weekly digest on synthetic data: visible courses with their week, materials and next
//! deadlines, and the study plan's progress.

use chrono::{Duration, NaiveDate, TimeZone, Utc};
use pagelamp_core::model::*;
use pagelamp_core::store::Store;
use pagelamp_core::views::{AsOf, weekly_digest};

const SOURCE: &str = "folder:demo";

fn at() -> AsOf {
    AsOf {
        now: Utc.with_ymd_and_hms(2026, 9, 24, 12, 0, 0).unwrap(),
        today: NaiveDate::from_ymd_opt(2026, 9, 24).unwrap(),
        tz: None,
    }
}

fn course(store: &Store, code: &str) -> String {
    let id = format!("{SOURCE}/course/{code}");
    store
        .upsert_course(&CourseUpsert {
            id: id.clone(),
            source_id: SOURCE.into(),
            external_id: code.into(),
            code: Some(code.into()),
            name: format!("{code} Demo Studies"),
            // Week 3 on 2026-09-24.
            term_start: NaiveDate::from_ymd_opt(2026, 9, 7),
            term_end: NaiveDate::from_ymd_opt(2026, 12, 18),
            url: None,
            syllabus_text: None,
            lms: Default::default(),
        })
        .unwrap();
    for n in 1..=7 {
        store
            .upsert_material(&MaterialUpsert {
                id: format!("{id}/material/{n}"),
                course_id: id.clone(),
                module_id: None,
                kind: MaterialKind::File,
                title: format!("Week 3 part {n}"),
                url: None,
                local_path: None,
                mime: None,
                published_at: None,
                week_hint: Some(3),
            })
            .unwrap();
    }
    id
}

fn event(id: &str, course_id: &str, title: &str, due_in_days: i64) -> Event {
    Event {
        id: id.into(),
        source_id: SOURCE.into(),
        course_id: Some(course_id.into()),
        kind: EventKind::AssignmentDue,
        title: title.into(),
        starts_at: None,
        ends_at: None,
        due_at: Some(at().now + Duration::days(due_in_days)),
        url: None,
        updated_at: at().now,
        course_hint: None,
    }
}

fn item(date: NaiveDate, title: &str, done: bool) -> StudyPlanItem {
    StudyPlanItem {
        date,
        course_id: None,
        title: title.into(),
        description: None,
        material_ids: Vec::new(),
        minutes: Some(30),
        done,
    }
}

#[test]
fn the_digest_lists_this_week_deadlines_and_plan_progress() {
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_source(&SourceRecord {
            id: SOURCE.into(),
            kind: SourceKind::Folder,
            label: "Demo courses".into(),
            config: serde_json::json!({ "path": "/demo/courses" }),
            last_synced_at: None,
            last_error: None,
            last_error_kind: None,
        })
        .unwrap();
    let visible = course(&store, "DEMO101");
    let hidden = course(&store, "DEMO404");
    store.set_course_hidden(&hidden, true).unwrap();
    store
        .replace_events(
            SOURCE,
            &[
                event("e1", &visible, "A2 due", 2),
                event("e2", &visible, "A3 due", 20),
                event("e3", &hidden, "Hidden due", 1),
            ],
        )
        .unwrap();
    let today = at().today;
    store
        .save_study_plan(&StudyPlan {
            horizon_start: today - Duration::days(7),
            horizon_end: today + Duration::days(7),
            items: vec![
                item(today - Duration::days(3), "Read week 2", true),
                item(today - Duration::days(1), "Review quiz", false),
                item(today - Duration::days(9), "Too old", true),
                item(today, "Practice problems", false),
                // The hidden course's, by id and by code: not counted, not listed.
                StudyPlanItem {
                    course_id: Some(hidden.clone()),
                    ..item(today - Duration::days(2), "Hidden course review", true)
                },
                StudyPlanItem {
                    course_id: Some("demo404".into()),
                    ..item(today, "Hidden course practice", false)
                },
            ],
            notes: None,
        })
        .unwrap();

    let digest = weekly_digest(&store, at()).unwrap();
    assert_eq!(digest.courses.len(), 1, "hidden courses are left out");
    let course = &digest.courses[0];
    assert_eq!(course.code.as_deref(), Some("DEMO101"));
    assert_eq!(course.week, Some(3));
    assert_eq!(course.material_count, 7);
    assert_eq!(course.material_titles.len(), 5);
    let deadlines: Vec<&str> = course
        .deadlines
        .iter()
        .map(|d| d.event.title.as_str())
        .collect();
    assert_eq!(deadlines, ["A2 due"], "only the next 7 days");
    let plan = digest.plan.unwrap();
    assert_eq!((plan.last_week_done, plan.last_week_planned), (1, 2));
    assert_eq!(plan.today.len(), 1);
    assert_eq!(plan.today[0].title, "Practice problems");
}
