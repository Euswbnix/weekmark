# PageLamp desktop app

The desktop app is a small control panel. You use it to add course sources, sync them, check on
your courses, and connect the AI app you already use. It has no chat UI. Students talk to their
own AI app, which reads PageLamp over MCP (see [`docs/ARCHITECTURE.md`](../../docs/ARCHITECTURE.md)).

Stack: Tauri 2 · React 19 · TypeScript (strict) · Vite · Tailwind CSS v4 + shadcn/ui · TanStack
Query · Zustand · react-i18next · Biome · Vitest.

## Run it

Prerequisites: Node 24 (`nvm use 24`), pnpm via corepack (`corepack enable pnpm`), and for the
real app a Rust toolchain (`rustup`).

```bash
pnpm install

pnpm run dev:mock      # UI only, in the browser, with synthetic demo data (no Rust needed)
pnpm tauri dev         # the real desktop app (builds src-tauri and the Rust core)
```

The desktop app bundles the `pagelamp` CLI as a sidecar (`src-tauri/binaries/`, built by
`pnpm run build:sidecar`). Tauri builds it automatically before `tauri dev` / `tauri build`; on a
fresh checkout run `pnpm run build:sidecar` once before `cargo clippy/test -p pagelamp-desktop`.

Checks (all must pass before handing work over):

```bash
pnpm run typecheck && pnpm run lint && pnpm run test && pnpm run build
```

`pnpm run format` fixes formatting and import order.

## Smoke test against the real backend

```bash
pnpm run smoke        # creates synthetic data in <temp>/pagelamp-smoke, prints the steps, runs `pnpm tauri dev`
pnpm run smoke:clean  # deletes <temp>/pagelamp-smoke
```

It points `PAGELAMP_HOME` at a scratch folder (never your real data) and creates a synthetic
course folder with two DEMO courses. The printed steps say what to click and what to expect.

## Mock mode

`pnpm run dev:mock` (or any build with `VITE_API=mock`) swaps the Rust backend for an in-memory
fake with synthetic courses (DEMO101 "Intro to Demo Studies", …). Dates are relative to today, so
there is always something due this week. Use it to build and review screens without Canvas or a
database.

Pick a situation with `?scenario=` before the `#`:

| URL | Shows |
|---|---|
| `http://localhost:1420/#/courses` | normal demo data |
| `http://localhost:1420/?scenario=empty#/` | first run (no sources) → onboarding |
| `http://localhost:1420/?scenario=expired#/sources` | Canvas token expired |
| `http://localhost:1420/?scenario=error#/sources` | a folder source that can't be found |
| `http://localhost:1420/?scenario=busy#/sources` | another process (the CLI) is syncing |
| `http://localhost:1420/?scenario=auto-sync-due#/courses` | the sources were last synced 13 hours ago: PageLamp syncs by itself at launch |
| `http://localhost:1420/?scenario=canvas-old#/sources` | the folder and the feed synced 2 hours ago, Canvas 5 days ago, no error: the sidebar shows the oldest (automatic sync is off here) |
| `http://localhost:1420/?scenario=light-synced#/sources` | an hour ago a sync with nobody at the app read only Canvas's deadlines and announcements, and found a course whose materials haven't been read yet (automatic sync is off here, so it stays) |

In the mock, a Canvas token containing "expired" is rejected, and a folder path containing
"missing" is not found. Tests use the same mock (`src/test/render.tsx`).

## Where things are

```
src/
  api/            the backend contract
    generated.ts    TS types generated from Rust (never edit; run `pnpm run gen:types`)
    types.ts        re-exports the generated types (+ a few UI helpers)
    client.ts       PageLampApi: one method per facade call
    tauri.ts        ── Tauri boundary ── the only file that calls Rust (invoke / Channel)
    mock/           in-memory implementation + synthetic fixtures
    queries.ts      TanStack Query hooks the screens use
  app/            router, providers, start gate
  brand/          product name, colours, logo, links (see "Branding")
  components/
    ui/             shadcn/ui components (generated; edit sparingly)
    common/         shared app components (PageHeader, PolicyBadge, SecretInput, …)
    layout/         sidebar and shell
  features/       one folder per screen (onboarding, sources, courses, course, connect, settings)
  i18n/           i18next setup + locales/<lang>/<screen>.json
  stores/         Zustand: ui preferences, live sync progress
  lib/            formatting, route paths, utils
src-tauri/        Rust shell: commands.rs = thin wrappers over pagelamp_app::App
```

## The Tauri boundary

- **UI → Rust:** `src/api/tauri.ts` calls commands with `invoke`. Rust parameter names are
  snake_case and are passed from JS in camelCase (`base_url` → `baseUrl`).
- **Rust side:** `src-tauri/src/commands.rs` has one command per `pagelamp_app::App` method, with
  the same names. Commands contain no logic. If the UI needs something new, add it to the facade
  in `crates/pagelamp-app` first (ask the backend), then add a wrapper here.
- **Errors:** every command fails with `AppError { kind, message }`, which becomes an `ApiError`
  in TS. The UI branches on `kind`, never on `message`.
- **Sync progress** streams over a `tauri::ipc::Channel<SyncEvent>`.
- **Permissions** are in `src-tauri/capabilities/default.json`: the folder picker and opening
  http(s) links. There is no shell and no filesystem access.

## The IPC contract test

`src/api/tauri.contract.test.ts` calls every method of the real Tauri client with a mocked IPC
and records what crosses the boundary in `src-tauri/tests/fixtures/ipc-calls.json`;
`src-tauri/tests/ipc_contract.rs` (`cargo test -p pagelamp-desktop`) replays that file against
the real commands. A renamed argument or a missing command fails one side. After changing
`tauri.ts` on purpose, run `pnpm exec vitest run -u src/api/tauri.contract.test.ts` and commit
the updated fixture.

## Regenerating the contract types

When the backend changes a type:

```bash
pnpm run gen:types                        # runs `cargo run -p pagelamp-cli -- schema`
pnpm run gen:types --from schema.json     # or from a schema file
```

Then fix whatever `pnpm run typecheck` reports.

## Adding a screen

1. Create `src/features/<name>/<Name>Page.tsx` and start it with `<PageHeader title=… />`.
2. Add the path to `src/lib/routes.ts` and a route to `src/app/router.tsx`. If it belongs in the
   sidebar, add it to `NAV` in `src/components/layout/Sidebar.tsx`.
3. Add `src/i18n/locales/en/<name>.json` and `zh-CN/<name>.json` with the same keys, and add a
   line for the namespace in `src/i18n/i18next.d.ts`. Use it with
   `const { t } = useTranslation("<name>")`.
4. Read data with hooks from `src/api/queries.ts`. Handle loading (Skeleton), error
   (`ErrorState`) and empty (`Empty`) states.
5. Add `<Name>Page.test.tsx` using `renderRoute("/your-path")`.

## Translations

- English is the source; zh-CN must have exactly the same keys and `{{placeholders}}`.
  `src/i18n/i18n.test.ts` enforces this.
- Write natural Chinese that a student would write, not a word-for-word translation.
- Never hard-code the product name. Use `{{product}}`, which is filled in from the brand.

## Branding (distributions)

A distribution, for example a student club's build, changes the name, colours, logo, links,
default language and school-specific wording without touching screen code:

1. Copy `src/brand/brands/default.ts` to `src/brand/brands/<id>.ts` and edit it. Only use logos
   you have the rights to.
2. Build with `VITE_BRAND=<id> pnpm tauri build`.
3. For the installer's name and icons, add a Tauri config overlay, e.g.
   `src-tauri/tauri.<id>.conf.json` with `productName` and `bundle.icon`, and pass
   `--config src-tauri/tauri.<id>.conf.json`. The `identifier` doesn't decide where the data
   lives: the data folder comes from the core (`ProjectDirs("dev", "PageLamp", "PageLamp")` in
   `crates/pagelamp-core/src/paths.rs`), so every build shares it. Only the window's saved UI
   preferences (theme, language, onboarding and disclosure flags) are kept per identifier, so
   changing it makes existing users pick those again.

## Rules that are easy to break

- **Secrets:** Canvas tokens and calendar-feed URLs stay in the input field's local state. They
  never go into Zustand, localStorage, query keys, URLs, toasts or logs, and are cleared after
  submit. The Rust side stores them in the OS keychain.
- **Required wording:** the Canvas personal-use notice and the AI disclosure come from
  `common.json`. Keep them verbatim.
- **No colour-only status:** badges pair an icon with text.
- **Synthetic data only:** the repo is public. Never commit real course material, names or
  tokens.
