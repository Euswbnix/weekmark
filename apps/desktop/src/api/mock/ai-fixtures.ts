// Synthetic AI setup data for mock mode (M1). Provider names are the real presets (design §2.5);
// everything else is made up: prices, model lists, versions, usage, keys. Terms links point at
// *.demo.test, like every other mock link.

import type { AiFeature, DisclosureFacts, ModelInfo, ProviderPreset, UsageRow } from "../ai";
import type { MockScenario } from "./fixtures";

const CLOUD = ["structure", "material_text"] as const;

function facts(
  partial: Omit<DisclosureFacts, "sends" | "admin_visibility" | "guardian_permission"> &
    Partial<Pick<DisclosureFacts, "guardian_permission">>,
): DisclosureFacts {
  return {
    sends: [...CLOUD],
    admin_visibility: "no",
    guardian_permission: false,
    ...partial,
  };
}

export const MOCK_PRESETS: ProviderPreset[] = [
  {
    id: "openai",
    label: "OpenAI",
    local: false,
    wire: "openai_responses",
    default_base_url: "https://api.openai.com/v1",
    needs_key: true,
    base_url_editable: false,
    data_policy: facts({
      version: 3101,
      recipient: { name: "OpenAI", terms_url: "https://openai.demo.test/api-terms" },
      training: { kind: "no_training" },
      retention: { kind: "stored_days", days: 30 },
      min_age: 13,
      guardian_permission: true,
      cost: "api_billing",
      on_device: false,
    }),
  },
  {
    id: "anthropic",
    label: "Anthropic",
    local: false,
    wire: "anthropic_messages",
    default_base_url: "https://api.anthropic.com",
    needs_key: true,
    base_url_editable: false,
    data_policy: facts({
      version: 3201,
      recipient: { name: "Anthropic", terms_url: "https://anthropic.demo.test/commercial-terms" },
      training: { kind: "no_training" },
      retention: { kind: "stored_days", days: 30 },
      min_age: 18,
      cost: "api_billing",
      on_device: false,
    }),
  },
  {
    id: "gemini",
    label: "Google Gemini",
    local: false,
    wire: "openai_chat",
    default_base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
    needs_key: true,
    base_url_editable: false,
    data_policy: facts({
      version: 3301,
      recipient: { name: "Google", terms_url: "https://google.demo.test/gemini-api-terms" },
      training: { kind: "may_train_free_tier" },
      retention: { kind: "provider_terms" },
      min_age: 18,
      cost: "api_billing",
      on_device: false,
    }),
  },
  {
    id: "openrouter",
    label: "OpenRouter",
    local: false,
    wire: "openai_chat",
    default_base_url: "https://openrouter.ai/api/v1",
    needs_key: true,
    base_url_editable: false,
    data_policy: facts({
      version: 3401,
      recipient: { name: "OpenRouter", terms_url: "https://openrouter.demo.test/terms" },
      training: { kind: "no_training" },
      retention: { kind: "provider_terms" },
      min_age: null,
      cost: "api_billing",
      on_device: false,
    }),
  },
  {
    id: "ollama",
    label: "Ollama",
    local: true,
    wire: "ollama_native",
    default_base_url: "http://127.0.0.1:11434",
    needs_key: false,
    base_url_editable: true,
    data_policy: facts({
      version: 3501,
      recipient: { name: "Ollama", terms_url: null },
      training: { kind: "no_training" },
      retention: { kind: "on_device" },
      min_age: null,
      cost: "free_on_device",
      on_device: true,
    }),
  },
  {
    id: "lm_studio",
    label: "LM Studio",
    local: true,
    wire: "openai_chat",
    default_base_url: "http://127.0.0.1:1234/v1",
    needs_key: false,
    base_url_editable: true,
    data_policy: facts({
      version: 3601,
      recipient: { name: "LM Studio", terms_url: null },
      training: { kind: "no_training" },
      retention: { kind: "on_device" },
      min_age: null,
      cost: "free_on_device",
      on_device: true,
    }),
  },
  {
    id: "custom",
    label: "Custom (OpenAI-compatible)",
    local: false,
    wire: "openai_chat",
    default_base_url: null,
    needs_key: true,
    base_url_editable: true,
    data_policy: facts({
      version: 3701,
      recipient: { name: "Custom endpoint", terms_url: null },
      training: { kind: "unknown" },
      retention: { kind: "provider_terms" },
      min_age: null,
      cost: "api_billing",
      on_device: false,
    }),
  },
];

/** The facts of an Ollama cloud model: served through the local daemon, not on this computer. */
export function ollamaCloudFacts(base: DisclosureFacts): DisclosureFacts {
  return {
    ...base,
    version: base.version + 50,
    recipient: { name: "Ollama", terms_url: "https://ollama.demo.test/cloud-terms" },
    retention: { kind: "provider_terms" },
    cost: "cloud_via_local",
    on_device: false,
  };
}

/** Price per million tokens, in micro-USD: [input, output]. Unlisted models have no price. */
export const MOCK_PRICES: Record<string, readonly [number, number]> = {
  "gpt-6-luna": [100_000, 400_000],
  "gpt-5.4-mini": [750_000, 4_500_000],
  "gpt-6-astra": [10_000_000, 40_000_000],
  "claude-haiku-4-5": [1_000_000, 5_000_000],
  "claude-sonnet-5": [2_000_000, 10_000_000],
  "claude-opus-5-5": [4_000_000, 20_000_000],
  "gemini-2.5-flash-lite": [100_000, 400_000],
  "gemini-3.8-flash": [750_000, 4_500_000],
  "openai/gpt-6-luna": [100_000, 400_000],
};

const ALL: AiFeature[] = ["study_plan", "weekly_explanation", "weekly_note", "course_calendar"];
const LIGHT: AiFeature[] = ["study_plan", "weekly_note", "course_calendar"];

function model(id: string, extra: Partial<ModelInfo> = {}): ModelInfo {
  return {
    id,
    label: null,
    on_device: false,
    runs_in_cloud: false,
    price_known: id in MOCK_PRICES,
    context_window: 200_000,
    reasoning_always_on: false,
    suggested_for: [],
    ...extra,
  };
}

/** The live model list per preset. */
export const MOCK_MODELS: Record<string, ModelInfo[]> = {
  openai: [
    model("gpt-6-luna", { suggested_for: LIGHT, context_window: 400_000 }),
    model("gpt-5.4-mini", { suggested_for: ["weekly_explanation"], context_window: 400_000 }),
    model("gpt-6-astra", { context_window: 1_000_000 }),
    model("gpt-6-preview-0929", { context_window: 400_000 }),
  ],
  anthropic: [
    model("claude-haiku-4-5", { suggested_for: ALL }),
    model("claude-sonnet-5"),
    model("claude-opus-5-5", { reasoning_always_on: true }),
  ],
  gemini: [
    model("gemini-2.5-flash-lite", { suggested_for: LIGHT, context_window: 1_000_000 }),
    model("gemini-3.8-flash", { suggested_for: ["weekly_explanation"], context_window: 1_000_000 }),
  ],
  openrouter: [model("openai/gpt-6-luna", { suggested_for: ALL })],
  ollama: [
    model("qwen3.5:9b", {
      on_device: true,
      price_known: true,
      context_window: 32_768,
      suggested_for: ALL,
    }),
    model("gemma4:12b", { on_device: true, price_known: true, context_window: 32_768 }),
    model("gpt-oss:120b-cloud", {
      on_device: false,
      runs_in_cloud: true,
      price_known: false,
      context_window: 131_072,
    }),
  ],
  lm_studio: [
    model("qwen3.5-4b", {
      on_device: true,
      price_known: true,
      context_window: 32_768,
      suggested_for: ALL,
    }),
  ],
  custom: [model("demo-model-large", { context_window: 128_000 })],
};

/** Hosts and key prefixes of coding-plan keys (design §2.5), which PageLamp refuses. */
export const CODING_PLAN_HOSTS = [
  "api.z.ai/api/coding",
  "coding-intl.dashscope.aliyuncs.com",
  "api.kimi.ai/coding",
  "api.kimi.com/coding",
];
export const CODING_PLAN_KEY_PREFIXES = ["sk-sp-"];

function row(
  backend_label: string,
  kind: "api_key" | "local",
  model: string,
  feature: AiFeature,
  runs: number,
  micro_usd: number | null,
  estimated = false,
): UsageRow {
  const input = runs * (feature === "weekly_explanation" ? 42_000 : 7_500);
  return {
    backend_label,
    // The facade decides how the cost is known; the mock derives it the same way.
    cost_basis: micro_usd === null ? "unpriced" : kind === "local" ? "free_on_device" : "priced",
    model,
    feature,
    runs,
    input_tokens: input,
    output_tokens: runs * 2_400,
    reasoning_tokens: runs * 600,
    micro_usd,
    estimated,
  };
}

/**
 * The usage ledger per scenario: `[monthsAgo, row]`. This month's API-key rows add up to the
 * budget use the scenario describes (40% of US$5 for ai-key, 99% for ai-budget, so its next explanation goes over).
 */
export function mockUsage(scenario: MockScenario): [number, UsageRow][] {
  switch (scenario) {
    case "ai-key":
      return [
        [0, row("OpenAI", "api_key", "gpt-6-luna", "study_plan", 6, 400_000)],
        [0, row("OpenAI", "api_key", "gpt-5.4-mini", "weekly_explanation", 14, 1_600_000, true)],
        [1, row("OpenAI", "api_key", "gpt-6-luna", "study_plan", 9, 620_000)],
        [1, row("OpenAI", "api_key", "gpt-5.4-mini", "weekly_explanation", 21, 2_480_000)],
      ];
    case "ai-budget":
      return [
        [0, row("OpenAI", "api_key", "gpt-6-luna", "study_plan", 10, 650_000)],
        [0, row("OpenAI", "api_key", "gpt-5.4-mini", "weekly_explanation", 38, 4_310_000)],
      ];
    case "ai-unpriced":
      return [
        [0, row("OpenAI", "api_key", "gpt-6-luna", "study_plan", 4, 260_000)],
        [0, row("OpenAI", "api_key", "gpt-6-preview-0929", "weekly_explanation", 3, null)],
      ];
    case "ai-local":
      return [
        [0, row("Ollama", "local", "qwen3.5:9b", "weekly_explanation", 9, 0)],
        [0, row("Ollama", "local", "qwen3.5:9b", "study_plan", 5, 0)],
        [0, row("Ollama", "local", "gpt-oss:120b-cloud", "weekly_explanation", 2, null)],
      ];
    default:
      return [];
  }
}
