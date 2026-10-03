// Mode A in the mock (M2): the ChatGPT plan through official Codex. It mirrors what the facade
// decides — runtime state, what a RuntimeOutdated error means now (`outdated_action`), sign-in,
// the weekly run cap, the disclosure facts per plan type — so the ChatGPT card can be built before
// the Rust side. Everything here is synthetic; links point at *.demo.test.
//
// PROVISIONAL until the owner's A7 tests (by 2026-10-18): what `codex login status` says about the
// plan type, whether `codex exec` works on Free/Go (exec_available), and the credits wording.

import type {
  AiBackendStatus,
  ChatGptPlanType,
  CodexStatus,
  DisclosureFacts,
  ModeAUsage,
  ModelInfo,
  RuntimeEvent,
  UsageRow,
} from "../ai";
import type { PageLampApi } from "../client";
import { ApiError } from "../errors";
import type { MockActivity } from "./activity";
import type { MockScenario } from "./fixtures";

type CodexApi = Pick<
  PageLampApi,
  | "codexStatus"
  | "installCodex"
  | "cancelCodexInstall"
  | "removeCodex"
  | "codexLogin"
  | "cancelCodexLogin"
  | "codexLogout"
  | "setCodexSource"
  | "setModeAWeeklyCap"
>;

export const MOCK_CODEX_PIN = "0.158.0";
const OLDER_CODEX = "0.157.1";
/** ≈71 MB compressed (design §2.3: ≈65–75 MB). */
export const MOCK_CODEX_DOWNLOAD_BYTES = 71_300_000;
/** The proposed default weekly cap (design §2.3). */
export const DEFAULT_WEEKLY_CAP = 40;
export const MOCK_DEVICE_CODE = "PLMP-4821";
export const CODEX_LABEL = "ChatGPT plan (through OpenAI Codex)";

/** The pin's supported models (codex-pin.toml): explicit `-m` from this list on every run. */
export const MOCK_CODEX_MODELS: ModelInfo[] = [
  {
    id: "gpt-6-luna",
    on_device: false,
    runs_in_cloud: false,
    price_known: false,
    context_window: 400_000,
    reasoning_always_on: false,
    suggested_for: ["study_plan", "weekly_note", "course_calendar"],
  },
  {
    id: "gpt-6-sol",
    on_device: false,
    runs_in_cloud: false,
    price_known: false,
    context_window: 400_000,
    reasoning_always_on: false,
    suggested_for: ["weekly_explanation"],
  },
];

/** The disclosure facts for mode A depend on the plan (design §2.3). */
export function codexFacts(plan: ChatGptPlanType): DisclosureFacts {
  const workspace = plan === "business" || plan === "edu" || plan === "enterprise";
  const PLANS: ChatGptPlanType[] = ["free", "go", "plus", "pro", "business", "edu", "enterprise"];
  return {
    version: 4100 + PLANS.indexOf(plan) + 1,
    sends: ["structure", "material_text"],
    recipient: { name: "OpenAI", terms_url: "https://openai.demo.test/terms" },
    training: workspace
      ? { kind: "no_training" }
      : {
          kind: "may_train",
          how_to_turn_off_url: "https://chatgpt.demo.test/settings/data-controls",
        },
    retention: { kind: "provider_terms" },
    admin_visibility:
      plan === "edu" || plan === "enterprise" || plan === "business"
        ? "yes"
        : plan === "unknown"
          ? "unknown"
          : "no",
    min_age: 13,
    guardian_permission: true,
    cost: "plan_credits",
    on_device: false,
  };
}

interface Scenario {
  installed: string | null;
  login: CodexStatus["login"];
  outdated: CodexStatus["outdated_action"];
  execAvailable: boolean | null;
  runs: number;
  acknowledged: boolean;
}

function scenarioState(scenario: MockScenario): Scenario {
  const plus = { state: "chatgpt", plan_type: "plus" } as const;
  const base: Scenario = {
    installed: MOCK_CODEX_PIN,
    login: plus,
    outdated: "none",
    execAvailable: true,
    runs: 12,
    acknowledged: true,
  };
  switch (scenario) {
    case "codex-signed-out":
      return { ...base, login: { state: "signed_out" }, runs: 0, acknowledged: false };
    case "codex-plus":
      return base;
    case "codex-edu":
      return { ...base, login: { state: "chatgpt", plan_type: "edu" }, acknowledged: false };
    case "codex-api-key":
      return { ...base, login: { state: "api_key", plan_type: null } };
    case "codex-outdated-pin":
      return { ...base, installed: OLDER_CODEX, outdated: "install_pin" };
    case "codex-outdated-app":
      return { ...base, outdated: "update_pagelamp" };
    case "codex-free":
      return {
        ...base,
        login: { state: "chatgpt", plan_type: "free" },
        execAvailable: false,
        runs: 0,
        acknowledged: false,
      };
    case "codex-cap":
      return { ...base, runs: DEFAULT_WEEKLY_CAP };
    default:
      return {
        installed: null,
        login: { state: "signed_out" },
        outdated: "none",
        execAvailable: null,
        runs: 0,
        acknowledged: false,
      };
  }
}

/**
 * Whether the mock offers the ChatGPT plan: only in the `codex-*` scenarios, which exist to show
 * its screens. Everywhere else it's off, as in every build until OpenAI confirms in writing (the
 * facade's CHATGPT_PLAN_OFFERED): no card, no ChatGPT copy, and every way into Codex refuses.
 */
export function chatgptPlanOfferedIn(scenario: MockScenario): boolean {
  return scenario.startsWith("codex-");
}

/** The facade's refusal while the ChatGPT plan isn't offered. */
export function chatgptPlanNotOffered(): ApiError {
  return new ApiError(
    "blocked",
    "The ChatGPT plan isn't available in this version of PageLamp. Use an API key or a model on this computer.",
    { blocked: "backend_disabled_in_this_build" },
  );
}

export interface MockCodex {
  api: CodexApi;
  /** This build offers the ChatGPT plan (`chatgptPlanOfferedIn`). */
  offered: boolean;
  /** Throws the facade's refusal unless the plan is offered. */
  requireOffered: () => void;
  /** The `codex` entry of ai_status, once Codex is installed or chosen for a feature. */
  backendStatus: (acknowledged: number | null, chosen: boolean) => AiBackendStatus | null;
  /** Whether the scenario starts with the disclosure acknowledged. */
  initiallyAcknowledged: boolean;
  models: () => ModelInfo[];
  /** For the estimate: would a run be refused by the weekly cap? */
  capReached: () => boolean;
  /** Null until Codex is installed or chosen for a feature, like the facade. */
  modeA: (chosen: boolean) => ModeAUsage | null;
  usageRows: () => [number, UsageRow][];
  signedIn: () => boolean;
  /** "Remove all AI data" runs `codex logout` first. */
  forget: () => void;
}

export function createMockCodex(options: {
  scenario: MockScenario;
  delay: (extra?: number) => Promise<void>;
  stepMs: number;
  activity: MockActivity;
}): MockCodex {
  const { delay, stepMs } = options;
  const start = scenarioState(options.scenario);
  const offered = chatgptPlanOfferedIn(options.scenario);
  const requireOffered = () => {
    if (!offered) throw chatgptPlanNotOffered();
  };
  const state = {
    installed: start.installed,
    source: "managed" as CodexStatus["runtime"]["source"],
    login: { ...start.login } as CodexStatus["login"],
    outdated: start.outdated,
    execAvailable: start.execAvailable,
    cap: DEFAULT_WEEKLY_CAP as number | null,
    runs: start.runs,
  };
  const cancelledInstalls = new Set<string>();
  let loginCancelled = false;
  const sleep = (ms: number) =>
    ms > 0 ? new Promise<void>((resolve) => setTimeout(resolve, ms)) : Promise.resolve();

  function status(): CodexStatus {
    return structuredClone({
      // As `aiStatus` (mock/ai.ts). The rest still answers, for clean-up.
      chatgpt_plan_offered: offered,
      runtime: {
        state: state.installed ? "installed" : "not_installed",
        source: state.source,
        installed_version: state.installed,
        pinned_version: MOCK_CODEX_PIN,
        download_bytes: MOCK_CODEX_DOWNLOAD_BYTES,
        untested_platform: false,
      },
      outdated_action: state.outdated,
      login: state.login,
      exec_available: state.execAvailable,
      weekly_cap: state.cap,
      runs_this_week: state.runs,
      system_codex: { version: "0.156.0", in_tested_range: false },
    } satisfies CodexStatus);
  }

  function requireInstalled() {
    if (!state.installed) {
      throw new ApiError("model", "Codex isn't installed.", { model_error: "runtime_missing" });
    }
  }

  /** Download, verify and install the pinned runtime; stops at the next step when cancelled. */
  async function install(
    installId: string,
    onEvent: (event: RuntimeEvent) => void,
  ): Promise<CodexStatus> {
    await delay();
    const total = MOCK_CODEX_DOWNLOAD_BYTES;
    const check = () => {
      if (cancelledInstalls.has(installId)) {
        cancelledInstalls.delete(installId);
        // The partial download is deleted, never resumed.
        throw new ApiError("cancelled", "Download cancelled.");
      }
    };
    onEvent({ type: "download_started", total_bytes: total });
    for (const part of [0.2, 0.45, 0.7, 1]) {
      await sleep(stepMs);
      check();
      onEvent({
        type: "progress",
        downloaded_bytes: Math.round(total * part),
        total_bytes: total,
      });
    }
    await sleep(stepMs);
    check();
    onEvent({ type: "verifying" });
    await sleep(stepMs);
    onEvent({ type: "installing" });
    await sleep(stepMs);
    state.installed = MOCK_CODEX_PIN;
    state.source = "managed";
    if (state.outdated === "install_pin") state.outdated = "none";
    onEvent({ type: "done", version: MOCK_CODEX_PIN });
    return status();
  }

  const api: CodexApi = {
    codexStatus: async () => {
      await delay();
      return status();
    },

    installCodex: async (installId, onEvent) => {
      // Refused before the activity starts, like the facade.
      requireOffered();
      return options.activity.during("codex_install", {}, () => install(installId, onEvent));
    },

    cancelCodexInstall: async (installId) => {
      cancelledInstalls.add(installId);
    },

    removeCodex: async () => {
      await delay(300);
      state.installed = null;
      state.login = { state: "signed_out" };
      state.outdated = "none";
    },

    codexLogin: async (method, onEvent) => {
      await delay();
      requireOffered();
      requireInstalled();
      loginCancelled = false;
      if (method === "device_code") {
        onEvent({
          type: "device_code",
          verification_url: "https://auth.demo.test/codex/device",
          user_code: MOCK_DEVICE_CODE,
          expires_in_secs: 900,
        });
      } else {
        onEvent({ type: "browser_opened", url: "https://auth.demo.test/oauth/authorize" });
      }
      onEvent({ type: "waiting" });
      for (let i = 0; i < 4; i++) {
        await sleep(stepMs);
        if (loginCancelled) throw new ApiError("cancelled", "Sign-in cancelled.");
      }
      // Like the facade: `codex login status` doesn't say the plan type yet (until A7), so a
      // fresh sign-in reports "unknown". Scenarios with a plan type preview what A7 may allow.
      state.login = { state: "chatgpt", plan_type: start.login.plan_type ?? "unknown" };
      onEvent({ type: "done" });
      return status();
    },

    cancelCodexLogin: async () => {
      loginCancelled = true;
    },

    codexLogout: async () => {
      await delay();
      state.login = { state: "signed_out" };
      return status();
    },

    setCodexSource: async (source) => {
      await delay();
      requireOffered();
      if (source === "system") {
        throw new ApiError(
          "invalid",
          "The installed Codex (0.156.0) is outside the tested range; PageLamp keeps its own.",
        );
      }
      state.source = source;
      return status();
    },

    setModeAWeeklyCap: async (runs) => {
      await delay();
      requireOffered();
      if (runs !== null && (!Number.isInteger(runs) || runs < 1)) {
        throw new ApiError("invalid", "The weekly cap must be a whole number of runs, at least 1.");
      }
      state.cap = runs;
    },
  };

  return {
    api,
    offered,
    requireOffered,
    initiallyAcknowledged: start.acknowledged,
    backendStatus: (acknowledged, chosen) => {
      // Not offered: no Codex backend at all, like the facade.
      if (!offered || (!state.installed && !chosen)) return null;
      const disclosure = codexFacts(state.login.plan_type ?? "unknown");
      const problems: AiBackendStatus["problems"] = [];
      if (!state.installed) problems.push("runtime_missing");
      else if (state.login.state === "signed_out") problems.push("not_signed_in");
      if (acknowledged !== null && acknowledged !== disclosure.version) {
        problems.push("disclosure_changed");
      }
      const setUp =
        state.installed && state.login.state !== "signed_out" && state.execAvailable !== false;
      return {
        backend: { kind: "codex" },
        label: CODEX_LABEL,
        kind: "codex",
        state: !setUp
          ? "needs_setup"
          : acknowledged !== disclosure.version
            ? "needs_disclosure"
            : "ready",
        problems,
        disclosure,
        disclosure_acknowledged: acknowledged,
      };
    },
    models: () => structuredClone(MOCK_CODEX_MODELS),
    capReached: () => state.cap !== null && state.runs >= state.cap,
    modeA: (chosen) =>
      state.installed || chosen ? { runs_this_week: state.runs, weekly_cap: state.cap } : null,
    usageRows: () =>
      start.runs > 0 && start.login.state !== "signed_out"
        ? [
            [0, codexRow("weekly_explanation", 9)],
            [0, codexRow("study_plan", 3)],
          ]
        : [],
    signedIn: () => state.login.state !== "signed_out",
    forget: () => {
      state.login = { state: "signed_out" };
      state.cap = DEFAULT_WEEKLY_CAP;
    },
  };
}

function codexRow(feature: UsageRow["feature"], runs: number): UsageRow {
  return {
    backend_label: CODEX_LABEL,
    cost_basis: "plan",
    model: feature === "weekly_explanation" ? "gpt-6-sol" : "gpt-6-luna",
    feature,
    runs,
    input_tokens: runs * (feature === "weekly_explanation" ? 41_000 : 7_000),
    output_tokens: runs * 2_600,
    reasoning_tokens: runs * 900,
    micro_usd: null,
    estimated: false,
  };
}
