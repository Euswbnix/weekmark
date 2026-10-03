// The AI contract (v0.3 M1; design §3.4, §3.5, §3.8). The shapes come from generated.ts (the
// facade's JSON Schema, `pnpm gen:types`); this file re-exports them with the lists and helpers
// the screens need. A few gaps the backend hasn't closed yet are marked PROVISIONAL.

import type {
  AiBackendStatus,
  AiFeature,
  AiStatus,
  BackendRef,
  BlockReason,
  Course,
  Effort,
  MaterialSharing,
  ModelErrorKind,
  ModelProviderRecord,
} from "./generated";

export type {
  // Mode A: the ChatGPT plan through official Codex (M2; design §2.3).
  AdminVisibility,
  AiBackendStatus,
  AiFeature,
  AiStatus,
  BackendKind,
  BackendProblem,
  BackendRef,
  BackendState,
  BlockReason,
  BudgetStatus,
  ChatGptPlanType,
  CodexLogin,
  CodexLoginMethod,
  CodexLoginState,
  CodexOutdatedAction,
  CodexRuntime,
  CodexRuntimeState,
  CodexSource,
  CodexStatus,
  CostBasis,
  CostEstimate,
  CostKind,
  DisclosureFacts,
  Effort,
  EstimateRequest,
  FeatureRouting,
  GenEvent,
  GenerationMeta,
  GenNoticeCode,
  GenStage,
  LocalServer,
  LocalServerKind,
  LoginEvent,
  MaterialSharing,
  ModeAUsage,
  ModelChoice,
  ModelErrorKind,
  ModelInfo,
  ModelProviderRecord,
  ProbeReport,
  ProviderPreset,
  ProviderWire,
  Recipient,
  RemoveAiDataReport,
  RetentionFact,
  RuntimeEvent,
  SentData,
  StructuredOutputTier,
  SystemCodex,
  TokenUsage,
  TrainingFact,
  UsageRow,
  UsageSummary,
} from "./generated";

// ----- lists in display order (typed against the generated unions) ------------------------------

export const AI_FEATURES: readonly AiFeature[] = [
  "study_plan",
  "weekly_explanation",
  "weekly_note",
  "course_calendar",
];

export const EFFORTS: readonly Effort[] = ["lowest", "low", "medium", "high"];

export const MATERIAL_SHARING: readonly MaterialSharing[] = [
  "unanswered",
  "allowed",
  "not_sure",
  "not_allowed",
];

/** Every BlockReason, as a Record so a code added in Rust fails to compile until listed. */
const BLOCK_REASON_SET: Record<BlockReason, true> = {
  course_policy_prohibited: true,
  course_ai_turned_off: true,
  course_hidden: true,
  no_readable_materials: true,
  material_sharing_not_allowed: true,
  coding_plan_key: true,
  disclosure_not_acknowledged: true,
  no_model_chosen: true,
  budget_reached: true,
  price_unknown_not_acknowledged: true,
  weekly_run_cap_reached: true,
  backend_disabled_in_this_build: true,
};
export const BLOCK_REASONS = Object.keys(BLOCK_REASON_SET) as BlockReason[];

/** Every ModelErrorKind, as a Record for the same reason. */
const MODEL_ERROR_SET: Record<ModelErrorKind, true> = {
  not_signed_in: true,
  auth_rejected: true,
  billing_or_quota: true,
  usage_limit: true,
  rate_limited: true,
  overloaded: true,
  invalid_request: true,
  model_not_found: true,
  context_too_long: true,
  refused: true,
  content_filtered: true,
  network: true,
  timeout: true,
  bad_output: true,
  runtime_missing: true,
  runtime_verify_failed: true,
  runtime_outdated: true,
  unsupported: true,
};
export const MODEL_ERROR_KINDS = Object.keys(MODEL_ERROR_SET) as ModelErrorKind[];

// ----- PROVISIONAL gaps ------------------------------------------------------------------------

/** PROVISIONAL: `material_sharing` joins `ai_policy` on the course types with schema v4. */
export type CourseWithSharing = Course & { material_sharing?: MaterialSharing | null };

export function materialSharing(course: Course): MaterialSharing {
  return (course as CourseWithSharing).material_sharing ?? "unanswered";
}

// ----- helpers ------------------------------------------------------------------------------------

/** Same backend? (BackendRef is a tagged union, so compare by value.) */
export function sameBackend(a: BackendRef, b: BackendRef): boolean {
  return backendKey(a) === backendKey(b);
}

/** A stable string for a backend: a React key, a Map key, a query key part. */
export function backendKey(backend: BackendRef): string {
  return backend.kind === "provider" ? `provider:${backend.provider_id}` : backend.kind;
}

/** The provider record behind an API-key or local backend (`ai_status` lists them apart). */
export function providerOf(
  status: Pick<AiStatus, "providers">,
  backend: Pick<AiBackendStatus, "backend">,
): ModelProviderRecord | null {
  const ref = backend.backend;
  if (ref.kind !== "provider") return null;
  return status.providers.find((p) => p.provider_id === ref.provider_id) ?? null;
}
