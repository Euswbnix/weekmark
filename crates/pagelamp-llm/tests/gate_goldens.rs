//! Per-wire golden test of the gate (design §4.4, M1 DoD 2): the HTTP request body each wire
//! actually sends never contains text of a course that isn't `readable`, for an explanation
//! context and a structure-only plan context. Synthetic DEMO courses with canary words.

use pagelamp_core::ai::Destination;
use pagelamp_core::ai::Effort;
use pagelamp_core::ai_gate::{ContextBudget, PlanScope, assemble, plan_context, week_context};
use pagelamp_core::model::*;
use pagelamp_core::store::Store;
use pagelamp_core::views::AsOf;
use pagelamp_llm::profile::{ApiKey, preset};
use pagelamp_llm::{GenerateRequest, HttpDriver, OutputSpec, check_base_url};

const SOURCE: &str = "folder:demo";

fn course(store: &Store, code: &str, canary: &str) -> String {
    let id = format!("{SOURCE}/course/{code}");
    store
        .upsert_course(&CourseUpsert {
            id: id.clone(),
            source_id: SOURCE.into(),
            external_id: code.into(),
            code: Some(code.into()),
            name: format!("{code} Demo Studies"),
            term_start: None,
            term_end: None,
            url: None,
            syllabus_text: None,
            lms: Default::default(),
        })
        .unwrap();
    let material = format!("{id}/material/slides");
    store
        .upsert_material(&MaterialUpsert {
            id: material.clone(),
            course_id: id.clone(),
            module_id: None,
            kind: MaterialKind::File,
            title: "Week 3 slides".into(),
            url: None,
            local_path: None,
            mime: None,
            published_at: None,
            week_hint: Some(3),
        })
        .unwrap();
    store
        .set_text_state(&material, TextStatus::Ok, None, Some("hash"))
        .unwrap();
    store
        .replace_chunks(
            &material,
            &[Chunk {
                material_id: material.clone(),
                ord: 0,
                locator: None,
                text: format!("Stomata open in light. {canary}"),
            }],
        )
        .unwrap();
    id
}

fn store() -> Store {
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
    course(&store, "DEMO101", "readablecanary");
    let withheld = course(&store, "DEMO303", "withheldcanary");
    store
        .set_course_policy(&withheld, AiPolicy::Prohibited, None)
        .unwrap();
    let off = course(&store, "DEMO202", "turnedoffcanary");
    store.set_course_ai_access(&off, false).unwrap();
    store
}

/// The body each preset's wire would send for `prompt_text`'s context.
fn bodies(prompt: &pagelamp_core::ai_gate::RenderedPrompt) -> Vec<(String, String)> {
    [
        ("openai", "https://api.openai.com/v1", "gpt-6-luna"),
        ("anthropic", "https://api.anthropic.com", "claude-sonnet-5"),
        (
            "openrouter",
            "https://openrouter.ai/api/v1",
            "openai/gpt-6-luna",
        ),
        ("ollama", "http://127.0.0.1:11434", "qwen3.5:9b"),
    ]
    .into_iter()
    .map(|(id, base, model)| {
        let profile = preset(id)
            .unwrap()
            .with_base_url(check_base_url(base).unwrap());
        let driver = HttpDriver::new(profile, Some(ApiKey::new("sk-demo-not-a-real-key"))).unwrap();
        let body = driver.request_body(&GenerateRequest {
            model: model.into(),
            prompt: prompt.clone(),
            output: OutputSpec::Text,
            effort: Effort::Lowest,
            max_output_tokens: 1000,
        });
        (id.to_string(), body.to_string())
    })
    .collect()
}

#[test]
fn no_wire_ever_sends_text_of_a_course_that_is_not_readable() {
    let store = store();
    let at = AsOf::now_local();
    let week = week_context(
        &store,
        "DEMO101",
        Some(3),
        at,
        Destination::Cloud,
        ContextBudget { max_chars: 100_000 },
    )
    .unwrap();
    for (wire, body) in bodies(&assemble("Explain the week.", &week, None)) {
        assert!(body.contains("readablecanary"), "{wire}: {body}");
        assert!(
            !body.contains("withheldcanary") && !body.contains("turnedoffcanary"),
            "{wire}"
        );
    }
    let plan = plan_context(&store, &PlanScope::default(), at).unwrap();
    for (wire, body) in bodies(&assemble("Plan the week.", &plan, None)) {
        for canary in ["readablecanary", "withheldcanary", "turnedoffcanary"] {
            assert!(!body.contains(canary), "{wire}: {canary} in {body}");
        }
        // Structure is there: the withheld course's material titles, not its text.
        assert!(body.contains("DEMO303 Demo Studies"), "{wire}");
    }
}
