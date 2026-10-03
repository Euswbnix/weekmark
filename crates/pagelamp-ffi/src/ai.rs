//! AI in the facade (v0.3 M1–M3, beta.2): model setup and the disclosure, costs, the ChatGPT
//! plan through Codex, weekly explanations, the weekly note, study plans and deleting what was
//! generated. Every generation takes the caller's `generation_id`; `cancel_generation(id)`
//! stops it.

use std::sync::Arc;

use pagelamp_app::ai::{
    AiStatus, BackendRef, CodexLoginMethod, CodexSource, CodexStatus, CostEstimate,
    EstimateRequest, ExplainOptions, GeneratedStudyPlan, LocalServer, ModelChoice, ModelInfo,
    ModelProviderRecord, OutputLanguage, ProbeReport, ProviderPreset, RemoveAiDataReport,
    StudyPlanRequest, UsageSummary, WeeklyExplanation, WeeklyNote, WeeklyNoteOptions,
    WeeklyNoteSettings,
};
use pagelamp_core::ai::{AiFeature, MaterialSharing};
use pagelamp_core::model::StoredStudyPlan;

use crate::observers::{
    CodexInstallObserver, CodexLoginObserver, GenObserver, gen_events, install_events, login_events,
};
use crate::{IsoDate, PageLamp, Result, blocking, spawned};

#[uniffi::export]
impl PageLamp {
    // ----- model setup -------------------------------------------------------------------------

    /// The providers the student can add (OpenAI, Anthropic, Ollama, …), with their facts.
    pub async fn model_provider_presets(&self) -> Result<Vec<ProviderPreset>> {
        let app = self.app.clone();
        blocking(move || Ok(app.model_provider_presets())).await
    }

    /// Model servers running on this computer (Ollama, LM Studio, …).
    pub async fn detect_local_servers(&self) -> Result<Vec<LocalServer>> {
        let app = self.app.clone();
        spawned(async move { app.detect_local_servers().await }).await
    }

    /// Every backend, its state and disclosure, the feature routing and the budget.
    pub async fn ai_status(&self) -> Result<AiStatus> {
        let app = self.app.clone();
        blocking(move || app.ai_status()).await
    }

    /// Adds a provider from a preset; the key goes to the secret store, never the database.
    pub async fn add_model_provider(
        &self,
        preset: String,
        base_url: Option<String>,
        api_key: Option<String>,
    ) -> Result<ModelProviderRecord> {
        let app = self.app.clone();
        spawned(async move {
            app.add_model_provider(&preset, base_url.as_deref(), api_key.as_deref())
                .await
        })
        .await
    }

    pub async fn update_model_provider_key(
        &self,
        provider_id: String,
        api_key: String,
    ) -> Result<ModelProviderRecord> {
        let app = self.app.clone();
        spawned(async move { app.update_model_provider_key(&provider_id, &api_key).await }).await
    }

    pub async fn remove_model_provider(&self, provider_id: String) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.remove_model_provider(&provider_id)).await
    }

    pub async fn list_models(&self, backend: BackendRef) -> Result<Vec<ModelInfo>> {
        let app = self.app.clone();
        spawned(async move { app.list_models(&backend).await }).await
    }

    /// A tiny request to the model: does it answer, and in structured output?
    pub async fn test_model(&self, backend: BackendRef, model: String) -> Result<ProbeReport> {
        let app = self.app.clone();
        spawned(async move { app.test_model(&backend, &model).await }).await
    }

    /// The model a feature uses (nil: none).
    pub async fn set_feature_model(
        &self,
        feature: AiFeature,
        choice: Option<ModelChoice>,
    ) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.set_feature_model(feature, choice)).await
    }

    /// The student read what `backend` receives (`version`: the disclosure they saw).
    pub async fn acknowledge_ai_disclosure(&self, backend: BackendRef, version: u32) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.acknowledge_ai_disclosure(&backend, version)).await
    }

    /// The student accepts that `model` has no known price.
    pub async fn acknowledge_unpriced_model(
        &self,
        backend: BackendRef,
        model: String,
    ) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.acknowledge_unpriced_model(&backend, &model)).await
    }

    /// The monthly budget in millionths of a US dollar (nil: none).
    pub async fn set_monthly_budget(&self, micro_usd: Option<u64>) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.set_monthly_budget(micro_usd)).await
    }

    pub async fn estimate_generation(&self, request: EstimateRequest) -> Result<CostEstimate> {
        let app = self.app.clone();
        blocking(move || app.estimate_generation(&request)).await
    }

    /// Use and cost in `month` (any day of it; nil: this month).
    pub async fn usage_summary(&self, month: Option<IsoDate>) -> Result<UsageSummary> {
        let app = self.app.clone();
        blocking(move || app.usage_summary(month)).await
    }

    /// Question (b): may a cloud model read this course's materials?
    pub async fn set_course_material_sharing(
        &self,
        course: String,
        answer: MaterialSharing,
    ) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.set_course_material_sharing(&course, answer)).await
    }

    /// Deletes generated explanations and plan drafts (one course, or all); returns how many.
    pub async fn delete_generated(&self, course: Option<String>) -> Result<u32> {
        let app = self.app.clone();
        blocking(move || app.delete_generated(course.as_deref())).await
    }

    /// Everything AI: providers and their keys, choices, usage, generations.
    pub async fn remove_all_ai_data(&self) -> Result<RemoveAiDataReport> {
        let app = self.app.clone();
        blocking(move || app.remove_all_ai_data()).await
    }

    // ----- the ChatGPT plan through Codex ---------------------------------------------------------

    pub async fn codex_status(&self) -> Result<CodexStatus> {
        let app = self.app.clone();
        spawned(async move { app.codex_status().await }).await
    }

    /// Downloads, verifies and installs the pinned Codex; `cancel_codex_install(install_id)`
    /// stops it.
    pub async fn install_codex(
        &self,
        install_id: String,
        observer: Arc<dyn CodexInstallObserver>,
    ) -> Result<CodexStatus> {
        let app = self.app.clone();
        let events = install_events(observer);
        let sink = events.sink();
        let result = spawned(async move { app.install_codex(&install_id, sink).await }).await;
        events.drain().await;
        result
    }

    pub async fn cancel_codex_install(&self, install_id: String) -> Result<()> {
        self.app.cancel_codex_install(&install_id);
        Ok(())
    }

    /// Removes the Codex PageLamp installed (never one the student installed themselves).
    pub async fn remove_codex(&self) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.remove_codex()).await
    }

    /// Signs in through Codex (browser or one-time code); `cancel_codex_login` stops it.
    pub async fn codex_login(
        &self,
        method: CodexLoginMethod,
        observer: Arc<dyn CodexLoginObserver>,
    ) -> Result<CodexStatus> {
        let app = self.app.clone();
        let events = login_events(observer);
        let sink = events.sink();
        let result = spawned(async move { app.codex_login(method, sink).await }).await;
        events.drain().await;
        result
    }

    pub async fn cancel_codex_login(&self) -> Result<()> {
        self.app.cancel_codex_login();
        Ok(())
    }

    pub async fn codex_logout(&self) -> Result<CodexStatus> {
        let app = self.app.clone();
        spawned(async move { app.codex_logout().await }).await
    }

    /// Runs per week on the ChatGPT plan (nil: the default cap).
    pub async fn set_mode_a_weekly_cap(&self, runs: Option<u32>) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.set_mode_a_weekly_cap(runs)).await
    }

    /// Which Codex to use: the one PageLamp installs, or the student's own.
    pub async fn set_codex_source(&self, source: CodexSource) -> Result<CodexStatus> {
        let app = self.app.clone();
        spawned(async move { app.set_codex_source(source).await }).await
    }

    // ----- weekly explanations ---------------------------------------------------------------------

    /// Explains a week of `course` (nil: the default week) from its readable materials, with
    /// citations; `cancel_generation(generation_id)` stops it.
    pub async fn explain_week(
        &self,
        course: String,
        week: Option<u32>,
        generation_id: String,
        options: ExplainOptions,
        observer: Arc<dyn GenObserver>,
    ) -> Result<WeeklyExplanation> {
        let app = self.app.clone();
        let events = gen_events(observer);
        let sink = events.sink();
        let result = spawned(async move {
            app.explain_week(&course, week, &generation_id, options, sink)
                .await
        })
        .await;
        events.drain().await;
        result
    }

    /// The saved explanations of `course` (one week, or all), newest first.
    pub async fn saved_explanations(
        &self,
        course: String,
        week: Option<u32>,
    ) -> Result<Vec<WeeklyExplanation>> {
        let app = self.app.clone();
        blocking(move || app.saved_explanations(&course, week)).await
    }

    pub async fn delete_explanation(&self, generation_id: String) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.delete_explanation(&generation_id)).await
    }

    /// Explanations in the app's language or the course's.
    pub async fn ai_output_language(&self) -> Result<OutputLanguage> {
        let app = self.app.clone();
        blocking(move || app.ai_output_language()).await
    }

    pub async fn set_ai_output_language(&self, language: OutputLanguage) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.set_ai_output_language(language)).await
    }

    // ----- the weekly note (beta.2) ----------------------------------------------------------------

    /// Writes this week's note from the courses' structure and the plan's progress (never
    /// material text); `cancel_generation(generation_id)` stops it. `options.automatic` only
    /// when `startup_tasks().prepare_weekly_note` said so.
    pub async fn write_weekly_note(
        &self,
        generation_id: String,
        options: WeeklyNoteOptions,
        observer: Arc<dyn GenObserver>,
    ) -> Result<WeeklyNote> {
        let app = self.app.clone();
        let events = gen_events(observer);
        let sink = events.sink();
        let result =
            spawned(async move { app.write_weekly_note(&generation_id, options, sink).await })
                .await;
        events.drain().await;
        result
    }

    /// The kept weekly notes, newest first.
    pub async fn weekly_notes(&self) -> Result<Vec<WeeklyNote>> {
        let app = self.app.clone();
        blocking(move || app.weekly_notes()).await
    }

    pub async fn delete_weekly_note(&self, generation_id: String) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.delete_weekly_note(&generation_id)).await
    }

    /// "Prepare it when I open PageLamp on Monday", and whether the note's model allows it.
    pub async fn weekly_note_settings(&self) -> Result<WeeklyNoteSettings> {
        let app = self.app.clone();
        blocking(move || app.weekly_note_settings()).await
    }

    /// Turns "prepare it on Monday" on (an API key or a model on this computer only) or off.
    pub async fn set_prepare_weekly_note_on_monday(&self, on: bool) -> Result<WeeklyNoteSettings> {
        let app = self.app.clone();
        blocking(move || app.set_prepare_weekly_note_on_monday(on)).await
    }

    // ----- study plans -----------------------------------------------------------------------------

    /// Drafts a plan (not saved until `accept_study_plan`); `cancel_generation` stops it.
    pub async fn generate_study_plan(
        &self,
        request: StudyPlanRequest,
        generation_id: String,
        observer: Arc<dyn GenObserver>,
    ) -> Result<GeneratedStudyPlan> {
        let app = self.app.clone();
        let events = gen_events(observer);
        let sink = events.sink();
        let result =
            spawned(async move { app.generate_study_plan(request, &generation_id, sink).await })
                .await;
        events.drain().await;
        result
    }

    /// Saves the draft `generation_id` as the current study plan.
    pub async fn accept_study_plan(&self, generation_id: String) -> Result<StoredStudyPlan> {
        let app = self.app.clone();
        blocking(move || app.accept_study_plan(&generation_id)).await
    }

    pub async fn set_study_plan_item_done(
        &self,
        plan_id: i64,
        item_index: u32,
        done: bool,
    ) -> Result<StoredStudyPlan> {
        let app = self.app.clone();
        blocking(move || app.set_study_plan_item_done(plan_id, item_index, done)).await
    }

    /// Stops a running generation or batch by the id the caller gave it; it ends with
    /// `Cancelled`. Unknown or finished ids are fine.
    pub async fn cancel_generation(&self, generation_id: String) -> Result<()> {
        let app = self.app.clone();
        blocking(move || app.cancel_generation(&generation_id)).await
    }
}
