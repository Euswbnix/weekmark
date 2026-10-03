//! User-facing notices printed by the CLI (edit the wording here; the product and command
//! names come from `pagelamp_core::brand`).

use pagelamp_app::ai::{AdminVisibility, CostKind, DisclosureFacts, RetentionFact, TrainingFact};
use pagelamp_core::ai::{BlockReason, ModelErrorKind};
use pagelamp_core::brand::{CLI_NAME, PRODUCT_NAME};

/// docs/ARCHITECTURE.md §3 rule 8: shown by `mcp-config` and on the first `… add`.
pub fn ai_disclosure() -> String {
    format!(
        "When you ask your AI app about a course, it reads that course's materials from \
         {PRODUCT_NAME} and sends them to your AI provider under your own account. \
         {PRODUCT_NAME} itself stores nothing remotely. Follow each course's AI policy; you can \
         turn sharing off per course (`{CLI_NAME} course ai-access <course> off`)."
    )
}

/// docs/ARCHITECTURE.md §3 rule 2: shown by `canvas add`.
pub fn canvas_personal_use() -> String {
    format!(
        "Canvas access tokens are for your own personal use only: never generate a token for \
         someone else's app, and don't share yours. Tokens expire — Canvas shows the maximum \
         when you create one (often 30–90 days). When sync reports that the token was \
         rejected, create a new one and run `{CLI_NAME} sources update-secret <source id>`. A \
         course folder plus the calendar feed works without any token."
    )
}

/// Where students create a token (shown right before the token prompt).
pub const CANVAS_TOKEN_HOWTO: &str = "Canvas: Account → Settings → Approved Integrations → + New \
    Access Token (fill in a purpose and an expiry; copy it right away).";

/// Shown before downloading Canvas files (leader research, 2026-09-25).
pub const CANVAS_DOWNLOAD_NOTICE: &str = "Downloading files through Canvas can count as viewing \
    them (e.g. module 'must view' requirements).";

/// Printed after the snippets (each client's notes say how to load it, e.g. quit Claude
/// Desktop first, so no generic "restart" line here).
pub fn try_prompt() -> String {
    format!("Try: \"Using {PRODUCT_NAME}, where is each of my courses this week?\"")
}

// ----- models PageLamp calls itself (`ai …`) ----------------------------------------------------

/// What a backend is sent and what happens to it, before its first use (design §3.8): the
/// facts from the facade plus the fixed limitations-and-risks and ownership paragraphs.
pub fn disclosure(facts: &DisclosureFacts) -> String {
    let name = &facts.recipient.name;
    let mut lines = vec![format!("What {PRODUCT_NAME} sends to {name}:")];
    lines.push(
        "  • Your courses' titles, dates, week numbers and deadlines; for explanations and course \
         calendars, also the text of course materials."
            .to_string(),
    );
    lines.push(if facts.on_device {
        format!("  • {name} runs on this computer: nothing leaves it.")
    } else {
        let terms = facts
            .recipient
            .terms_url
            .as_deref()
            .map(|url| format!(" ({url})"))
            .unwrap_or_default();
        format!("  • It goes to {name} under your account and {name}'s terms{terms}.")
    });
    lines.push(match &facts.training {
        TrainingFact::NoTraining => format!("  • Training: {name} says it doesn't train on it."),
        TrainingFact::MayTrain {
            how_to_turn_off_url,
        } => format!(
            "  • Training: {name} may train its models on it{}.",
            how_to_turn_off_url
                .as_deref()
                .map(|url| format!(" (to turn that off: {url})"))
                .unwrap_or_default()
        ),
        TrainingFact::MayTrainFreeTier => format!(
            "  • Training: on a free tier, {name} may train on it and people may review it."
        ),
        TrainingFact::Unknown => format!("  • Training: unknown; check {name}'s terms."),
    });
    lines.push(match facts.retention {
        RetentionFact::NotStored => "  • Storage: not kept after the answer.".to_string(),
        RetentionFact::StoredDays { days } => format!("  • Storage: kept up to {days} days."),
        RetentionFact::ProviderTerms => format!("  • Storage: as {name}'s terms say."),
        RetentionFact::OnDevice => "  • Storage: stays on this computer.".to_string(),
    });
    match facts.admin_visibility {
        AdminVisibility::Yes => lines.push(
            "  • Your account is a school or workspace account: its administrators can see what \
             is sent, including text from your course materials."
                .to_string(),
        ),
        AdminVisibility::Unknown => lines.push(
            "  • If your account is an Edu or workspace account, its administrators may be able \
             to see what is sent, including text from your course materials."
                .to_string(),
        ),
        AdminVisibility::No => {}
    }
    if let Some(age) = facts.min_age {
        lines.push(format!(
            "  • Age: you must be at least {age}{}.",
            if facts.guardian_permission {
                ", and under 18 you need a parent's or guardian's permission"
            } else {
                ""
            }
        ));
    }
    lines.push(match facts.cost {
        CostKind::ApiBilling => format!(
            "  • Cost: billed to your API account. {PRODUCT_NAME} estimates each run first and \
                 stops runs that could go over your monthly budget (`{CLI_NAME} ai budget`)."
        ),
        CostKind::PlanCredits => "  • Cost: counts against your plan's usage limits.".into(),
        CostKind::FreeOnDevice => "  • Cost: free; it runs on this computer.".into(),
        CostKind::CloudViaLocal => {
            "  • It is served by an app on this computer but runs in the cloud.".into()
        }
        CostKind::SelfHosted => "  • Costs, if any, are set by whoever runs this server.".into(),
    });
    if let Some(location) = &facts.location {
        lines.push(format!("  • Processed in: {location}."));
    }
    lines.push(String::new());
    lines.push(
        "Generative AI can be wrong: check what it says against your course materials. It \
         doesn't replace your instructors' rules for their courses."
            .to_string(),
    );
    lines.push(
        "Course materials, and anything made from them, remain your institution's and your \
         instructors'."
            .to_string(),
    );
    lines.join("\n")
}

/// Before the first use of a model without a known price (`unchecked`: the model list wasn't
/// asked, so it may well have one).
pub fn unpriced_model(unchecked: bool) -> String {
    format!(
        "{} {PRODUCT_NAME} can't estimate its cost, so your monthly budget can't stop its runs; \
         check the usage on the provider's own site.",
        if unchecked {
            format!("If {PRODUCT_NAME} doesn't know this model's price,")
        } else {
            format!("{PRODUCT_NAME} doesn't know this model's price:")
        }
    )
}

pub const REMOVE_ALL_AI_DATA: &str = "Remove all AI data: providers and their keys, model \
    choices, acknowledgements, usage records, generated texts and the pre-update database backup?";

/// Why a run wouldn't start (`BlockReason`), with the command that changes it.
pub fn block_reason(reason: BlockReason) -> String {
    match reason {
        BlockReason::CoursePolicyProhibited => {
            "the course's AI policy prohibits AI use.".to_string()
        }
        BlockReason::CourseAiTurnedOff => {
            format!("AI access is off for this course (`{CLI_NAME} course ai-access <course> on`).")
        }
        BlockReason::CourseHidden => {
            format!("the course is hidden (`{CLI_NAME} course show <course>`).")
        }
        BlockReason::NoReadableMaterials => {
            "there are no readable materials for that week.".to_string()
        }
        BlockReason::MaterialSharingNotAllowed => format!(
            "you answered that this course's materials may not be shared with an AI service, and \
             this model runs in the cloud (`{CLI_NAME} course sharing <course> …`, or choose a \
             model on this computer)."
        ),
        BlockReason::CodingPlanKey => {
            "coding-plan keys may only be used in the vendor's own coding tools.".to_string()
        }
        BlockReason::DisclosureNotAcknowledged => {
            format!("you haven't accepted what this provider is sent yet (`{CLI_NAME} ai use …`).")
        }
        BlockReason::NoModelChosen => format!(
            "no model is chosen for this feature (`{CLI_NAME} ai use <feature> <provider> <model>`)."
        ),
        BlockReason::BudgetReached => {
            format!("it could go over this month's budget (`{CLI_NAME} ai budget`).")
        }
        BlockReason::PriceUnknownNotAcknowledged => format!(
            "{PRODUCT_NAME} doesn't know this model's price and you haven't accepted that \
             (`{CLI_NAME} ai use …`)."
        ),
        BlockReason::WeeklyRunCapReached => "this week's run limit is reached.".to_string(),
        BlockReason::BackendDisabledInThisBuild => {
            "this way of using a model isn't available in this build.".to_string()
        }
    }
}

/// A model error (`ModelErrorKind`) in a few words.
pub fn model_error(kind: ModelErrorKind) -> &'static str {
    match kind {
        ModelErrorKind::NotSignedIn => "not signed in",
        ModelErrorKind::AuthRejected => "the key was rejected",
        ModelErrorKind::BillingOrQuota => "out of credit or over a spending limit",
        ModelErrorKind::UsageLimit => "the plan's usage limit is reached",
        ModelErrorKind::RateLimited => "too many requests; try again shortly",
        ModelErrorKind::Overloaded => "the service is overloaded or failing",
        ModelErrorKind::InvalidRequest => "the service rejected the request",
        ModelErrorKind::ModelNotFound => "the provider doesn't have that model",
        ModelErrorKind::ContextTooLong => "too much text for the model",
        ModelErrorKind::Refused => "the model declined to answer",
        ModelErrorKind::ContentFiltered => "the provider's content filter stopped the answer",
        ModelErrorKind::Network => "couldn't reach the provider",
        ModelErrorKind::Timeout => "the provider took too long",
        ModelErrorKind::BadOutput => "the answer couldn't be used",
        ModelErrorKind::RuntimeMissing => "the local runtime isn't installed",
        ModelErrorKind::RuntimeVerifyFailed => "the downloaded runtime failed its check",
        ModelErrorKind::RuntimeOutdated => "the local runtime needs an update",
        ModelErrorKind::Unsupported => "the model can't do this",
    }
}

/// After `course sharing … not-allowed`.
pub fn sharing_not_allowed_note() -> String {
    format!(
        "{PRODUCT_NAME} won't send this course's material text to cloud models it runs; models \
         on this computer and your own AI app (over MCP) are not affected."
    )
}

/// The one-time question (b) reminder, after a course's first cloud reading (D37 option 2).
pub fn sharing_reminder_note() -> String {
    format!(
        "This course's material text was just sent to a cloud AI service. Check whether your \
         instructor allows that, then record it: `{} course sharing <course> allowed | \
         not-sure | not-allowed`.",
        pagelamp_core::brand::CLI_NAME
    )
}
