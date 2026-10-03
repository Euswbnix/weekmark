// IPC contract, TypeScript half. Calls every PageLampApi method of the real Tauri client with
// a mocked IPC and records exactly what would cross the boundary (command + arguments) in
// src-tauri/tests/fixtures/ipc-calls.json. The Rust half (src-tauri/tests/ipc_contract.rs)
// replays that file against the real commands, so a renamed argument or an unregistered
// command fails a test on one side or the other.
//
// After changing tauri.ts on purpose: `pnpm exec vitest run -u src/api/tauri.contract.test.ts`
// and commit the updated fixture.

import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { createTauriApi } from "./tauri";

afterEach(() => clearMocks());

const COURSE = "folder:contract-test/course/DEMO101";
const SOURCE = "folder:contract-test";
// Values the facade rejects before any network access, so the Rust replay stays offline and
// deterministic (the contract is about names and shapes, not about succeeding).
const OFFLINE_CANVAS_URL = "contract-test-not-a-url";
const OFFLINE_FEED_URL = "http://calendar.example.edu/feed.ics"; // plain http → invalid
const OFFLINE_LLM_URL = "http://llm.example.edu/v1"; // plain http to another computer → invalid
const PROVIDER = { kind: "provider", provider_id: "contract-test-provider" } as const; // not_found

it("sends the commands and arguments the Rust side expects", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    // Plugin calls (folder picker, opener) aren't ours; the contract covers our commands.
    if (!cmd.startsWith("plugin:")) calls.push({ cmd, args });
    return null;
  });
  const api = createTauriApi();
  const onEvent = () => {};

  await api.status();
  await api.listSources();
  await api.addCanvasSource(OFFLINE_CANVAS_URL, "contract-test-token");
  await api.addFolderSource("/tmp/contract-test-courses", "2026-09-08", "Courses");
  await api.addFolderSource("/tmp/contract-test-courses");
  await api.addIcalSource(OFFLINE_FEED_URL, null);
  await api.updateSourceSecret(SOURCE, "contract-test-secret");
  await api.removeSource(SOURCE);
  await api.syncAll({}, onEvent);
  await api.syncSource(SOURCE, { download_files: false }, onEvent);
  await api.downloadCourseFiles(COURSE, onEvent);
  await api.cancelSync();
  await api.listCourses();
  await api.courseOverview(COURSE);
  await api.weekMaterials(COURSE, 3);
  await api.weekMaterials(COURSE);
  await api.listDeadlines(null, 7, 0);
  await api.listDeadlines(COURSE, 21, 7);
  await api.search("sampling", null, 20);
  await api.latestStudyPlan();
  await api.setCoursePolicy(COURSE, "learning_aid", "Syllabus §5");
  await api.setCourseTerm(COURSE, "2026-09-08", null);
  await api.setCourseAiAccess(COURSE, false);
  await api.setCourseHidden(COURSE, true);
  await api.setCourseMaterialSharing(COURSE, "not_sure");
  await api.keepCourseCurrent(COURSE, "2026-12-31");
  await api.keepCourseCurrent(COURSE, null);
  await api.setCourseDates(COURSE, {
    first_class: "2026-09-08",
    last_class: "2026-12-04",
    exams_end: null,
    breaks: [],
    second_segment: null,
  });
  await api.setCourseDates(COURSE, null);
  await api.lifecycleSummary();
  await api.snoozeLifecycleBanner();
  await api.snoozeRemovalSuggestions([COURSE], "not_now");
  await api.clearRemovalSnooze([COURSE]);
  // Removal: the course and removed ids are made up, so the facade answers not_found (nothing is
  // removed or moved to the Trash); purging every due removal finds none in the empty data dir.
  await api.removalPreview([COURSE]);
  await api.removeCourses([COURSE], {
    reason: null,
    keep_downloaded_files: false,
    purge_now: false,
    delete_pre_update_backup: false,
  });
  await api.removedCourses();
  await api.restoreCourse("contract-test-removed");
  await api.purgeRemovedCourses(["contract-test-removed"], false);
  await api.purgeRemovedCourses(null, false);
  await api.forgetRemovedCourse("contract-test-removed");
  // Calendar: made-up material and proposal ids, which the facade refuses before any network.
  await api.courseCalendar(COURSE);
  await api.setCalendarSources(COURSE, ["contract-test-material"], []);
  await api.downloadMaterialFiles(COURSE, ["contract-test-material"], onEvent);
  await api.scanCourseCalendar(COURSE);
  await api.acceptCalendarProposal(1, null);
  await api.acceptPassingProposals([1]);
  await api.dismissCalendarProposal(1);
  await api.syllabusReadingOffers();
  await api.readCourseCalendar(
    COURSE,
    "contract-test-generation",
    { override_budget: false },
    () => {},
  );
  await api.readCourseCalendars(
    [COURSE],
    "contract-test-batch",
    { override_budget: false },
    () => {},
  );
  await api.cancelGeneration("contract-test-generation");
  await api.clearKeepCourseCurrent(COURSE);
  await api.confirmCourseDates(COURSE);
  await api.mcpClientConfigs();
  await api.diagnosticReport();
  await api.doctor();
  await api.lastCrash();
  await api.clearLastCrash();
  await api.revealDataDir();
  await api.revealLogsDir();
  await api.logUiError("contract-test error", "Error: contract-test error\n    at render");
  await api.logUiError("contract-test error without a stack", null);
  await api.updatePrefs();
  await api.setUpdatePrefs({ auto_check: true, channel: "beta" });
  await api.effectiveUpdateChannel();
  await api.startupTasks();
  await api.acknowledgeWhatsNew();
  await api.acknowledgeUpdateDisclosure();
  await api.lastUpdateCheck();
  await api.aiStatus();
  await api.modelProviderPresets();
  await api.addModelProvider("custom", OFFLINE_LLM_URL, "contract-test-key");
  await api.updateModelProviderKey(PROVIDER.provider_id, "contract-test-key");
  await api.removeModelProvider(PROVIDER.provider_id);
  await api.detectLocalServers();
  await api.listModels(PROVIDER);
  await api.testModel(PROVIDER, "contract-test-model");
  await api.setFeatureModel("weekly_note", { backend: PROVIDER, model: "m", effort: "lowest" });
  await api.setFeatureModel("weekly_note", null);
  await api.acknowledgeAiDisclosure({ kind: "codex" }, 1);
  await api.acknowledgeUnpricedModel(PROVIDER, "contract-test-model");
  await api.setMonthlyBudget(5_000_000);
  await api.setMonthlyBudget(null);
  await api.estimateGeneration({ feature: "weekly_explanation", course: COURSE, week: 3 });
  await api.estimateGeneration({ feature: "study_plan", courses: [COURSE], horizon_days: 7 });
  await api.usageSummary("2026-09-01");
  await api.usageSummary(null);
  await api.removeAllAiData();
  await api.codexStatus();
  await api.installCodex("contract-test-install", () => {});
  await api.cancelCodexInstall("contract-test-install");
  await api.removeCodex();
  await api.codexLogin("device_code", () => {});
  await api.cancelCodexLogin();
  await api.codexLogout();
  await api.setModeAWeeklyCap(40);
  await api.setModeAWeeklyCap(null);
  await api.setCodexSource("managed");
  await api.updaterStatus();
  await api.checkForUpdate();
  await api.installUpdate(() => {});

  // Channels serialise as "__CHANNEL__:<callback id>"; the id is irrelevant to the contract.
  const json = JSON.stringify(calls, null, 2).replace(/"__CHANNEL__:\d+"/g, '"__CHANNEL__:0"');
  await expect(`${json}\n`).toMatchFileSnapshot("../../src-tauri/tests/fixtures/ipc-calls.json");
});
