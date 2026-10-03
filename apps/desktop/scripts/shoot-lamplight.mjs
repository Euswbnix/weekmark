#!/usr/bin/env node
// Screenshots for the Lamplight review (docs/design/macos-shell.md §8): the same screens in light
// and dark, English and Chinese, and the platform contexts mock mode can simulate. Synthetic data
// only (mock mode). Run it before and after a change and compare the folders.
//
//   pnpm exec vite --mode mock --port 1531          # in another terminal
//   node scripts/shoot-lamplight.mjs <out-dir> [--shots courses,timeline,settings,syncing] [--unreleased]
//
// The mock as the release ships it (?shipped, lib/features.ts): screens that aren't released yet
// only appear with --unreleased, or on the screens that exist to show them.
//
// Drives Chrome/Chromium over the DevTools protocol, like record-demo.mjs (no npm packages).
// Browser: $CHROME, else Google Chrome. Base URL: $SHOOT_URL, else http://localhost:1531.
// Contexts that need a real Windows or Linux window (Mica, WebKitGTK) can't be shown here: the
// "windows" shots only set data-platform (fonts, radii) and, with "mica", the Mica tokens.

import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const out = resolve(process.argv[2] ?? "lamplight-shots");
const only = process.argv.includes("--shots")
  ? (process.argv[process.argv.indexOf("--shots") + 1] ?? "").split(",")
  : null;
const BASE = process.env.SHOOT_URL ?? "http://localhost:1531";
const unreleased = process.argv.includes("--unreleased");
const WIDTH = 1280;
const HEIGHT = 800;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// In-page steps for the shots that need a state (run after the screen settles).
const startSync = `[...document.querySelectorAll(".pl-toolbar button")].at(-1).click();
  await wait(1500);`;
const openCapsule = `document.querySelector(".pl-accessory button").click();
  await wait(500);`;

/**
 * Screens: a route (hash), a mock scenario and, optionally, steps to reach a state. `unreleased`:
 * the screen shows UI the release doesn't have yet.
 */
const SCREENS = {
  courses: { hash: "#/courses", scenario: "demo" },
  timeline: {
    hash: `#/courses/${encodeURIComponent("folder:demo-courses/course/DEMO101")}?tab=timeline`,
    scenario: "demo",
  },
  week: {
    hash: `#/courses/${encodeURIComponent("folder:demo-courses/course/DEMO101")}`,
    scenario: "demo",
  },
  policy: {
    hash: `#/courses/${encodeURIComponent("folder:demo-courses/course/DEMO101")}?tab=policy`,
    scenario: "demo",
  },
  // The policy tab's lower half: AI access and question (b).
  "policy-access": {
    hash: `#/courses/${encodeURIComponent("folder:demo-courses/course/DEMO101")}?tab=policy`,
    scenario: "demo",
    unreleased: true,
    act: `document.querySelector("main").scrollTop = 1e6; await wait(400);`,
  },
  "course-settings": {
    hash: `#/courses/${encodeURIComponent("folder:demo-courses/course/DEMO101")}?tab=settings`,
    scenario: "demo",
  },
  sources: { hash: "#/sources", scenario: "demo" },
  connect: { hash: "#/connect", scenario: "demo" },
  settings: { hash: "#/settings", scenario: "demo" },
  // The course list as a contents page (§6.4), with Current / Upcoming / Past groups.
  contents: {
    hash: "#/courses",
    scenario: "phases",
    act: `[...document.querySelectorAll("h2")].at(-1).scrollIntoView({ block: "start" });
  document.querySelector("main").scrollTop -= 56; await wait(400);`,
  },
  // The Timeline tab with a proposal (scan and AI cards, quotes, the calendar in force).
  proposals: {
    hash: `#/courses/${encodeURIComponent("canvas:canvas.demo.test/course/332")}?tab=timeline`,
    scenario: "proposals",
    unreleased: true,
  },
  // The toolbar row once content scrolls under it (glass, and the title echo).
  scrolled: {
    hash: "#/sources",
    scenario: "demo",
    act: `document.querySelector("main").scrollTop = 320; await wait(400);`,
  },
  // Floating surfaces: an open dialog, an open select (dropdown), and (sync-details) a popover.
  dialog: {
    hash: "#/sources",
    scenario: "demo",
    act: `[...document.querySelectorAll(".pl-toolbar button")].at(0).click(); await wait(600);`,
  },
  dropdown: {
    hash: "#/settings",
    scenario: "demo",
    act: `const trigger = document.querySelector('[data-slot="select-trigger"]');
  trigger.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0, pointerType: "mouse" }));
  trigger.click(); await wait(600);`,
  },
  // The accessory bar mid-sync, and its details.
  syncing: { hash: "#/courses", scenario: "demo", act: startSync },
  "sync-details": { hash: "#/courses", scenario: "demo", act: `${startSync}\n${openCapsule}` },
};

/** Appearance contexts: theme × locale × simulated platform/backdrop. */
const CONTEXTS = [];
for (const platform of ["macos", "windows", "windows-mica"]) {
  for (const theme of ["light", "dark"]) {
    for (const locale of ["en", "zh-CN"]) CONTEXTS.push({ platform, theme, locale });
  }
}

mkdirSync(out, { recursive: true });
const profile = mkdtempSync(join(tmpdir(), "pagelamp-shots-profile-"));
const port = 9335;
const chrome = spawn(
  process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
  [
    "--headless=new",
    `--remote-debugging-port=${port}`,
    `--user-data-dir=${profile}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--hide-scrollbars",
    `--window-size=${WIDTH},${HEIGHT}`,
    "about:blank",
  ],
);

async function pageSocket() {
  for (let i = 0; i < 50; i++) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
      const page = list.find((t) => t.type === "page");
      if (page) return page.webSocketDebuggerUrl;
    } catch {}
    await sleep(200);
  }
  throw new Error("browser did not start");
}

const ws = new WebSocket(await pageSocket());
await new Promise((r) => ws.addEventListener("open", r, { once: true }));
let nextId = 0;
const pending = new Map();
ws.addEventListener("message", (e) => {
  const msg = JSON.parse(e.data);
  if (msg.id && pending.has(msg.id)) {
    pending.get(msg.id)(msg);
    pending.delete(msg.id);
  }
});
function send(method, params = {}) {
  const id = ++nextId;
  ws.send(JSON.stringify({ id, method, params }));
  return new Promise((ok, fail) =>
    pending.set(id, (m) => (m.error ? fail(new Error(JSON.stringify(m.error))) : ok(m.result))),
  );
}
const evaluate = async (expression) =>
  (await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true })).result
    ?.value;

/** Wait until the page has an h1 and no loading skeleton, then let fonts and motion settle. */
async function settle() {
  for (let waited = 0; waited < 10_000; waited += 100) {
    const ready = await evaluate(
      `!!document.querySelector("h1") && !document.querySelector("[aria-busy=true]")`,
    );
    if (ready) break;
    await sleep(100);
  }
  await evaluate("document.fonts.ready.then(() => true)");
  await sleep(600);
}

try {
  await send("Page.enable");
  await send("Runtime.enable");
  // A headless page never has focus; without this the app dims the accessory bar as inactive.
  await send("Emulation.setFocusEmulationEnabled", { enabled: true });
  await send("Emulation.setDeviceMetricsOverride", {
    width: WIDTH,
    height: HEIGHT,
    deviceScaleFactor: 1,
    mobile: false,
  });
  let count = 0;
  for (const [name, screen] of Object.entries(SCREENS)) {
    if (only && !only.includes(name)) continue;
    for (const ctx of CONTEXTS) {
      const [platform, backdrop] = ctx.platform.split("-");
      const params = new URLSearchParams({ scenario: screen.scenario, platform });
      if (backdrop) params.set("backdrop", backdrop);
      if (!unreleased && !screen.unreleased) params.set("shipped", "");
      // Preferences live in localStorage (stores/ui.ts); set them, then load the screen.
      await send("Page.navigate", { url: `${BASE}/?${params}` });
      await sleep(300);
      await evaluate(`localStorage.setItem("pagelamp.ui", JSON.stringify({
        state: { theme: ${JSON.stringify(ctx.theme)}, locale: ${JSON.stringify(ctx.locale)},
                 showHiddenCourses: false, showPastCourses: true, onboardingSkipped: true,
                 aiDisclosureAcknowledgedAt: "2026-09-01T00:00:00Z" },
        version: 1 }))`);
      // A full load (not a same-document hash change), so the app reads the new preferences.
      await send("Page.navigate", { url: "about:blank" });
      await sleep(100);
      await send("Page.navigate", { url: `${BASE}/?${params}${screen.hash}` });
      await settle();
      if (screen.act) {
        await evaluate(`(async () => {
          const wait = (ms) => new Promise((r) => setTimeout(r, ms));
          ${screen.act}
        })()`);
      }
      const { data } = await send("Page.captureScreenshot", { format: "png" });
      const file = join(out, `${name}-${ctx.platform}-${ctx.theme}-${ctx.locale}.png`);
      writeFileSync(file, Buffer.from(data, "base64"));
      count += 1;
    }
  }
  console.log(`${count} screenshots in ${out}`);
} finally {
  ws.close();
  // Let Chrome exit before removing its profile, or it may still be writing into it.
  const exited = new Promise((r) => chrome.once("exit", r));
  chrome.kill();
  await Promise.race([exited, sleep(5000)]);
  rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
}
