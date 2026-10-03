# PageLamp for Mac: the "Lamplight" shell design

> **Design specification** · 2026-09-26 · status: accepted for implementation on `feat/macos-shell`
> **Scope:** the native macOS app (`apps/macos`: SwiftUI, Liquid Glass, minimum macOS 26) and the Windows/Linux translation of the Tauri 2 app (`apps/desktop`).
> **Basis:** concept **calm-desk** (ranked first by both reviews), plus the reviewers' grafts from **native-maximal** and **glance-dashboard**, with every flaw and API error they found fixed (Appendix C).
> **Inputs:** fact-checked research of 2026-09-26 (Liquid Glass APIs, Mac scope, cross-platform glass, Rust–Swift bridge; kept in the maintainers' research notes, not in this repository, and summarised where used); the current Tauri UI and its i18n files; ARCHITECTURE §3 (hard rules; rule 8 = AI access), §5, §7.
> **Never in scope:** a chat UI, any write to the LMS, glass in the content layer.

## 0. Conventions

- **[M1] [M2] [M3]** milestone of a screen, state or component (§12). M1 = shell, This Week, course detail read-only, Connect, Sources list, Settings basics (mock data allowed). M2 = full Tauri parity, required before students get the Mac app. M3 = menu bar extra, reminders, Quick Look, polish.
- **[26] [26.1] [26.4] [27]** minimum macOS of an API. 26.0 is the deployment target, so [26] needs no gate; the others sit behind `if #available(macOS …, *)`. Unmarked APIs are ≤ macOS 15. Full list: Appendix A.
- **[verify]** typechecks, but the rendered behaviour must be checked on a Mac before merging; each names a fallback.
- **Wireframes:** `G` = functional layer (Liquid Glass), `C` = content layer. `░` lamp wash · `▪` an SF Symbol named in the text · `(( … ))` our custom glass · `[ … ]` standard button · `[[ … ]]` the screen's one tinted action · `╭╮` window/sheet (system) · `┌┐` content callout. 1 column ≈ 10.7 pt. Generated on a character grid with East-Asian width 2, so Chinese lines align.
- **Code** typechecks against `MacOSX27.0.sdk` at `-target arm64-apple-macos26.0` (Appendix B). `@State` in snippets means `typealias ViewState = SwiftUI.State` until the Xcode licence is accepted (CLT builds cannot expand the `@State` macro).
- **Strings:** keys refer to `apps/desktop/src/i18n/locales/{en,zh-CN}/*.json`; *new* marks a new key; Mac-only and title-case labels live under `mac.*` (§7.3).

## 1. Concept

### 1.1 Lamplight (灯下)

Students open PageLamp for twenty seconds at a time, often late, one lamp on: where is each course this week, what is due, what did my AI app plan, may it read this course? Then they return to their AI app. PageLamp is the calm, trustworthy surface that app reads from.

**The window is a desk.** The *content layer* is paper: system backgrounds and fills, strict macOS type, hairlines instead of boxes. The *functional layer* is the lamp's glass: system sidebar, toolbar, sheets, popovers, the menu bar window, and **one floating accessory bar**. **One warm pool of light marks *now*** (today on This Week, the current week on a course); its band is full-bleed, so `backgroundExtensionEffect` spills the warm light under the glass sidebar and inspector. Step to another week and the lamp goes out. **Calm is honesty:** the AI disclosure, the Canvas personal-use notice and "No AI" withholding are printed in full sentences, never hidden in glass, tooltips or colour.

### 1.2 Principles

1. **Paper below, glass above.** Glass only on navigation, toolbars, floating controls and presentations; content never calls `glassEffect` (HIG Materials).
2. **One light.** The lamp wash marks only "now", once per screen, always beside a word ("Today", "This week"); never a text or glyph colour.
3. **Type before boxes.** HIG macOS text styles, whitespace, 0.5 pt hairlines; boxes only for callouts, code and choice tiles.
4. **One floating group.** The main window's custom glass is the bottom accessory bar (§6.2), the sidebar's selection capsule (§2.3, user decision 2026-09-26) and the course section picker's thumb (§3.2.1, user decision 2026-09-27).
5. **One tinted action per screen, often none** (This Week has none). §3.0 arbitrates.
6. **Honest by default.** No streaks, engagement badges, red countdowns or pre-ticked opt-ins; compliance text is never collapsed, truncated or colour-only; **AI policies are never colour-coded**, so a student's own "No AI" never looks like an error.
7. **Native first.** System controls, text colours, separators, radii, and the student's own accent colour.

### 1.3 Glass budget and rules

| Window | System glass (automatic) | Custom glass (ours) |
|---|---|---|
| Main | toolbar and items, search, sidebar, inspector column, sheets, popovers, menus | one bottom `safeAreaBar` with **one** `GlassEffectContainer`: status capsule + a fused neutral bubble *or* a separate tinted fix bubble; on a course's This Week section also the week scrubber + its return capsule. **≤ 4 shapes**, typically 0–1; plus the sidebar selection capsule (one `NSGlassEffectView`, §2.3) and the course section picker's thumb (one `NSGlassEffectView`, §3.2.1) |
| Welcome | title-bar area | step-bar buttons (`.glass` / `.glassProminent`), siblings in one container, **no slab behind them** |
| Settings, sheets, popovers, dialogs, MenuBarExtra | the window/presentation itself | **none**: contents use fills and standard `.bordered` / `.borderedProminent` buttons |

- `.regular` only; `.clear` is never used (text-heavy UI).
- Tint (`.regular.tint(.accentColor)`, `.glassProminent`) only on the single primary action.
- No glass on glass: nothing glass inside sheets, popovers or the menu bar window; nearby custom glass shares one container. Exception: the sidebar selection capsule sits on the system sidebar glass, as a selected segment sits in a toolbar control (user decision 2026-09-26); it is alone in its column, so it has no container.
- Exception: the course section picker's selected segment is one `NSGlassEffectView` on the lamp band, as the system's 27 segmented control draws its own thumb with glass (user decision 2026-09-27); alone, no container.
- `glassEffectID` / `glassEffectUnion` / `glassEffectTransition` only on views that carry their own `.glassEffect(…)`, never on `.buttonStyle(.glass)` buttons or system toolbar items (their glass is not addressable).
- Only `Sources/PageLamp/Chrome/` may use `glassEffect` or glass button styles; CI greps for `glassEffect|buttonStyle\(\.glass|NSGlassEffect` elsewhere.

## 2. Information architecture

### 2.1 Scenes [M1; welcome M2; menu bar extra M3]

```swift
@main struct PageLampApp: App {
  @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate   // last-window + Dock reopen
  @AppStorage("showInMenuBar") private var showInMenuBar = false            // off until the student chooses
  @AppStorage("lampLights") private var lampLights = true
  @State private var model = AppModel()                                     // @Observable
  private let firstRun = LaunchHint.needsWelcome                            // UI hint, read synchronously

  var body: some Scene {
    Window("PageLamp", id: "main") { RootView().environment(model) }       // single instance
      .defaultSize(width: 1180, height: 760)
      .windowResizability(.contentMinSize)                                  // RootView .frame(minWidth: 760, minHeight: 520)
      .defaultLaunchBehavior(firstRun ? .suppressed : .presented)
      .commands { PageLampCommands() }                                      // + SidebarCommands, InspectorCommands, ToolbarCommands
    Window("Welcome to PageLamp", id: "welcome") { WelcomeFlow().environment(model) }
      .windowStyle(.hiddenTitleBar)
      .windowResizability(.contentSize)                                     // 680 × 620
      .windowBackgroundDragBehavior(.enabled)
      .restorationBehavior(.disabled)
      .defaultLaunchBehavior(firstRun ? .presented : .suppressed)
    Settings { SettingsView().environment(model) }
    MenuBarExtra(isInserted: $showInMenuBar) { MenuBarWeekView().environment(model) } label: {
      Image(systemName: lampLights && model.dueWithin24h ? "lamp.desk.fill" : "lamp.desk")
        .accessibilityLabel(model.menuBarAccessibilityLabel)
    }
    .menuBarExtraStyle(.window)
  }
}
```

### 2.2 Launch, windows, refresh, threading

- **One window at launch, no flash:** `LaunchHint.needsWelcome = !UserDefaults.bool("hasSourcesOrSkipped")`, refreshed after each `status()` (a UI hint, not the disclosure acknowledgement, §13); a stale hint lands on S3 (**Set Up PageLamp…**). Finishing welcome = `openWindow(id: "main")` + `dismissWindow(id: "welcome")`; closing it = **Skip for Now**.
- `applicationShouldTerminateAfterLastWindowClosed` returns `!showInMenuBar`; a Dock click reopens the main window.
- **Refresh** on `NSApplication.didBecomeActiveNotification`, after each sync/download, and every 5 s while `status().sync_in_progress` is true for another process (S17). If `status()` fails: S2, sidebar collapsed.
- **Monday's weekly note [M3]** (preview builds): `startup_tasks(now).prepare_weekly_note` is asked at launch (60 s later when the login item is on: the network or a local model may not be up; a failed launch read counts as a read), every hour while the app runs, 60 s after a wake (the hour stops when the Mac goes to sleep and restarts at the wake, so an hour that passed asleep never fires then) and on activation at most every 15 minutes or on a new day. Each answer that says so starts Monday's run unless a run is going; the facade checks and records the one try a Monday in one write transaction as the run starts (`start_automatic_note`), so asking often, and from both apps, never prepares it twice.
- **Threading:** every facade call goes through the UniFFI `PageLamp` object as `async` + `spawn_blocking`; `SyncEvent`s arrive on `pagelamp-rt` threads and reach `@MainActor` through an `AsyncStream` (bridge research §2.3–2.4).

### 2.3 Sidebar [M1] (custom source list: `ScrollView` + `LazyVStack` in the system sidebar column; `.navigationSplitViewColumnWidth(min: 200, ideal: 232, max: 300)`)

| Section | Rows |
|---|---|
| — | **This Week** / 本周 · `lamp.desk` |
| **Courses** / 课程 | visible, active courses by code: icon `book.closed` (`hand.raised` for No AI, neutral); title = code (or name); trailing `HStack` (not `.badge`, which means a count): `exclamationmark.triangle` if its source is failing, then "Wk 4" / "第 4 周" (*new* `common.week.compact`) or "—"; full name in the `.help` tooltip and in the VoiceOver label (not as an accessibility help, which would say it twice) |
| **Hidden** / 已隐藏 [M2] | only with View ▸ Show Hidden Courses; `eye.slash`, `.secondary` |
| **Past Courses** / 往期课程 [M2] | `Section(isExpanded:)`, collapsed; `enrollment_active == false` |
| **Setup** / 配置 | **Sources & Sync** / 数据来源与同步 · `folder.badge.gearshape`, trailing count (callout, `.secondary`), only for n failing sources; VoiceOver value "{{count}} source(s) need attention"; **Connect AI App** / 连接 AI 应用 · `cable.connector`, warning glyph while `temporary_location` is set |
| footer (`.safeAreaInset(edge: .bottom)`) | plain text button → Sources, **Subheadline 11 pt**, never glass: "Synced 2 h ago" / "2 小时前同步", "Syncing…" / "正在同步…", "Offline · synced 3 h ago" / "离线 · 3 小时前同步", "Needs attention" / "需要处理", "Not synced yet" / "还没有同步过" (glyph + words) |

**Status roles** (fixes the "status in three places" finding): the footer is the persistent low-emphasis state and never shows counts; the capsule is the transient event channel (progress, results, fixes) on the current page; the toolbar's Sync Now is an action. Critical problems also appear in content callouts, the Sources badge and the capsule, never only at the sidebar bottom. The list itself colours icons with the accent (`.tint`) while the window is active and `.secondary` when it is inactive, and makes the highlighted title Semibold (27's accent sidebar icons, reproduced). No Settings row: ⌘, opens the Settings scene.

**Selection: one glass capsule (user decision, 2026-09-26, final).**

- The highlighted row sits on one Liquid Glass capsule that looks like the selected segment of the system segmented control: light, translucent, specular rim, soft depth, untinted; not the system's solid accent fill.
- It slides to the new row whenever the selection changes: click (tap-to-click included), keyboard, type-select, ⌘1/2/3, the status capsule's fix button, any programmatic navigation.
- The sidebar's own glass stays system-drawn (the split view's sidebar column).
- Implementation: one `NSGlassEffectView` (`.regular`, radius h/2) in `Chrome/SidebarSelectionGlass.swift`, moved by an additive `CASpringAnimation` = `motion.quick` (`PLMotion.quickSpring`).

**Why the system List selection is replaced.**

- On 27, `List(selection:)` draws its selection per row as an `NSVisualEffectView` (material `.selection`, radius 8). Whenever the list has focus in a key window it becomes a solid `controlAccentColor` fill with a white Semibold title, and it jumps from row to row.
- The 27 SDK has no API to restyle it: `listRowBackground` sits above it, but the title still turns white; `selectionDisabled` only removes selectability; a `List` without `selection:` loses the highlight and keyboard selection; setting the private `NSOutlineView`'s `selectionHighlightStyle` is fragile.
- So the rows are custom and reproduce the source list's geometry, keys, pointer and VoiceOver behaviour as measured on 27.2. `SidebarLayout` and `SidebarNavigation` live in PageLampModel with tests.

**Motion order (no jank).**

- The page switch goes first: a selection sets `AppModel.destination` at once, and the bold title moves with it.
- The capsule starts only after the page's first frame has been drawn and the main thread has been free for a whole display frame, detected with a display link (`SidebarMotionGate`), with a cap of 0.25 s. (SwiftUI animations and the segmented control's thumb are advanced on the main thread; a slide that shares frames with a 60–126 ms page build steps 30–90 px per frame.)
- Selections made while a page is building are coalesced: the latest wins, and at most one page waits.
- Holding ↑/↓, or dragging, glides the capsule without building pages; release commits once the capsule is within 1 pt, at most 0.2 s.
- Retargets add to the running spring (the velocity carries over). Reduce Motion: the capsule jumps. `scripts/perf-probe.sh capsule` measures it with real input events.
- **Open question, decided on device** (Preview builds: Debug ▸ Sidebar Capsule Moves Before the Page): whether the capsule should lead instead. Measured with the capsule probe at 60 Hz, the default starts the capsule ~150 ms after the input (the page appears at ~115 ms); leading starts it at ~50 ms and the page follows two display frames later (~150 ms; one frame is not enough, the update cycle can flush both together). While it leads, the slide overlaps the page build: the render server draws the Core Animation spring, but a main-thread probe cannot see those frames, so only eyes can judge it. The page and the toolbar follow `AppModel.pageDestination` in both modes. The losing variant is removed after the decision.

**Metrics** (measured on 27.2, except the small row height: see below the table; x from the sidebar edge; W = column width):

| | Small | Medium | Large |
|---|---|---|---|
| Row height = capsule height (radius h/2) | 24 | 32 | 40 |
| Title (Regular; Semibold when highlighted) | 11 | 13 | 15 |
| Title x | 42 | 46 | 48 (native; if the wide `folder.badge.gearshape` glyph touches the title on device, match a native list at large) |
| Icon (Medium, `imageScale(.large)`, `.tint` / `.secondary`) | 11 | 13 | 15 |
| Icon column from x 16 (centre) | 22 (27) | 26 (29, measured) | 28 (30) |

Small row height: 24 is the source-list table's `rowHeight` at the small size; medium and large match the system's SwiftUI sidebar exactly. At small that list uses automatic row heights and sizes each row to its content, about 5 pt above and below the tallest glyph (25–27 pt with these icons, so its rows sit up to 7.5 pt lower by Connect). The custom list keeps a uniform 24 on purpose: the one capsule keeps its height as it slides from row to row, like a segmented control's thumb.

Every size: trailing items callout 12, ending at W − 16; headers 19 pt tall with a 13 pt gap above (none as the first item), 11 pt Semibold `.secondary` at x 14, vertically centred; capsule x 10, width W − 20; focus ring 3 pt, 0–3 pt outside the capsule, `keyboardFocusIndicatorColor`; Show Borders 1 pt `separatorColor` inside. Row 0 sits at the toolbar safe area (y 52), no extra padding; medium row tops in the window 52, 97, 116, 148, 180, 225, 244, 276. The one source of these numbers is `SidebarMetrics`.

**States:**

| State | Capsule | Title / icon |
|---|---|---|
| Rest, window active | regular glass, untinted | Semibold `.primary` / `.tint` (accent) |
| Focused by Tab, ⌃F6 or any sidebar key (window key) | + focus ring | same |
| Focused by a click | no ring (the first key shows it) | same |
| Window inactive | unchanged, no ring | Semibold / icons `.secondary` |
| Hover | none (native sidebars have none) | none |
| Pressing, dragging, holding ↑/↓ | glides to the previewed row | bold on the previewed row |
| Page still building | waits (≤ 0.25 s), then slides | bold already moved |
| Selection not in the list (hidden course) | hidden; fades in when the row returns | no bold row |
| Reduce Motion | jumps, no fade | same |
| Reduce Transparency / Increase Contrast / 27 glass slider | system glass (frosted, high contrast) [verify] | same |
| Show Borders | + 1 pt outline | same |
| Snapshots | flat `.quaternary` capsule with a `.separator` stroke | same |

**Keyboard** (sidebar focused):

| Key | Action |
|---|---|
| Tab / ⇧Tab, ⌃F6 | into and out of the list: one stop, with or without Full Keyboard Access (`.focusable(interactions: .edit)`) |
| ↑ / ↓ key-down, also with ⇧ ⌃ ⌘ | previous / next row, page at once, no wrap, headers skipped |
| ↑ / ↓ held | preview glide; the page commits on release |
| ⌥↑ / ⌥↓ | first / last row |
| Home / End | scroll to top / bottom; the selection does not change |
| Page Up / Page Down | passed on (the system list ignores them) |
| letters, digits, symbols without ⌘ or ⌃ | type-select: searches after the highlighted row and wraps; more letters within 2 × (key-repeat delay + interval) (1.167 s by default) extend the prefix; case, diacritics and width ignored; headers never match; no match changes nothing |
| Space | extends an active search; otherwise passed on |
| Return, Esc, ←, → | passed on (Esc still dismisses the status capsule's attention) |
| menu shortcuts (⌘1/2/3, Go, ⌘R …) | handled by menus first; the capsule follows after the page |

**Pointer:** a press selects on mouse-down, like `NSTableView`; a drag previews and commits on release, a cancel reverts; clicks on headers or gaps only focus the list; control-click selects in M1 (M2: the context menu must not select, as in native lists).

**VoiceOver:**

```
AXScrollArea
  list "Sidebar"                      (LazyVStack + .contain + label; roleDescription "list")
    AXStaticText "This Week"          selected, press
    AXHeading "Courses"
    AXStaticText "DEMO205, Foundations of Sample Data, Week 4, source needs attention"   (the label has the full name)
    AXHeading "Setup"
    AXStaticText "Sources & Sync"     value "1 source needs attention"
    AXStaticText "Connect AI App"     value <temporary-location title, when set>
AXButton footer status (after the list)
```

Row modifier order (measured): `.accessibilityElement(children: .ignore)`, label, value, `.isSelected` (committed row only), `.accessibilityAction`, `.accessibilityRemoveTraits(.isButton)`, `.accessibilityAddTraits(.isStaticText)`; before them `.contentShape(.accessibility, .rect)`, so VoiceOver's outline covers the whole row the capsule marks, not only icon and text. The capsule is hidden from VoiceOver. Keyboard commits post an announcement ("label, value"); pointer, menu and VoiceOver-press changes do not. The list reports no selected child (the system's outline does), so when the keyboard brings focus to it (Tab, ⌃F6: the focus ring shows) the highlighted row is announced too; a click is not. The course tooltip is not an accessibility help (the label already has the full name). Headers are `.isHeader`.

### 2.4 Navigation model [M1]

```swift
enum Destination: Hashable, Codable { case thisWeek, course(String), sources, connect }
enum CourseSection: String, CaseIterable, Codable { case week, deadlines, timeline }
@Observable final class CourseUIState { var section: CourseSection = .week; var selectedWeek: UInt32? = nil } // nil = now
```
- Two-column `NavigationSplitView`, no back stack; the inspector belongs to the course detail. `@SceneStorage` restores the destination and the inspector; a course keeps its section and week for the session and opens at *now* after relaunch.
- Search (⌘F) is a temporary mode of the detail column (§3.8).
- Cross-links: "Open Sources & Sync" → `.sources`; "Set Term Dates…" / "AI Policy…" open the inspector scrolled to that section; menu bar rows call `openWindow(id: "main")`, `NSApp.activate()`, then set the destination.

### 2.5 Toolbar per screen

| Screen | Leading (`.navigation`) | Title / subtitle | Trailing (`.primaryAction`) |
|---|---|---|---|
| This Week [M1] | — | "This Week" / rolling "Sep 26 – Oct 2" (`Date.IntervalFormatStyle`) | **Sync Now** (`arrow.triangle.2.circlepath`, untinted, ⌘R) |
| Course [M1] | `ControlGroup { Previous Week; Next Week }` (every section, enabled in This Week only: hiding it re-tiled the toolbar on each section switch and made the picker's thumb stutter) · **This Week** (`arrow.uturn.backward`, own item, only when away from now; enabled in This Week only) | course code | **Download Files…** (`square.and.arrow.down`, icon-only, Canvas with `downloadable_files > 0`) [M2] · **Open Course Website** (`safari`, icon-only, `.visibilityPriority(.low)` [26.1]) · `ToolbarSpacer(.fixed)` · **Inspector** (`sidebar.trailing`) |
| Sources & Sync [M1] | — | "Sources & Sync" / "Where PageLamp gets your course data" | **Add Source…** (`plus`) [M2] · `ToolbarSpacer(.fixed)` · **Sync All** (`.glassProminent` + `.sharedBackgroundVisibility(.hidden)` when it wins §3.0, else a default toolbar button) |
| Connect [M1] | — | "Connect AI App" | — |

All screens: `.searchable(placement: .toolbar)` [M2]. Every item is also a menu command; items hide whole; icon-only items carry a `Label`; text and icon-only items never share a background. Overflow order (narrow, zh-CN): website, then download; week controls never. On 26.0 (no `visibilityPriority`) the website item is dropped below 900 pt (it stays in the Course menu). Download progress never morphs a toolbar item; it runs in the capsule.

### 2.6 Inspector [M1 read-only · M2 editable]

`.inspector(isPresented:)` + `.inspectorColumnWidth(min: 260, ideal: 300, max: 360)` on the course detail only; `InspectorCommands` gives ⌃⌘I. Open on the first course visit when the window is ≥ 1100 pt, then the student's choice persists. `Form(.grouped)` with **AI Policy**, **Course Materials**, **Term Dates**, **Course** (§3.2), replacing the Tauri "AI policy" and "Settings" tabs. M1 shows values with `LabeledContent`. Closing it never hides AI state: the header's AI status line, the Course menu and Settings ▸ Privacy all expose it.

### 2.7 Menus and shortcuts

| Menu | Items |
|---|---|
| PageLamp | About (standard panel via `CommandGroup(replacing: .appInfo)`) · Settings… ⌘, · Hide · Quit |
| File | Add Source… ⇧⌘N [M2] · Sync Now ⌘R · Download Course Files… (no shortcut) [M2] · Close ⌘W |
| Edit | Undo/Redo ⌘Z ⇧⌘Z via `UndoManager` [M2] for Hide Course, AI access, AI policy, term dates · Find ▸ Search… ⌘F (`.searchFocused`) [M2] |
| View | Show/Hide Sidebar · Show/Hide Toolbar ⌥⌘T · Show/Hide Inspector ⌃⌘I · This Week ⌘1 · Sources & Sync ⌘2 · Connect AI App ⌘3 · Show Hidden Courses ✓ [M2] · Enter Full Screen ⌃⌘F |
| Go | Previous Week ⌘[ · Next Week ⌘] · Current Week ⇧⌘T |
| Course (`@FocusedValue`) | Open Course Website · Download Files… · AI Policy… · Let My AI App Read Materials ✓ (disabled for No AI) · Set Term Dates… · Hide Course / Show in Course List |
| Window | standard · Set Up PageLamp… [M2] |
| Help | PageLamp Help · Copy Diagnostic Report… · Report a Problem… · Open Logs Folder |

- ⌘T is HIG-reserved (Fonts) and ⌥⌘T toggles the toolbar, hence ⇧⌘T. ⌘←/→ are left alone (input-source switching). ⌘[ / ⌘] are "align" keys in the HIG, but PageLamp has no text formatting and Safari/Finder use them for back/forward. ⌘S saves the AI-policy draft while the inspector has focus.
- Menu icons stay hidden on 27 (all items are verbs; none opts in with `.labelStyle(.titleAndIcon)`).
- Context menus: course (the Course menu's items) · material (Open, Quick Look [M3], Show in Finder, Copy Link) · deadline (Open in Browser, Go to Course, Copy Title) · source (Sync, Replace…, Show in Finder, Remove…).

### 2.8 Data behind each screen

| Screen | Reads | Writes / actions |
|---|---|---|
| Shell | `status()`, `list_courses()`, `last_crash()` | `clear_last_crash()` |
| This Week | `this_week(…)` (§13 #2); until then `list_deadlines(nil, 7, 0)` + `latest_study_plan()` + `list_courses()` with `thisWeek.ts` grouping ported (M1 only); `weekly_notes()`, `estimate_generation(weekly_note)` [M3] | `sync_all`; `write_weekly_note`, `cancel_generation`, `delete_weekly_note` [M3] |
| Course | `course_overview`, `week_materials(course, week)`, `list_deadlines(course, 14, 0)` and `(course, 0, 7)` | `set_course_policy`, `set_course_ai_access`, `set_course_term`, `set_course_hidden`, `download_course_files` |
| Sources | `list_sources()`, `status()` | `sync_all`, `sync_source`, `add_folder_source`, `add_ical_source`, `add_canvas_source`, `update_source_secret`, `remove_source` |
| Connect | `mcp_client_configs(url)`, `mcp_launch(url)`, `doctor().mcp_clients` (which clients are *configured*) | none: never writes AI-app configs |
| Settings | `status()`, `doctor()` (version if the DB is down), `diagnostic_report()`, `logs_dir()`, `list_courses()`; `weekly_note_settings()` [M3] | `set_course_ai_access`; `set_prepare_weekly_note_on_monday` [M3] |
| Search | `search(query, nil, 50)` | — |

`url` = `Bundle.main.url(forAuxiliaryExecutable: "pagelamp")` (the CLI sidecar).

## 3. Screens and states

### 3.0 Shared rules

- **Backgrounds:** pages keep the system background; callouts, code and row fills use `.fill.tertiary` / `.quaternary` / `.secondary`; settings-like surfaces use `Form(.grouped)`. The system then handles Increase Contrast, the 27 tint slider and both appearances.
- **ReadingColumn:** `.frame(maxWidth: 760, alignment: .leading)`, centred, 40 pt gutters (24 below a 760 pt detail). **LampBand** (§6.1) is the first view of every reading page.
- **Callout** (never glass): glyph + Headline title + body + actions on `.fill.tertiary` in `.rect(cornerRadius: 12)` + `.containerShape(.rect(cornerRadius: 12))`; 1 pt `.separator` border only under Increase Contrast / Show Borders. Tone = glyph only (`info.circle`, `exclamationmark.triangle` warning, `key`/`xmark.octagon` danger, `lock.shield` privacy); text stays `.primary`.
- **CodeBlock:** `.callout.monospaced()`, `.textSelection(.enabled)`, in `ConcentricRectangle(corners: .concentric(minimum: .fixed(6)), isUniform: true)` on `.fill.quaternary`; Copy (`.bordered .controlSize(.small)`) shows "Copied" / 已复制 for 2 s (`.contentTransition(.symbolEffect(.replace))`), announced.
- **Primary-action arbiter** (one tinted action per window): the first present of (1) **Save AI Policy** with a dirty draft, (2) the fix for the first failing source (**Replace Token…** / **Replace Feed Address…**), (3) **Set Term Dates…** when the course's phase is unknown (M0.10; exam period, break, not started and ended have known dates, so the button stays on the page but isn't tinted), (4) the page primary (Sync All; Add a Source… in S3; Sync Now in S4; Try Again in S2) renders `.borderedProminent` / `.glassProminent`; every other candidate `.bordered`. Sheets keep their own default button.

### 3.1 This Week (home) [M1; day ribbon M3]

```text
W1 · Main window, This Week (1180 × 760 pt)
╭──────────────────────┬────────────────────────────────────────────────────────────────────────────────╮
│ ● ● ●          [|] ░░│░░ This Week ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░             ↻        ⌕ Search       │  ← G toolbar; the wash runs under it
│                  ░░░░│░░ Sep 26 – Oct 2 ░░░░░░░░░░░░░░░░░░░░                                          │
│ [▪ This Week     ]░░░│░░░░░ Friday, September 26                                                      │  ← C lamp band (full-bleed); ░ also mirrored under the sidebar
│                    ░░│░░░░░ 3 deadlines in the next 7 days · 2 plan tasks today                       │
│ Courses             ░│░░░░░ ▪ Next up  Problem Set 2 — questions 1–3 · DEMO205                        │  ← only when due ≤ 24 h
│  ▪ DEMO101    Wk 4   │░░░             Due today 11:59 PM · in 58 min            [ Open Course ]       │
│  ▪ DEMO205    Wk 4   │      Fri 26  Sat 27  Sun 28  Mon 29  Tue 30  Wed 1  Thu 2                      │  ← day ribbon [M3]
│  ▪ DEMO310  ▪ Wk 5   │                                                                                │
│  ▪ DEMO099    Wk 2   │      Next 7 days                               3 deadlines · 1 class           │
│                      │      ────────────────────────────────────────────────────────────────          │
│ Setup                │      Today       11:59 PM  Problem Set 2 — questions 1–3  DEMO205  Assignment  │
│  ▪ Sources & Sync  1 │      Fri Sep 26   2:00 PM  Lecture 9                      DEMO101  Class       │  ← classes .secondary
│  ▪ Connect AI App    │      Tomorrow     9:00 AM  Quiz 3                         DEMO101  Quiz        │
│                      │                                                                                │
│                      │      Your study plan                                                           │
│                      │      Made by your AI app 2 days ago · covers Sep 22 – Oct 5                    │
│                      │        ○ Skim Week 4 slides                          DEMO101    30 min         │
│                      │      Show Full Plan ▾   Read-only here. To change the plan, ask your AI app.   │
│                      │                                                                                │
│                      │      Your courses                                                              │
│                      │      DEMO101  Intro to Demo Studies ······················ Week 4              │  ← Contents list (§6.4)
│                      │               Next: Quiz 3 · Sat 9:00 AM   ▪ Learning aid only · 12 of 14      │
│ ✓ Synced 2 h ago     │                                                                                │  ← sidebar footer, not glass
│                      │                 (( ✓ Sync finished · 3 new materials ))                        │  ← G status capsule
╰──────────────────────┴────────────────────────────────────────────────────────────────────────────────╯
```

```swift
ScrollView {
  VStack(alignment: .leading, spacing: 40) {
    LampBand(lit: true) { TodayHeader(); NextUpLine(); DayRibbon() /* [M3] */ }
    ReadingColumn { CrashNotice(); SourceProblemCallouts(); Next7DaysSection(); WeeklyNoteSection() /* [M3] */; StudyPlanSection(); ContentsSection() }
  }
}
.scrollEdgeEffectStyle(.soft, for: .bottom)
.safeAreaBar(edge: .bottom) { AccessoryBar(showsScrubber: false) }
.navigationTitle("This Week").navigationSubtitle(rollingRange)
.toolbar { ToolbarItem(placement: .primaryAction) { SyncNowButton() } }
```
- **Header:** `Text(.now, format: .dateTime.weekday(.wide).month(.wide).day())`, Large Title; summary Callout `.secondary`, one sentence per count.
- **Next up** (§6.5) only when the earliest real deadline is ≤ 24 h away; else "Next: Quiz 3 · Sat 9:00 AM" (`courses.card.next`) or nothing. **Day ribbon [M3]:** 7 plain buttons, today first (under the pool), dots for deadline days; a click scrolls to that day.
- **Next 7 days** (rolling, so titled so): `Grid(alignment: .leadingFirstTextBaseline)` — day (120 pt; relative day, then date) · time (mono, right-aligned) · title (Semibold if due today) · code · kind (`.secondary`). Classes `.secondary`; undated events excluded. Rows are plain Buttons → course Deadlines; `accessibilityRotor("Deadlines")`.
- **Your study plan** (read-only): today + 2 days, **Show Full Plan** (`DisclosureGroup`); static `circle` / `checkmark.circle.fill` glyphs (values "Done" / "Not done yet"); notes callout; stale → `clock` (warning) + `courses.plan.stale`; footer `courses.plan.readOnly`. A plan PageLamp wrote says so (`courses.plan.madeByPageLamp`, `stalePageLamp`, `mac.plan.readOnly`) and carries its AI-generated line.
- **Weekly note [M3]** (design §5.3, §7; the Tauri app's weekly note card), preview builds until shipped, between Next 7 days and Your study plan: a few sentences on the week and three things to focus on, from the courses' structure and the plan (no material text). The note shown (the run's new one, else the newest kept, or one picked under Earlier notes, the last 5) with "For the week of Sep 28, 2026" (· "Prepared on Monday"), its focus list (each item's course a link to it), what was left out as graded work, "This note is for an earlier week.", the AI-generated line with **Copy** (the note, the focus list with each course, the label) and **Delete…** (asked first). "≈ $x" and **Write My Weekly Note** / **Write a New Note** (plain, never tinted); the run's headline and stage with **Stop** (one cancel per press). The run belongs to the app: leaving This Week doesn't stop it. Monday's run (§2.2) never goes over the budget and never moves focus; "not due any more" and "nothing to write about" (the week emptied as the run started; Write already says so) end silently, anything else leaves one line ("Monday's note wasn't prepared: …") until a click or a note. VoiceOver hears the stages and how a run ended while the card shows, never the note as it's written.
- **Plan with PageLamp… [M3]** (design §5.1, §7; the Tauri app's Plan page), preview builds until shipped: a sheet on the main window. The form (days to plan, hours per week, days off, the active courses, a note) with "≈ $x" and the facade's block (go over the budget this time; use a model without a price; else why, with Settings ▸ AI for a missing model or disclosure); Write My Plan; the run's headline and stage with Stop (one cancel per press); the draft by day with its warnings, courses planned from structure only, unscheduled tasks and the AI-generated line; Discard, Write Again (its own "≈ $x") and Use This Plan (saved as the study plan This Week shows). VoiceOver hears the stages and how the run ended, never the plan as it's written; a failure is read out. Closing the sheet stops a run in flight; nothing is saved until Use This Plan.

| Element | English | 简体中文 |
|---|---|---|
| summary | 3 deadlines in the next 7 days · 2 plan tasks today | 未来 7 天有 3 个截止日期 · 今天有 2 项学习任务 |
| next up | Next up · Due today 11:59 PM · in 58 min | 接下来 · 今天 23:59 截止 · 还有 58 分钟 |
| section | Next 7 days (*new* `courses.thisWeek.next7`) | 未来 7 天 |
| plan | Made by your AI app 2 days ago · covers Sep 22 – Oct 5 | 2天前由你的 AI 应用生成 · 覆盖 9月22日 至 10月5日 |
| read-only | Read-only here. To change the plan, ask your AI app. | 这里仅供查看。想调整计划，直接跟你的 AI 应用说就行。 |

### 3.2 Course detail [M1 read-only · M2 editing, scrubber, downloads · M3 Quick Look, Explain]

```text
W2 · Course detail, This Week section, current week, inspector open
╭──────────────────────┬────────────────────────────────────────────────────┬──────────────────────────────╮
│ ● ● ●          [|] ░░│░░ DEMO205   ‹ ›                   ⤓   ↗ ░░░░░░░░   │░░░      ⌕ Search      [|]    │  ← G toolbar
│ ▪ This Week        ░░│░░ DEMO205 · Canvas · synced 2 h ago                │ AI Policy                    │  ← C eyebrow · G inspector (system)
│                     ░│░░ Foundations of Sample Data                       │ How can you use AI in        │
│ Courses              │░░ ▪ Learning aid only · ▪ 12 of 14 readable        │ this course?                 │  ← C AI status line → inspector
│  ▪ DEMO101    Wk 4   │░░ [ This Week │ Deadlines │ Timeline │ Explain ]   │ ( ) Not set                  │  ← custom glass segmented control (§3.2.1)
│ [▪ DEMO205    Wk 4 ] │░░ Week 4 · Sep 22 – 28         This week           │ ( ) No AI                    │  ← C week line (adjustable); lit = now
│  ▪ DEMO310  ▪ Wk 5   │                                                    │ (•) Learning aid only        │
│  ▪ DEMO099    Wk 2   │   Modules  ▪ Week 4: Sampling  ▪ Lab 3             │     AI can help you under-   │
│                      │   Materials              12 of 14 readable         │     stand the material, but… │
│ Setup                │   ────────────────────────────────────────         │ ( ) Allowed with citation    │
│  ▪ Sources & Sync  1 │   ▪ Week 4 slides — Sampling    Readable           │ ( ) No restrictions          │  ← C material rows
│  ▪ Connect AI App    │     File · Week 4: Sampling · Sep 22               │ Rule from your syllabus      │
│                      │   ▪ Lab 3 recording        Can't be read           │ [ "Use of generative AI…" ]  │
│                      │     File · Lab 3 · Sep 23                          │ ● Unsaved changes            │
│                      │   ▪ PS 2 data.csv         Not downloaded           │  Discard  [[Save AI Policy]] │  ← the one tinted action
│                      │   ┌──────────────────────────────────────────┐     │ ───────────────────────────  │
│                      │   │ ▪ 2 files here aren't downloaded yet, so │     │ Course Materials             │
│                      │   │   your AI app can't read them.           │     │ Let my AI app read    (━●)   │
│                      │   │                     [Download Files…]    │     │ this course's materials      │
│                      │   └──────────────────────────────────────────┘     │ Term Dates              ▸    │
│ ✓ Synced 2 h ago     │   Recent announcements                             │ Course                  ▸    │
│                      │  (( ‹  1  2  3 [4]  5  6  7  8  9  10 › ))         │                              │  ← G week scrubber [M2]
╰──────────────────────┴────────────────────────────────────────────────────┴──────────────────────────────╯
```

```swift
ScrollView {
  VStack(alignment: .leading, spacing: 32) {
    LampBand(lit: ui.showsCurrentWeek) { CourseEyebrow(); CourseTitle(); AIStatusLine(); CourseSectionPicker(ui: ui); WeekLine() }
    ReadingColumn {
      SourceAlert()
      switch ui.section { case .week: WeekSection(); case .deadlines: DeadlinesSection(); case .timeline: TimelineSection() }
    }
  }
}
.safeAreaBar(edge: .bottom) { AccessoryBar(showsScrubber: ui.section == .week) }
.inspector(isPresented: $inspectorShown) { CourseInspector().inspectorColumnWidth(min: 260, ideal: 300, max: 360) }
.navigationTitle(course.code ?? course.name)
.toolbar { CourseToolbar() }
```

**Header band.**
- Eyebrow "DEMO205 · Canvas · synced 2 h ago" (*new* `mac.course.eyebrow`; + " · Syncing now…" / " · 正在同步…"). Title: course name, Large Title Semibold, always sans.
- **AI status line:** one plain Button → inspector at AI Policy; neutral glyph + ink text, **never colour-coded**: `questionmark.circle` "AI policy not set · Set…" / "尚未设置 AI 使用规定 · 去设置…" (*new* `course.aiStatus.notSet`) · `hand.raised` No AI / 禁止使用 AI · `lightbulb` Learning aid only / 仅限辅助学习 · `quote.opening` Allowed with citation / 注明后可用 · `checkmark.circle` No restrictions / 没有限制; then `common.aiMaterials.*` ("12 of 14 readable by your AI app" / "共 14 份资料，AI 应用可读取 12 份" · "AI access to materials is off" / "已关闭 AI 读取资料" · "Materials not shared (No AI course)" / "资料不共享（禁止使用 AI 的课程）"), plus Past course / 往期课程 and Hidden / 已隐藏.
- **Section picker:** This Week · Deadlines · Timeline / 本周 · 截止日期 · 教学进度, then Explain / 讲解 [M3] in preview builds until shipped: a custom segmented control that reads as the system's 27 tabs control, whose selected segment is one glass thumb that slides on every change (§3.2.1); where the band is narrower than its natural width (273 pt English, 243 Chinese; with Explain 364 and 324, so the menu takes over below a window of about 943 / 903 pt with the sidebar and inspector at their ideal widths [estimated]), the system pop-up menu (`.pickerStyle(.menu)`). A course opened at Explain where it is off shows This Week.
- **Week line:** "Week 4 · Sep 22 – 28" (Title 1, mono digits, `.numericText(value:)`) + a word tag: This week / 本周 · Last week / 上周 · Next week / 下周 · In 2 weeks / 2 周后 · 2 weeks ago / 2 周前 (*new* `course.week.relative.*`). "Week 4 of 12" / "第 4 周（共 12 周）" (*new* `common.week.ofTotal`) needs §13 #3; the date range needs §13 #4 (else omitted). One adjustable VoiceOver element (§6.3).
- **Lit** when the line shows the current week and today is inside the term (This Week section: displayed week = `timeline.current_week`; other sections always show the current week). Away from now (week 6): band unlit, tag "In 2 weeks", toolbar **This Week**, and the scrubber's "↩ This Week" capsule splits off. Unknown: "Week unknown" / 周次未知, no pool (S11).

**This Week section.**
- Modules line (`square.stack.3d.up`, Callout `.secondary`); "Materials" + "12 of 14 readable".
- **MaterialRow** (content, two lines, hairlines, no cards): kind glyph (`doc.text` · `doc.richtext` · `text.book.closed` · `megaphone` · `link`), title, meta "File · Week 4: Sampling · Sep 22", trailing glyph + short status (*new* `mac.materialStatus.*`; VoiceOver reads `common.textStatus.*`): ok `checkmark.circle` Readable / 可读取 · pending `hourglass` Waiting / 等待处理 · not_downloaded `arrow.down.circle` Not downloaded / 未下载 (+ `downloadBlock.*`) · unsupported `minus.circle` Can't be read / 无法读取 · error `exclamationmark.triangle` Couldn't extract / 提取失败 · course withheld `hand.raised` Not shared / 不共享 · course turned off `hand.raised` Off for AI / 未向 AI 开放.
- **Mac list behaviour** (the page must stay a `ScrollView` for the spill, so rows are custom): `.focusable()` + `@FocusState<MaterialID?>`; click focuses; double-click (`.onTapGesture(count: 2)`) or Return (`.onKeyPress(.return)`) opens (`NSWorkspace.shared.open` for `file://`, browser for http); ↑/↓ via `.onMoveCommand`; Space → `.quickLookPreview($url, in: urls)` [M3]; focus fill `.fill.secondary` radius 8 + system ring; hover `.fill.quaternary` (none under Reduce Highlighting Effects [26.4]); `accessibilityRotor("Materials")`.
- **Download callout [M2]** (Canvas, `downloadable_files > 0`, not No AI): `download.callout_*` + **Download Files…**; hidden for No AI (its sentence is about AI reading), the toolbar item stays. Then recent announcements.

```text
W3a · Deadlines section
╭──────────────────────────────────────────────────────────────╮
│   ▪ These are titles and dates only. PageLamp never          │
│     downloads assignment instructions to solve them.         │
│   Coming up                      Due in the next 14 days     │
│   Today       11:59 PM  Problem Set 2 — q. 1–3   Assign. ↗   │
│   Thu Oct 9    6:00 PM  Midterm                  Exam    ↗   │
│   Recently past                  Due in the last 7 days      │
│   Mon Sep 22  11:59 PM  Problem Set 1            Assign. ↗   │
╰──────────────────────────────────────────────────────────────╯
```

**Deadlines:** the rule-4 note always first — "These are titles and dates only. PageLamp never downloads assignment instructions to solve them." / "这里只有标题和日期。PageLamp 不会下载作业要求，更不会用它们替你做作业。" Coming up / 即将截止 (14 days) · Recently past / 最近已截止 (7 days, `.secondary`); rows end with a labelled **Open in Browser** (`arrow.up.forward.app`); empty → `course.deadlines.empty.*` + Open Sources & Sync →.

```text
W3b · Timeline section
╭──────────────────────────────────────────────────────────────╮
│   Where this course is now                                   │
│   This is very likely right. The clues are listed below.     │
│    1   2   3  [4]  5   6   7   8   9  10  11  12             │
│                ▲ now                                         │
│   How we worked this out                                     │
│   • Module "Week 4: Sampling" unlocked Sep 22                │
│   Wrong week?  [Set Term Dates…]                             │
╰──────────────────────────────────────────────────────────────╯
```

**Timeline:** confidence in words (`timeline.confidence.*` + `common.confidence.*`). **Term strip [M2]:** plain buttons 1…`total_weeks` (else up to `max(available_weeks)`), empty weeks `.tertiary`, the current week marked "▲ now" in `.primary` (not a second lamp); a click opens This Week at that week. Evidence verbatim, tagged English (§7.1). **Set Term Dates…** opens the inspector (tinted only via the arbiter).

```text
W3c · Explain section [M3]
╭──────────────────────────────────────────────────────────────╮
│   Explain a week                                             │
│   PageLamp asks your model to explain the week's materials…  │
│   Week  [ Week 4 (this week) ▾ ]                             │
│   ≈ $0.01 · OpenAI · gpt-6-luna         [ Explain Week 4 ]   │
│   AI-generated · OpenAI · gpt-6-luna · Sep 25…  Copy Delete… │
│   Week 4 slides — Sampling and Surveys                       │
│   The key idea of **…** is …   [▤ Week 4 slides…, p. 2]      │
│   Check your understanding   1. What is the main point of …? │
│   Not read this time                                         │
│   Assignment 4 — Survey Simulation (looks like graded work)  │
│   [ Not Graded Work? Include It and Write Again ]            │
│   Earlier explanations   Fri, Sep 25, 9:40 AM        Show    │
╰──────────────────────────────────────────────────────────────╯
```

**Explain [M3]** (design §5.2, §7; the Tauri app's Explain tab), preview builds until shipped: the fourth section, with its own week (the course's default week; a picker of the weeks with materials; "Recent materials" when the weeks aren't known). Hidden, No AI and AI access off say why instead of Generate (off: **Course Materials…** shows the course's state in the inspector, read-only in M1; No AI: **AI Policy…**). "≈ $x" and the facade's block as in Plan, then **Explain Week 4**; the run's headline ("Writing with OpenAI · gpt-6-luna"), how many materials it reads and its stage, with Stop (one cancel per press). The explanation: the AI-generated line with **Copy** (the text, each paragraph's sources and the label, as the Tauri app copies it) and **Delete…** (asked first); each section's paragraphs (the Markdown subset: strong, emphasis in medium weight, code; links, images and HTML stay text) with source chips that open the material (its file on this computer, else its link); citations the facade removed; the check questions; what wasn't read and why, with **Not Graded Work? Include It and Write Again** for what only looks like graded work; the course's "cite AI use" note; after a course's first cloud run, the one-time question whether its materials may be shared with AI services (three answers and Dismiss). A stale explanation offers **Write Again**. Each run starts from "≈ $x" for exactly what it sends: Generate from the week's; Include It from its own line above the button (the explanation's include plus the left-out materials the facade brings back, with its over-budget tick); Write Again of an explanation written here with materials included sends them again, priced the same way (its line under the stale banner), else from the week's. Earlier explanations of the week (the last 5) can be shown again. The run belongs to the course: switching sections keeps it going, leaving the course stops it, and a cloud run stops when the course's answer becomes "not allowed" or it becomes hidden, No AI or off in another app. VoiceOver hears the stages and how the run ended (announcements, no capsule), never the text as it's written; focus moves to the result.

**Download flow [M2]** (W6e): `.confirmationDialog(titleVisibility: .visible)` + `.dialogIcon(Image(systemName: "arrow.down.circle"))`; message = `download.viewingNotice` **first** ("Downloading files through Canvas can count as viewing them (e.g. module 'must view' requirements)." / "通过 Canvas 下载文件可能会被算作“已查看”（例如满足模块里“必须查看”的要求）。") then `download.dialogDetail`; **Download** / Cancel. Progress runs in the capsule ("Downloading 12 of 40 files · DEMO205" / "正在下载 12/40 个文件 · DEMO205"); result `download.done_*`, plus a fused **Details** bubble listing skipped files (first 5 + `moreWarnings_*`).

**Inspector [M1 read-only · M2 editable].**
- **AI Policy** / AI 使用规定: `course.policy.title`; `Picker(selection:).pickerStyle(.radioGroup)` — name + `common.policy.description.*` (also the hint); `policy.explanation`; brand `aiPolicyHint`; syllabus `TextEditor` (≥ 72 pt). **Before saving No AI:** "When you save, DEMO205's materials stop being shared with your AI app." / "保存后，DEMO205 的资料将不再共享给你的 AI 应用。" (*new* `course.policy.consequence`). Row: 6 pt warning dot "Unsaved changes" / 有未保存的更改 · **Discard Changes** · **Save AI Policy** (⌘S) → capsule "✓ AI policy saved", undoable. Leaving dirty asks "Discard your AI policy changes?" / "要放弃对 AI 使用规定的更改吗？".
- **Course Materials** / 课程资料: `Toggle(.switch)` "Let my AI app read this course's materials" / "允许 AI 应用读取这门课的资料", immediate, undoable; note `aiAccess.note.*`. **No AI:** shown off and `.disabled(true)`, stored `ai_access` kept (rule 8), note `withheld_by_policy` as text and hint. Capsule `aiAccess.nowOn` / `nowOff`.
- **Term Dates** / 学期日期: `term.description`, `breaksNote`; first day `DatePicker(.field)`; a "Set a last day" checkbox reveals the optional second picker; source "Set by you" / 由你设置 · "From Canvas" / 来自 Canvas · "No dates yet" / 还没有日期 (*new* `course.term.source.*`); **Save Dates**, **Use Synced Dates** (`set_course_term(nil, nil)`); `term.invalid` inline; undoable.
- **Course** / 课程: source + freshness; **Hide this course** + `settings.hideDescription` → capsule "DEMO099 is hidden. Your AI app won't see it." + fused **Undo ⌘Z** (6 s; Edit ▸ Undo keeps working); Open Course Website; Download Files…; `pastCourse.hint`.

#### 3.2.1 Section picker: one glass thumb (user decision 2026-09-27, final)

**Decision.**

- The picker reads as the system's 27 tabs control (`NSSegmentedControl`, role `.tabs`, `.fillEqually`): the same track, thumb inset and radius, label type and colour, heights and spacing, measured on 27.2 (`SegmentedMetrics`).
- Its selected segment is the shared glass thumb (`Chrome/GlassThumb.swift`, also the sidebar capsule's): one `NSGlassEffectView` (`.regular`, radius 4) moved by an additive Core Animation spring, `PLMotion.sectionSpring`, drawn by the render server.
- It slides on **every** change: a click (a trackpad tap too), Space, VoiceOver's press, a model change (Go menu, ⇧⌘T), and a cross-link that opens a course at a section (`ThisWeekNavigation.open(courseId:section:)`). The new section's build (~20 ms of main thread) never makes it stutter.
- Narrow: the system pop-up menu, unchanged. macOS 26: the same custom control (the 26 system control, `.segmented` with an accent fill, is not kept).
- Keyboard and VoiceOver stay as good as the system control's: parity measured against it (below); Preview builds compare on device with Debug ▸ Course Pages Use the System Section Picker (removed after sign-off).

**Why AppKit, not SwiftUI with `.accessibilityRepresentation`.**

- `GlassSegmentedControl` subclasses `NSControl`, so it keeps AppKit's own focus rules of the system control (`acceptsFirstResponder` true, `canBecomeKeyView` and `needsPanelToBecomeKey` false with Full Keyboard Access off: no Tab stop; with it on, a Tab stop at the system control's place), first mouse, key routing and the automatic focus ring (`drawFocusRingMask` around the key segment). A click never takes focus, with Full Keyboard Access off or on, as with the system control (its `mouseDown` doesn't call `NSControl`'s).
- Its accessibility is written by hand: the control is the tab group, one `NSAccessibilityElement` per section is a tab (three; four with Explain) (`GlassSegmentedAccessibility.swift`), so VoiceOver's focus can sit on one segment like the system control's.
- Pointer events arrive directly: a mouse-down and mouse-up in the same run-loop pass still select (the system control ignores them).
- An `NSControl` answers AppKit's legacy accessibility queries from its cell (it has none here), so in-process legacy reads say `AXUnknown`; what VoiceOver reads is SwiftUI's node for the view, which reflects the modern overrides. Verify out of process (below).

**Metrics** (`SegmentedMetrics`, `SegmentedLayout`; English / Chinese course labels):

| | Value |
|---|---|
| Control | 273 × 24 / 243 × 24: each segment wants its text + 24 rounded up to 0.5, + 4 (tabs style), + 1 before every segment but the first; every slot takes the widest (91 / 81); checked against AppKit's `intrinsicContentSize` in the tests. With Explain [M3]: 364 × 24 / 324 × 24 (the widest slot is unchanged) |
| Track | radius 6 continuous; ink (black light, white dark) 4.7 % + a 3.0 % sheen; Increase Contrast 14.9 %; Show Borders 1 pt ink 12.5 %, plus-darker / plus-lighter, track only |
| Thumb | inset 2, height 20, radius 4 continuous: 87 / 86 / 86 at x 2 / 94 / 185 (77 / 76 / 76 at 2 / 84 / 165); Explain's 86 at 276 (76 at 246) |
| Labels | system 13 Regular in every state, `labelColor` (84.7 %), 100 % with Increase Contrast; boxes 16 tall at y 4, their text width rounded up, centred on the thumb (x 14 / 107 / 202.5); baseline 17 pt from the top: each title's line fragment drawn at its box's top-left (`NSString.draw(at:)`), as SwiftUI lays out the system control's labels (a 13 pt line 16 tall, baseline at 13) |
| VoiceOver frames | the slots: 91 wide at x 0 / 91 / 182, Explain 273 (81), full height, as VoiceOver reads the system control's segments |

**States:**

| State | Track | Thumb | Labels | Ring / input |
|---|---|---|---|---|
| Rest, light or dark | 4.7 % + 3.0 % | glass at the selection | `labelColor` | none |
| Focused (Tab with Full Keyboard Access, VoiceOver focus, forced) | same | same | same | AppKit ring around the key segment's thumb, 3 pt out, focus colour, zoom-in |
| Clicked, Full Keyboard Access off | — | — | — | takes no focus, no ring |
| Pressed or dragging | same | at the pressed segment / following the pointer | same | selection unchanged until mouse-up |
| Window inactive | same | system inactive glass | same | no ring |
| Disabled | same | same (the system `.tabs` control doesn't change either) | same | no pointer, keys, press or VoiceOver focus; `AXEnabled` 0 |
| Reduce Motion | same | jumps | same | same |
| Reduce Transparency / Increase Contrast / 27 glass slider | 14.9 % with Increase Contrast | system glass [verify] | 100 % with Increase Contrast | same |
| Show Borders | + 1 pt outline | no outline | same | same |
| Hover | none | none | none | no tracking areas |
| Snapshots | real track | flat `.fill.quaternary` + 1 pt `.separator` | SwiftUI text | — |

**Pointer:** mouse-down starts the thumb toward the pressed segment in the same event; drags retarget it to follow the pointer (widths snap, ≤ 1 pt); mouse-up commits the segment under the pointer, clamped (a release far outside still commits; no cancel), a release on the selection only brings the thumb back; a lost mouse-up (a new press, leaving the window) reverts; the first click in an inactive window activates it and selects. Not reproduced: the system's press lens (the thumb grows to 101 × 32 and magnifies the labels): no public API.

**Keyboard** (the control first responder; measured identical to the system control with Full Keyboard Access off and on, the latter per process):

| Key | Action |
|---|---|
| Tab / ⇧Tab | not a stop with Full Keyboard Access off (AppKit's `NSControl` rule); with it on, a stop in both directions at the system control's place (7th of 13, between the AI status button and Open Sources & Sync), the key segment kept [U1 on device]; ⌃Tab leaves |
| ← / →, also with ⌘ ⌥ ⇧ ⌃, key repeat | move the key segment and wrap; the selection stays; the ring and VoiceOver's focus follow |
| Space (no modifiers) | selects the key segment (it slides); ⇧ ⌃ ⌥ ⌘ Space go on |
| ↓ (any modifiers) | taken, nothing happens (the system control would open a segment's menu) |
| ↑, Return, Enter, Home, End, Page Up / Down, letters, Esc | go on, as from the system control (the same keys reach the window unhandled: ↑ Return Enter Home End and a modified Space beep, the others pass silently) |
| menu shortcuts (⌘[ ⌘] ⇧⌘T) | menus; the thumb slides when `section` changes |

The key segment survives focus changes; a click moves it to the clicked segment. One difference, kept on purpose: the system control's key segment snaps back to the selection whenever SwiftUI updates the picker for any reason (the Esc that closes the capsule's warning, an environment change); ours moves only on keys, clicks, VoiceOver and selection changes.

**VoiceOver** (after the AI status button, before the week line; out of process, as VoiceOver reads it):

```
AXTabGroup "Course sections"   value = selected tab; no actions of its own (SwiftUI adds AXScrollToVisible); nothing settable
  AXRadioButton/AXTabButton "This Week"   role description "tab", value 1/0, AXFocused settable (not while disabled), AXPress
  AXRadioButton/AXTabButton "Deadlines"
  AXRadioButton/AXTabButton "Timeline"
```

The title is the description (no AXTitle, no AXSelected). Focus is on a segment, never the group: the key segment while the control is first responder (setting AXFocused on a segment makes the control first responder at that segment). Press selects and slides, focus unchanged; pressing while disabled does nothing. Notifications, as the system control posts them (measured out of process with an `AXObserver`, the way VoiceOver listens): **no value-changed notification** for any selection change; focused-element-changed on the key segment when it moves while the control is first responder and when a focused control selects (Space, also on the selected segment, a click, VoiceOver's press); nothing when a click or press selects on an unfocused control. When focus arrives (Tab, ⇧Tab, VoiceOver focus), AppKit posts the one focused-element notification for the key segment itself (the control adds none), and SwiftUI posts one for its own node of the view, which reads the SwiftUI label: `.accessibilityLabel` on the representable ("Course sections"), as the system Picker's label reaches it. "1 of 3" ("1 of 4" with Explain) is VoiceOver's count of the children [verify U3].

**Motion** (`perf-probe.sh segment`): one additive `CASpringAnimation` on `position` (`sectionSpring`, 0.25 s, bounce 0; the system click slide, ω 26.4, ζ 1.05), started in the input's own handler; the control writes the selection to the model on the next run-loop turn, so this turn's commit carries the slide to the render server before the new section builds (a forced `CATransaction.flush()` did the same but stalled the content's crossfade on back-to-back switches). The new frame and its spring are committed in one transaction (`GlassThumbMover.slide`): outside an event (VoiceOver's press) no implicit transaction is open, and committed apart the thumb would show at its destination until the spring arrived with the next commit, after the section build; a model change starts in the SwiftUI update that carries it (with that update's commit); a cross-link's new control starts at `CourseUIState.pickerSection` (where the thumb last stood) and slides once in the page's first frame; later visits start in place. Retargets add up (the velocity carries over). Reduce Motion: jumps.

**Parity with the system control** (HEAD 3dca928 and the Debug switch, same machine, 27.2, 60 Hz):

| # | Aspect | System control | Custom | Status |
|---|---|---|---|---|
| 1 | Size | 273×24 / 243×24 | same (AppKit cross-check test) | match |
| 2 | Slot | the band's leading edge, 16 below the AI line, 8 above the week line | same frame in the window | match |
| 3 | Track | 4.7 % + 3.0 %, radius 6; IC 14.9 %; Show Borders 1 pt 12.5 % plus-darker / plus-lighter | same (SwiftUI) | match |
| 4 | Thumb frames | 87/86/86 at 2/94/185; 77/76/76 at 2/84/165; height 20, radius 4 | same | match |
| 5 | Thumb material | SDF glass | `NSGlassEffectView`: same values + a ring shadow (0.06), frosted fill, white vibrancy underlay | justified: slightly frostier, no public API |
| 6 | Press lens | 101×32, magnified labels | none | justified: no public API |
| 7 | Labels | 13 Regular, `labelColor` in every state, IC 100 %; baseline 17.0, ink rows 7.0–17.5 (English) / 6.5–18.5 (Chinese) at 2× | same; ink centroid within 0.002 pt, light and dark, English and Chinese | match |
| 8 | Disabled look | unchanged | unchanged | match |
| 9 | Focus ring | AppKit ring at the key segment + 3 pt: ink (−1, −1, 93, 26), accent 50 %, zoom-in, hidden when inactive | AppKit ring, same ink and colour | match |
| 10 | Hover | none | none | match |
| 11 | Click slide | main-thread spring ω 26.4; stutters (34–41 ms frames) | render-server `sectionSpring`: every read within 0.4 pt of the ideal curve | better (requested) |
| 12 | First movement | 50–75 ms after mouse-down | the spring starts 2–7 ms after the input is handled (committed before the new section builds) | better |
| 13 | Space slide | ω 18.8 (≈ 0.33 s) | 0.25 s | justified: one motion per control |
| 14 | Drag follow | lens spring ω 18.2 | the 0.25 s spring retargets | justified |
| 15 | Model change / cross-link | jumps / appears in place | slides | requested |
| 16 | Down + up in one pass | ignored | selects | justified |
| 17 | Reduce Motion | still slides on a click, Space and VoiceOver's press (measured with Reduce Motion reported on); a model change and a cross-link jump / appear in place as always | jumps on every trigger | better (per decision); U5 |
| 18 | Selection timing | on mouse-up | on mouse-up | match |
| 19 | Drag and release | follows x, release commits, no cancel | same | match |
| 20 | First click, inactive window | `acceptsFirstMouse` true | true | match |
| 21 | Click focus | none, with Full Keyboard Access off or on (the window / sidebar keeps focus) | same | match |
| 22 | Tab, Full Keyboard Access off | not a stop (14 Tabs) | same | match |
| 23 | Tab, Full Keyboard Access on | a stop, 7th of 13 (after "AI policy not set", before "Open Sources & Sync"), both directions, the key segment kept | same | match (measured per process); U1 on device |
| 24 | ← / → (plain, ⌘ ⌥ ⇧ ⌃, repeat) | move the key segment, wrap | same | match |
| 25 | Space | plain Space selects; ⇧ ⌃ ⌥ ⌘ Space go on | same | match |
| 26 | Other keys | ↓ taken; ↑ Return Enter Home End and a modified Space beep; Page Up/Down, letters, Esc pass silently; page scroll unchanged. The key segment snaps back to the selection on any SwiftUI update of the picker (the Esc that closes the capsule's warning, an environment change) | same keys, beeps and scroll offset; the key segment stays | match; the snap-back is not copied (justified: focus shouldn't move on an unrelated update) |
| 27 | Accessibility focus | follows the key segment | same | match |
| 28 | Group | tab group "Course sections" (also `AXAttributedDescription`), value, SwiftUI's 275×26 frame, no actions, nothing settable; SwiftUI's node for the focused view labelled "Course sections" | same (pressing the group anyway: an error from the system control, success with no effect from ours) | match |
| 29 | Segments | tab button, description, value 1/0, no `AXSelected`, `AXFocused` settable (not when disabled), `AXPress` | same | match (the system control also lists nil AXHelp / AXIdentifier / AXUserInputLabels) |
| 30 | Frames | segments 91/91/91 at 0/91/182 (81) | same | match |
| 31 | Accessibility press | selects, slides, no focus change; no notification when unfocused; on the focused control it moves the focus (focused-element on the pressed segment, also when that segment is already selected) | same | match |
| 32 | Disabled accessibility | `AXEnabled` 0, press and focus ignored | same | match |
| 33 | VoiceOver speech | inferred | same tree | U3 |
| 34 | Narrow | system menu; switches at ≤ 852 (en) / ≤ 822 (zh) with the inspector open | unchanged; same widths; widening doesn't slide | match |
| 35 | macOS 26 | `.segmented` accent fill | custom | intentional; U8 |
| 36 | Notifications (out of process, `AXObserver`) | never value-changed; focused-element on the key segment when it moves while focused, when a focused control selects (Space 2×, also on the selected segment; press 3×) and when focus arrives (1×, plus SwiftUI's labelled node); re-posts on SwiftUI updates of the picker (the window resigning key; a click on the picker while the sidebar has focus re-posts the sidebar) | the same notifications, each once (Space 1×, press 2×); no re-posts on unrelated updates | match (a repeat on the same element tells VoiceOver nothing new; speech: U3) |

### 3.3 Sources & Sync [M1 list, sync, progress · M2 add, replace, remove, drop]

```text
W5 · Sources & Sync (content column): expired token, live progress, OK
╭──────────────────────────────────────────────────────────────────────────╮
│  Sources & Sync                          [+]   [ ↻ Sync All ]            │
│  Where PageLamp gets your course data                                    │
│  ┌────────────────────────────────────────────────────────────────────┐  │
│  │ ▪ Demo Canvas                                    ▪ Access expired  │  │
│  │   Canvas · Connected as Demo Student                               │  │
│  │   ▪ Your Canvas token has expired or was revoked. Create a new one │  │
│  │     in Canvas (Account → Settings → Approved Integrations → + New  │  │
│  │     Access Token) and replace it here.          [[Replace Token…]] │  │
│  │   https://canvas.demo.test ↗              Last synced 3 days ago   │  │
│  │   [Sync]                                                   Remove… │  │
│  └────────────────────────────────────────────────────────────────────┘  │
│  ┌────────────────────────────────────────────────────────────────────┐  │
│  │ ▪ Fall term                                 Syncing · 12 of 40     │  │
│  │   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━───────────────────   │  │
│  │   Extracting text — week4-slides.pdf                   (en text)   │  │
│  │   Folder  ~/Documents/Courses                    [Show in Finder]  │  │
│  └────────────────────────────────────────────────────────────────────┘  │
│  ┌────────────────────────────────────────────────────────────────────┐  │
│  │ ▪ LMS calendar                                            ✓ OK     │  │
│  │   Feed address  ▪ Private feed address (stored in your keychain)   │  │
│  │   Last synced 2 hours ago        [Sync]  [Replace Feed Address…]   │  │
│  └────────────────────────────────────────────────────────────────────┘  │
│  PageLamp keeps your courses on this computer. Your AI app reads         │
│  them only when you ask.                                                 │
│            (( ◔ Syncing 1 of 3 · Fall term │ Details ))                  │
╰──────────────────────────────────────────────────────────────────────────╯
```

- `Form { ForEach(sources) { Section { … } header: { SourceHeader } } }.formStyle(.grouped)`; `LabeledContent` rows (`sources.card.*`); folder paths mono + **Show in Finder**; feeds show `lock` + "Private feed address (stored in your keychain)" — secrets are never rendered (rule 3).
- Status glyph + words (`common.sourceError.*`): OK `checkmark.circle` · Not synced yet `circle.dashed` · Syncing `ProgressView(value:total:)` · Needs attention `exclamationmark.triangle` · Access expired `key`. Fixes sit in the problem callout (`sources.problem.expiredCanvas` / `expiredFeed`); the first wins the arbiter.
- Live progress: bar + backend message (tagged English) + "12 of 40" (`.numericText()`); then "Last run" (Done / error / "Warnings (2)" / "提示（2 条）") until **Hide Results**. Announce start and end only. Busy → S17.
- **Folder drop [M2]:** `.dropDestination(for: URL.self) { items, session in … }` [26]; dashed outline "Drop to add as a course folder" / "松开即可添加为课程文件夹"; opens Add Source pre-filled. Empty: `ContentUnavailableView` (`folder.badge.plus`) + **Add Your First Source**. Footer: `common.disclosure.short`, always.

```text
W6a · Add Source sheet, folder + feed chosen (≈ 560 pt)
╭──────────────────────────────────────────────────────────╮
│  Add a Source                                            │
│  Your new source starts syncing as soon as it's added.   │
│  ┌───────────────────────┐ ┌───────────────────────┐     │
│  │ (•) Course folder +   │ │ ( ) Canvas token      │     │
│  │     calendar feed     │ │ Personal use          │     │
│  │ Recommended · any LMS │ │                       │     │
│  └───────────────────────┘ └───────────────────────┘     │
│  Course folder [ ~/Documents/Courses ] [Choose Folder…]  │
│  [x] Term start [Sep 2, 2026 ▾]  Name [e.g. Fall term]   │
│  Calendar feed address (optional)                        │
│  [ •••••••••••••••••••••••••••••• ] [Show]               │
│  ▪ This address is private — PageLamp stores it in your  │
│    system keychain.                                      │
│                          [Cancel]  [[Add Source]]        │
╰──────────────────────────────────────────────────────────╯
```

```text
W6b · Add Source sheet, Canvas chosen
╭──────────────────────────────────────────────────────────╮
│  ┌───────────────────────┐ ┌───────────────────────┐     │
│  │ ( ) Folder + feed     │ │ (•) Canvas token      │     │
│  └───────────────────────┘ └───────────────────────┘     │
│  ┌───────────────────────────────────────────────────┐   │
│  │ ▪ For your own use                                │   │
│  │ Personal access tokens are for your own use only, │   │
│  │ and they expire — Canvas shows the maximum when   │   │
│  │ you create one (often 30–90 days). Helping        │   │
│  │ classmates set up? Point them to the course       │   │
│  │ folder + calendar feed option instead.            │   │
│  └───────────────────────────────────────────────────┘   │
│  Canvas address  [ https://canvas.example.edu    ]       │
│  Access token    [ ••••••••••••••••••••  ] [Show]        │
│  ▾ How do I create a token?  (expanded the first time)   │
│  ▪ PageLamp checks the token with Canvas, then stores    │
│    it in your system keychain.                           │
│                          [Cancel]  [[Add Source]]        │
╰──────────────────────────────────────────────────────────╯
```

- **Add Source sheet [M2]** (`.sheet` + `.presentationSizing(.form)`): tiles are radio `Button`s (`.isSelected` trait; selected = 1.5 pt accent stroke + dot): "Course folder + calendar feed · Recommended · works with any LMS" / "课程文件夹 + 日历订阅 · 推荐 · 适用于任何教学平台"; "Canvas access token · Personal use" / "Canvas 访问令牌 · 仅限个人使用". Folder via `fileImporter(… [.folder])` or drop; optional term start = checkbox + `DatePicker(.field)`. Secrets: `SecureField` ⇄ `TextField` Show toggle ("Show token" / "Hide token"); values live only in the field.
- **Canvas notice (rule 2):** always expanded **above** the fields: `person.crop.circle.badge.checkmark`, "For your own use" / 仅供本人使用, `common.canvasNotice` + `shareHint`; how-to (`tokenHowTo`, `noTokenButton`) in a `DisclosureGroup`, open the first time; `httpWarning` for http.
- **Disclosure gate (rule 8):** while unacknowledged the sheet starts with the full disclosure and an unticked "I understand"; **Add Source** stays disabled with the visible hint `onboarding.welcome.acknowledgeFirst`.
- Submit shows "Checking your token…" / 正在验证访问令牌…; errors inline from `sources.addErrors.<kind>.<error>`.

```text
W6c Replace Token · W6d Remove confirmation · W6e Download confirmation
╭──────────────────────────────────────────────────────────╮
│  Replace Token                                           │
│  Create a new token in Canvas (Account → Settings →      │
│  Approved Integrations → + New Access Token) and paste   │
│  it here. Demo Canvas and its courses stay as they are.  │
│  New access token  [ •••••••••••••••••••• ] [Show]       │
│  ▪ Canvas didn't accept this token. It may have expired  │
│    or been revoked — create a new one.                   │
│                      [Cancel]  [[ Checking… ◌ ]]         │
├──────────────────────────────────────────────────────────┤
│        ▪ trash      Remove Demo Canvas?                  │
│  This removes the source and everything synced from it:  │
│  its courses, the course files you downloaded, your      │
│  AI-policy and term settings for those courses, and its  │
│  stored access token. Nothing in Canvas changes.         │
│           [ Remove Source ]   [ Cancel ]                 │
├──────────────────────────────────────────────────────────┤
│      ▪ arrow.down.circle  Download this course's files?  │
│  Downloading files through Canvas can count as viewing   │
│  them (e.g. module 'must view' requirements). PageLamp   │
│  downloads the course's files to this computer and       │
│  extracts their text. Nothing is submitted or changed    │
│  in Canvas.                  [ Cancel ]  [ Download ]    │
╰──────────────────────────────────────────────────────────╯
```

- **Replace** (`update_source_secret`): `.defaultFocus` on the field; **Replace** is the default; success → capsule "Token replaced for Demo Canvas" + fused **Sync Now**.
- **Remove:** `.confirmationDialog` + `.dialogIcon(Image(systemName: "trash"))`, body per kind (`sources.remove.description.*`), `Button(role: .destructive)`, and Cancel with `.keyboardShortcut(.defaultAction)` so Return never removes [verify; fallback `.alert` with Cancel default].

### 3.4 Connect your AI app [M1; running check, copy variants, access popover M2]

```text
W7 · Connect AI App (content column)
╭──────────────────────────────────────────────────────────────────────────╮
│  Connect your AI app                                                     │
│  PageLamp works inside the AI app you already use. Add it                │
│  once, then ask about your courses in plain language.                    │
│  ┌────────────────────────────────────────────────────────────────┐      │
│  │ ▪ What your AI app can read                                    │      │
│  │ When you ask your AI app about a course, it reads that course's│      │
│  │ materials from PageLamp and sends them to your AI … (full)     │      │
│  │ You confirmed this on Sep 26, 2026.      Review Course Access… │      │
│  └────────────────────────────────────────────────────────────────┘      │
│  ▪ Starts in the background     ▪ Data stays on this computer            │
│  ▪ Set up once per app          ▪ Moved PageLamp? Copy setup again       │
│  Set up  [ Claude Desktop │ Claude Code │ Codex │ Other app ]            │
│  Easiest to start with · Add to a config file                            │
│  1  Quit Claude Desktop completely first (⌘Q). It saves over this        │
│     file when it quits, so changes made while it's open are lost.        │
│     ▪ Claude Desktop is open right now.                                  │
│  2  Open the configuration file at this location.                        │
│     ~/Library/Application Support/Claude/claude_desktop_config.json      │
│     [Copy Path]  [Show in Finder]                                        │
│  3  If the file is new or empty, paste in this snippet.                  │
│     ┌────────────────────────────────────────────────────────┐           │
│     │ {"mcpServers": {"pagelamp": {"command": "/Applicatio…  │           │
│     └────────────────────────────────────────────────────────┘           │
│     [Copy Configuration]                                                 │
│     ▸ It already has an "mcpServers" section   → [Copy Entry Only]       │
│     ▸ Other settings but no "mcpServers" → [Copy mcpServers Section]     │
│  4  Save the file, then open Claude Desktop again.                       │
│  Try it   "What's happening in my courses this week?"   [Copy]           │
╰──────────────────────────────────────────────────────────────────────────╯
```

- **Order:** temporary-location warning (S15) → intro → **full AI disclosure** + `acknowledgedOn` → How it works → Set up → Good to know (`note_codes`) → Try it → quarantine hint (ad-hoc builds only).
- **Client picker:** `sortClientConfigs` order, `.tabs` [27] / `.segmented`, last "Other app" / 其他应用. Preselect the first client already *configured* per `doctor().mcp_clients` (a returning student re-copying after moving the app), else Claude Desktop ("Easiest to start with" / 最容易上手); configured clients show "✓ Set up" / "✓ 已设置" (*new*). Steps from `install_kind`, exactly as Tauri `InstallSteps`.
- **Claude Desktop step 1** (*new* `mac.connect.quitFirst`): "Quit Claude Desktop completely first (⌘Q). It saves over this file when it quits, so changes made while it's open are lost." / "先完全退出 Claude Desktop（⌘Q）。它退出时会覆盖这个文件，所以在它运行时做的修改会丢失。" **Live check [M2]** (read-only; `NSWorkspace.shared.runningApplications` + `didLaunch/didTerminateApplicationNotification`): "Claude Desktop is open right now." / "Claude Desktop 正在运行。" or "Claude Desktop is closed. You can edit the file now." / "Claude Desktop 已退出，现在可以编辑这个文件了。" Bundle id [verify] (believed `com.anthropic.claudefordesktop`), else the line is omitted. PageLamp never quits the app or writes its config.
- **Copy:** Copy Path, Show in Finder (`activateFileViewerSelecting`, or the parent folder), **Copy Configuration**; [M2] **Copy Entry Only** / **Copy mcpServers Section** under "It already has an "mcpServers" section" / "…other settings but no "mcpServers"" (`copyEntry`, `copyKey`, `steps.pasteJsonEntry/Key`; content per §13 #5). In a temporary location every Copy first asks "Copy anyway? It will stop working after you move PageLamp." / "仍要复制吗？移动 PageLamp 之后，这段设置就会失效。"
- **Review Course Access… / 查看各门课的读取情况… [M2]:** read-only popover ("DEMO099 · Not shared (No AI)", "DEMO310 · Hidden — never shown"). Errors: `connect.errorTitle` + Try Again; `connect.empty.*`.

### 3.5 Settings (Settings scene, 600 pt; `TabView` + `Tab`; `Form(.grouped)`)

```text
W8a · Settings ▸ General (600 pt)
╭────────────────────────────────────────────────────────────╮
│  [General]   Reminders   AI   Data   Privacy   Help        │
├────────────────────────────────────────────────────────────┤
│  Language        [ System (English)      ▾ ]               │
│                  The menu bar switches language after      │
│                  PageLamp reopens.       [Reopen Now]      │
│  Appearance      [ System │ Light │ Dark ]                 │
│  Week starts on  [ System (Sunday)       ▾ ]               │
│  Menu bar        [ ] Show PageLamp in the menu bar         │
│                  [ ] Light the lamp when something is due  │
│                      within 24 hours                       │
│  Login           [ ] Open PageLamp at login                │
╰────────────────────────────────────────────────────────────╯
```

- **General** [M1]: Language System / English / 简体中文 — content live (`.environment(\.locale)` + `LocalizedStringResource.locale`); menus and dialogs follow `AppleLanguages`, hence **Reopen Now** (sets the app-domain `AppleLanguages`, relaunches with `NSWorkspace.openApplication(at:configuration:)` + `createsNewApplicationInstance`, terminates [verify]). Appearance (`NSApp.appearance`). **Week starts on [M2]** (*new* `settings.general.weekStartsOn`, 每周开始于): System / Monday / Sunday → `Calendar.firstWeekday` for pickers, week ranges, the weekly reminder and `this_week`. Menu bar and **Open at login** (`SMAppService.mainApp`, independent; [verify] on the signed preview build, ad-hoc builds may not register) [M3]: both off until the student turns them on; the system keeps the login item (System Settings ▸ General ▸ Login Items) and PageLamp writes no launch agent or plist of its own. **Reminders tab [M3]**, preview builds until shipped: "Remind me with notifications" (the Mac app's own answer, kept by the facade under an `app.mac.*` key so it never turns on the Tauri app's tray or login item), the permission as macOS has it (not asked until the student says yes; denied → Open Notification Settings), deadlines coming up (48 h and 24 h before), your week (day and time), today's study plan (time). Notifications are `UNCalendarNotificationTrigger`s from `reminders(now, now + 7 days)` (wall-clock time in the reminder's IANA zone; at most 64 waiting, the system's limit), rescheduled after each refresh, a settings change, a time zone change and a wake; what fired is marked shown on the next pass, what was never scheduled comes from `due_reminders` at once; with reminders off or not allowed at launch, what came due shows as a catch-up card on This Week. A student who says yes in both the Mac app and the Tauri app on one data folder may get a reminder twice (the Tauri timer can show one before the Mac marks it after it fired); with consent in one app only, never.
- **AI tab [M3]** (model-access design §7; the Tauri app's Settings → AI models, output language and usage), preview builds until shipped: the student's models in priority order (state, problems, key ending, the one-line data policy; Review and Turn On… / What's Shared…, Replace Key…, Remove…); Add an API Key… (one sheet: provider, address where the preset allows it, key; the facade checks the key with a free call); models on this computer (Ollama, LM Studio; Use, Look Again; nothing is downloaded); which model does what (model, effort, Test); the disclosure sheet before a backend's first run (all its facts, age and free-tier confirmations, turned on per facts version); the monthly budget for API keys; the answers' language; **Weekly note**: "Prepare my weekly note when I open PageLamp on Monday" (a switch; offered while the note's model is a provider: an API key, a model on this computer, or a cloud model through Ollama; shown "Paused: …" when on but the model no longer allows it, and it can still be turned off; hidden otherwise), its hint what Monday's note costs (on this computer by the chosen model's facts, else by the backend's kind; no price; or "≈ $x" each Monday); usage per month; Remove All AI Data…. **The AI settings are shared with the Tauri app**: both read and write the same database and keychain entries through the facade, so a key, a model choice, a budget, an acknowledgement or Monday's weekly note set in one app shows in the other (only one app prepares the note: the facade records the try). The key goes from the `SecureField` straight to `add_model_provider` / `update_model_provider_key`; the field is cleared on submit and on cancel, and nothing else (a model, `UserDefaults`, logs, snapshots) ever holds it; only its last 4 characters are shown. Ad-hoc builds are signed anew on every rebuild, so macOS may ask again for keychain access to the stored keys; the signed preview build is the one to test with. Not on the Mac: the ChatGPT plan card (the facade doesn't offer the plan in this build; its usage rows and weekly runs aren't shown either).
- **Data** [M1]: `settings.data.*`; paths with Copy Path / Show in Finder; counts from `status().counts` (readable = `indexed_materials`), mono right-aligned.

```text
W8c · Settings ▸ Privacy
╭────────────────────────────────────────────────────────────╮
│   General   Reminders    Data  [Privacy]    Help           │
├────────────────────────────────────────────────────────────┤
│  How your AI app uses your courses                         │
│  When you ask your AI app about a course, it … (full)      │
│  ✓ You confirmed this on Sep 26, 2026.                     │
│  AI access by course                                       │
│  ┌───────────────────────────────────────────────────────┐ │
│  │ Course   AI policy          AI reads  State           │ │
│  │ DEMO101  ▪ Learning aid only  (━●)   Readable         │ │
│  │ DEMO310  ▪ Not set            (○━)   Off              │ │
│  │ DEMO099  ▪ No AI              (○━)   Withheld (No AI) │ │
│  └───────────────────────────────────────────────────────┘ │
│  Hidden courses are never shown to your AI app.            │
│  ▪ Tokens and feed addresses live in your keychain. …      │
╰────────────────────────────────────────────────────────────╯
```

- **Privacy** [M1 text · M2 table]: full disclosure + date (or `notAcknowledged`). **AI access by course** / 各门课的 AI 读取设置 `Table`: Course · AI policy (neutral) · "AI reads" switch (label = the full sentence) · State. No AI rows disabled, "Withheld (No AI)" / "不共享（禁止使用 AI）"; hidden rows only while shown. Footer "Hidden courses are never shown to your AI app." / "隐藏的课程永远不会提供给你的 AI 应用。" Then `privacy.keychain`, `readOnly`, `aiProvider`, and a Mac `perCourse` pointing at the table and the inspector.

```text
W8d · Settings ▸ Help and the Diagnostic Report sheet
╭────────────────────────────────────────────────────────────╮
│   General   Reminders    Data   Privacy   [Help]           │
├────────────────────────────────────────────────────────────┤
│  [Copy Diagnostic Report…]  Version numbers, sync status,  │
│                             recent log lines.              │
│  [Open Logs Folder]  [Report a Problem on GitHub ↗]        │
├────────────────────────────────────────────────────────────┤
│  Diagnostic Report                         (sheet)         │
│  Check that nothing private is in it before sharing. …     │
│  ┌────────────────────────────────────────────────┐        │
│  │ pagelamp 0.3.0-beta.1 · macOS 27.2 (arm64)     │        │
│  │ sources: canvas(auth_expired) folder(ok) …     │        │
│  └────────────────────────────────────────────────┘        │
│                            [Done]  [[Copy Report]]         │
╰────────────────────────────────────────────────────────────╯
```

- **Help** [M1]: Copy Diagnostic Report… always previews first (same sheet from the Help menu, S6, S2; works without the DB). **About** [M1]: standard panel; version `status().version` (else `doctor().version`); tagline; Apache-2.0; links.

### 3.6 Welcome window [M2] (680 × 620 pt, hidden title bar, 520 pt column)

```text
W9a · Welcome, step 1 (680 × 620 pt, hidden title bar)
╭──────────────────────────────────────────────────────────────╮
│ ● ● ●                                       [ English   ▾ ]  │
│               ░░░░░░░░░░░    ▪    ░░░░░░░░░░░                │
│                    Welcome to PageLamp                       │
│              A reading lamp for your courses.                │
│         Welcome  ·  Add a source  ·  First sync              │
│   PageLamp keeps your courses — materials, weekly structure  │
│   and deadlines — on your computer, … PageLamp never solves  │
│   or submits assignments for you.                            │
│   ┌────────────────────────────────────────────────────────┐ │
│   │ ▪ How your AI app uses your courses                    │ │
│   │ When you ask your AI app about a course, it reads that │ │
│   │ course's materials from PageLamp and sends them to     │ │
│   │ your AI provider under your own account. … (full text) │ │
│   │ [ ] I understand                                       │ │
│   └────────────────────────────────────────────────────────┘ │
│  ((Skip for Now))                      (( Get Started → ))   │
│                       Tick "I understand" to continue.       │
╰──────────────────────────────────────────────────────────────╯
```

```text
W9b step 2 · W9c first sync running · W9d done
╭──────────────────────────────────────────────────────────────╮
│         Welcome ✓  ·  Add a source  ·  First sync            │
│   Where are your courses?                                    │
│   [ tiles, Canvas notice and fields exactly as W6a / W6b ]   │
│  ((← Back))                        (( Add and Continue → ))  │
├──────────────────────────────────────────────────────────────┤
│   Syncing your courses                                       │
│                ░░░░░░░░    ▪    ░░░░░░                       │
│   ◌ Demo Canvas                        Syncing · 3 of 8      │
│     ━━━━━━━━━━━━━━━━━━━━━━──────────────────────────         │
│   ○ LMS calendar                                Waiting      │
│  ((Continue in the Background))                              │
├──────────────────────────────────────────────────────────────┤
│   Your courses are ready                                     │
│        4              58               23                    │
│     Courses       Materials     Deadlines and events         │
│   ▪ Canvas files aren't downloaded automatically, because…   │
│   [ ] Show PageLamp in the menu bar   [ ] Open at login      │
│  ((Go to My Courses))            (( Connect Your AI App → )) │
╰──────────────────────────────────────────────────────────────╯
```

```swift
.scrollEdgeEffectStyle(.soft, for: .bottom)
.safeAreaBar(edge: .bottom) {
  GlassEffectContainer {                                  // sibling glass buttons; no slab behind them
    HStack {
      Button(secondaryTitle, action: secondary).buttonStyle(.glass)
      Spacer()
      Button(primaryTitle, action: primary).buttonStyle(.glassProminent).keyboardShortcut(.defaultAction)
    }.padding(.horizontal, 20).padding(.bottom, 16)
  }
}
```
- Step text "Welcome · Add a source · First sync" / "欢迎 · 添加数据来源 · 首次同步" (current Semibold; done ✓ + "(done)"). Language `Picker(.menu)`, live. Steps `.push(from: .trailing)` `.smooth(duration: 0.4)` (crossfade under Reduce Motion); focus to the heading (`@AccessibilityFocusState`); titles crossfade (no glass-ID morph on button-style glass).
- **Step 1:** Get Started `.disabled(!acknowledged)` with "Tick "I understand" to continue." / "请先勾选"我已了解"，再继续。" visible while disabled (also its hint). **Skip for Now** / 暂时跳过 needs no acknowledgement; adding a source later shows the disclosure gate (§3.3).
- **Step 2:** the sheet's tiles and forms, identical Canvas notice; **Add and Continue** / 添加并继续.
- **Step 3:** per-source progress; **Continue in the Background** / 在后台继续 → main window, S5 (*new* `mac.onboarding.backgroundHint`: "…its progress shows at the bottom of the main window"); "Almost there" / 就差一点了; "The sync didn't run" / 这次没能同步 + **Try Again**; done → counts roll in, `canvasFilesNote`, **Connect Your AI App** / 连接 AI 应用 primary; [M3] opt-ins, **unticked**.

### 3.7 Menu bar extra [M3] (`.menuBarExtraStyle(.window)`, 340 pt)

```text
W10 · Menu bar extra (340 pt), with attention and empty variants
╭────────────────────────────────────╮
│ This Week          Sep 26 – Oct 2  │
├────────────────────────────────────┤
│ Due soon                           │
│ Today 11:59 PM                     │
│   Problem Set 2 · DEMO205          │
│ Tomorrow 9:00 AM                   │
│   Quiz 3 · DEMO101                 │
├────────────────────────────────────┤
│ Today's plan · from your AI app    │
│ ○ Skim Week 4 slides      30 min   │
├────────────────────────────────────┤
│ DEMO101  Wk 4   DEMO205  Wk 4      │
│ DEMO310  Wk 5   DEMO099 ▪ Wk 2     │
├────────────────────────────────────┤
│ ✓ Synced 2 h ago                   │
│ [Sync Now]        [Open PageLamp]  │
├────────────────────────────────────┤
│ ▪ Demo Canvas needs a new token.   │
│                  [Replace Token…]  │
├────────────────────────────────────┤
│ ▪ No courses yet · Add a source    │
│               [Set Up PageLamp…]   │
╰────────────────────────────────────╯
```

`VStack` + `Divider`; plain rows with `.fill.quaternary` hover in `ConcentricRectangle(corners: .concentric(minimum: .fixed(6)), isUniform: true)` inside `.containerShape(.rect(cornerRadius: 12))` (the extra's container shape [verify]); a row opens the main window at that course; `.bordered .controlSize(.small)` buttons; `SettingsLink`; nothing inside is glass; ≤ 5 deadlines and ≤ 4 plan items from the same digest as This Week. Variants: attention (first block, Replace Token… opens the main window + sheet), empty (Set Up PageLamp…), syncing, offline, busy. Label `lamp.desk` / `lamp.desk.fill` (§6.5), accessibility "PageLamp — 1 deadline in the next 24 hours" / "PageLamp — 24 小时内有 1 个截止日期". Its language follows `AppleLanguages`.

### 3.8 Search [M2]

`.searchable(text:placement: .toolbar, prompt: "Search courses, materials and deadlines" / "搜索课程、资料和截止日期")`, ⌘F via `.searchFocused`; results replace the detail (Courses · Materials · Deadlines); `«…»` → bold runs; a material opens its course at its week; hidden courses excluded unless shown; local only; 250 ms after the last committed keystroke (never IME marked text); none → `ContentUnavailableView.search(text:)`.

### 3.9 State catalogue

```text
S1 loading · S2 backend unavailable · S3 no sources · S4 no courses
╭──────────────────────────────────────────────────────────────────────╮
│  S1 · This Week                                                      │
│      ▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅                            │
│      ▅▅▅▅▅▅▅▅▅▅   ▅▅▅▅▅▅▅▅  ▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅▅   ▅▅▅▅▅            │
├──────────────────────────────────────────────────────────────────────┤
│                 ▪ externaldrive.badge.exclamationmark                │
│               PageLamp couldn't load your data                       │
│   The desktop app couldn't talk to its local database. Try           │
│   again, or restart PageLamp.                                        │
│   [[Try Again]]  [Copy Diagnostic Report…]  [Open Logs Folder]       │
├──────────────────────────────────────────────────────────────────────┤
│              ▪ lamp.desk (unlit)   No courses yet                    │
│         Add a source so PageLamp can find your courses.              │
│             [[Add a Source…]]     Set Up PageLamp…                   │
├──────────────────────────────────────────────────────────────────────┤
│              ▪ tray   No courses yet                                 │
│   Your sources haven't brought in any courses yet. Sync now,         │
│   or add another source.   [[Sync Now]]  [Add Source…]               │
╰──────────────────────────────────────────────────────────────────────╯
```

```text
S5 first sync · S6 crash recovered · S7 source problem
╭──────────────────────────────────────────────────────────────────────╮
│                  ░░░░░░░░   ▪   ░░░░░░░░                             │
│                     Syncing your courses                             │
│        Your courses appear here when the sync finishes.              │
│          (( ◔ Syncing 1 of 2 · Demo Canvas · 3 of 8 ))               │
├──────────────────────────────────────────────────────────────────────┤
│  ┌────────────────────────────────────────────────────────────────┐  │
│  │ ▪ PageLamp stopped unexpectedly while your AI app was using it │  │
│  │   It happened 2 hours ago. Sharing a diagnostic report helps   │  │
│  │   get it fixed.  [Copy Diagnostic Report…] [Report…]  Dismiss  │  │
│  └────────────────────────────────────────────────────────────────┘  │
├──────────────────────────────────────────────────────────────────────┤
│  ┌────────────────────────────────────────────────────────────────┐  │
│  │ ▪ Your Canvas token for Demo Canvas stopped working            │  │
│  │   It may have expired or been revoked. Add a new token to keep │  │
│  │   these courses up to date.                                    │  │
│  │          [[Replace Token…]]   Open Sources & Sync              │  │
│  └────────────────────────────────────────────────────────────────┘  │
│   Course header variant: ▪ Your access token has expired — this      │
│   course isn't updating.  [Replace Token…]                           │
╰──────────────────────────────────────────────────────────────────────╯
```

```text
S8 · Status capsule, every state (all regular glass; "│" = seam of a fused union)
╭──────────────────────────────────────────────────────────────────────────────────────────╮
│  idle ............... hidden (the sidebar footer says "Synced 2 h ago")                  │
│  syncing ............ (( ◔ Syncing 2 of 3 · Canvas ))                                    │
│  finished (4 s) ..... (( ✓ Sync finished · 3 new materials ))                            │
│  notice, fused ...... (( DEMO099 is hidden. Your AI app won't see it. │ Undo ⌘Z ))       │  ← 6 s; union
│  notice, fused ...... (( ✓ Downloaded 38 files · some skipped │ Details ))               │
│  notice, fused ...... (( ▪ Can't connect · check your internet │ Try Again ))            │
│  notice, fused ...... (( ◌ Another sync is running │ Check Again ))                      │
│  attention, split ... (( ▪ Demo Canvas needs a new token  × ))  (( Replace Token… ))     │  ← right bubble tinted
│  downloading ........ (( ◔ Downloading 12 of 40 files · DEMO205 ))                       │
╰──────────────────────────────────────────────────────────────────────────────────────────╯
```

```text
S9 No AI · S10 hidden · S11 week unknown / outside term
╭──────────────────────────────────────────────────────────────────────╮
│   ▪ No AI  ·  Materials not shared with your AI app                  │
│   ┌──────────────────────────────────────────────────────────────┐   │
│   │ ▪ Materials not shared (No AI course)                        │   │
│   │   Your AI app can still see titles, dates and deadlines      │   │
│   │   for planning, but it won't read this course's materials.   │   │
│   │                                            [AI Policy…]      │   │
│   └──────────────────────────────────────────────────────────────┘   │
│   ▪ Week 2 reading                                 Not shared        │
│   Inspector: Let my AI app read … (○━) disabled + withheld note      │
├──────────────────────────────────────────────────────────────────────┤
│   ▪ Hidden · Skipped in your course list and never shown to          │
│     your AI app.                         [Show in Course List]       │
├──────────────────────────────────────────────────────────────────────┤
│   ▪ We couldn't work out which week this course is in, so            │
│     these are the materials from the last 14 days.                   │
│                                           [[Set Term Dates…]]        │
│   ▪ Today is outside this course's term dates, so there's no         │
│     current week.                         [[Set Term Dates…]]        │
╰──────────────────────────────────────────────────────────────────────╯
```

```text
S12 quiet empties · S13 search · S14 not found / section error
╭──────────────────────────────────────────────────────────────────────╮
│   Next 7 days                                                        │
│   ▪ Nothing due in the next 7 days                                   │
│     Deadlines from your courses and calendar feed show up here.      │
│   Your study plan                                                    │
│   ▪ No study plan yet. Ask your AI app: "Make me a study plan        │
│     for the next two weeks."    [Copy the Prompt]  Connect →         │
├──────────────────────────────────────────────────────────────────────┤
│   Search: "sampling"                          ⌕ sampling  ×          │
│   Materials                                                          │
│   ▪ Week 4 slides — Sampling frames       DEMO205 · Week 4           │
│     …choose a «sampling» frame that covers…                          │
│                 ▪ No Results for "samplng"                           │
├──────────────────────────────────────────────────────────────────────┤
│                  ▪ Course not found                                  │
│   It may have been removed together with its source, or it           │
│   hasn't synced yet.                    [Back to This Week]          │
│   ▪ Couldn't load your study plan.              [Try Again]          │
╰──────────────────────────────────────────────────────────────────────╯
```

```text
S15 temporary location + quarantine · S16 offline · S17 busy
╭──────────────────────────────────────────────────────────────────────╮
│   ┌────────────────────────────────────────────────────────────┐     │
│   │ ▪ Move PageLamp to Applications before connecting          │     │
│   │   macOS is running a temporary copy of PageLamp … Quit     │     │
│   │   PageLamp, drag it into your Applications folder, open it │     │
│   │   from there, then come back to this page.                 │     │
│   └────────────────────────────────────────────────────────────┘     │
│   Copy buttons first ask "Copy anyway? It will stop working…"        │
│   If macOS blocks PageLamp … Open Anyway. Still blocked?             │
│   ┌──────────────────────────────────────────────────┐               │
│   │ xattr -dr com.apple.quarantine /Applications/…   │ [Copy]        │
│   └──────────────────────────────────────────────────┘               │
├──────────────────────────────────────────────────────────────────────┤
│   footer: ▪ Offline · synced 3 h ago                                 │
│   capsule: (( ▪ Can't connect · check your internet │ Try Again ))   │
│   Sources: Demo Canvas ▪ Can't connect · Fall term ✓ OK              │
├──────────────────────────────────────────────────────────────────────┤
│   ┌──────────────────────────────────────────────────────────────┐   │
│   │ ▪ Another sync is running                                    │   │
│   │   It may have been started from the command line. When it    │   │
│   │   finishes, check again to refresh.            [Check Again] │   │
│   └──────────────────────────────────────────────────────────────┘   │
│   sync buttons disabled with .help; Remove… disabled                 │
╰──────────────────────────────────────────────────────────────────────╯
```

| State | M | Main surface | Also in |
|---|---|---|---|
| S1 loading | 1 | blank 250 ms, then `.redacted(reason: .placeholder)` | menu bar |
| S2 backend unavailable | 1 | full window, sidebar hidden | menu bar |
| S3 / S4 empty | 1 | This Week | menu bar |
| S5 first sync | 2 | This Week + capsule | footer, menu bar |
| S6 crash recovered | 2 | first callout under each page header until dismissed | Help menu |
| S7 token expired / feed rejected | 1 (sheet 2) | This Week, course header, Sources | badge, capsule, sidebar glyph, menu bar |
| S9 No AI | 1 (edit 2) | header, callout, rows, inspector | sidebar, Contents, Privacy table, Connect popover, menu bar |
| S10 hidden · S13 search · S16 offline · S17 busy | 2 | header · detail · capsule + Sources rows · Sources callout | Hidden section · — · footer, menu bar · capsule, disabled buttons |
| S11 week unknown / outside term · S12 empties · S14 errors · S15 location/quarantine | 1 | week line · sections · detail/section · Connect | "—" in sidebar and Contents · — · — · sidebar glyph |

## 4. Visual system (values in §10)

### 4.1 Glass map (review checklist; new glass is added here with a reason)

| Surface | Implementation | Tint | M |
|---|---|---|---|
| sidebar, toolbar items, search, Settings tabs, inspector, sheets, popovers, dialogs, menus | system | system; contents plain | 1 |
| Sync All | `.glassProminent` + `.sharedBackgroundVisibility(.hidden)` | only when it wins §3.0 | 1 |
| status capsule · neutral bubble | `.glassEffect(.regular.interactive(), in: .capsule)`; bubble joins `glassEffectUnion(id: "status")` in notice states | none | 1 · 2 |
| fix bubble | `.glassEffect(.regular.tint(.accentColor).interactive(), in: .capsule)`, never unioned | **tinted** | 1 |
| sidebar selection capsule | `NSGlassEffectView` `.regular`, radius h/2, Core Animation spring (§2.3, §5) | none | 1 |
| section picker thumb | `NSGlassEffectView` `.regular`, radius 4, Core Animation `sectionSpring` (§3.2.1, §5) | none | 1 |
| week scrubber · return capsule | `.glassEffect(.regular.interactive(), in: .capsule)`, IDs `"scrubber"`, `"return"` | none | 2 |
| welcome step bar | sibling `.buttonStyle(.glass)` / `.glassProminent` in one container, no slab | primary | 2 |
| menu bar extra | `.menuBarExtraStyle(.window)`; nothing inside is glass | system | 3 |

Capsule, bubbles, scrubber and return capsule share **one** `GlassEffectContainer` (`AccessoryBar`). **Never glass:** lamp band, headers, callouts, code, rows, Contents, tiles, forms, tables, empty states, term strip, day ribbon (the one exception on content: the section picker's thumb on the lamp band, as the system segmented control draws its own thumb with glass, §3.2.1).

### 4.2 Colour (roles; values in §10.1)

- **macOS uses system colours** for page backgrounds (none set), `Form(.grouped)`, fills (`.fill.tertiary/.quaternary/.secondary`), text (`.primary/.secondary/.tertiary`) and `.separator`; our dynamic `PLColor` (light, dark and increased-contrast variants) only for lamp wash/rule and success/warning/danger glyphs. The web uses the `color.*` tokens.
- **Accent = the student's system accent.** No root `.tint` (so 27's accent-coloured sidebar icons stay theirs); the brand colour reaches the Mac only as the `AccentColor` asset (used when the accent is multicolor), which needs `actool` — at the latest in M3. Tauri uses the brand accent.
- **Status colours only on glyphs**, words stay `.primary`; the light warning glyph (3.72:1) is never text. **Policies are never colour-coded.** Accent only on the one tinted action, the selected tile stroke, links, focus, selection. Lamplight only as the wash or the RT/IC 3 pt rule (hue 78, apart from warning 55).
- **Text on the pool:** web `text.secondary` is 5.21:1 (light, 28 %) and 5.68:1 (dark, 17 %). On the Mac check `.secondary` with Accessibility Inspector on 26 and 27; if < 4.5:1 lower the `lamp.wash` peak, never the text colour.

### 4.3 Typography (HIG macOS text styles; no Dynamic Type on macOS)

| Role | Style | pt | Weight / notes |
|---|---|---|---|
| Today's date, welcome title | Large Title | 26/32 | Regular; `.fontDesign(.serif)` only for these two, only in Latin locales [M3]; zh sans Semibold |
| Course name | Large Title | 26/32 | Semibold, always sans |
| Week line, onboarding counts | Title 1 | 22/26 | Regular (Semibold when lit); `.monospacedDigit()`, `.numericText(value:)` |
| Section title · Contents week | Title 3 | 15/20 | Semibold (`.isHeader`) · Regular mono digits |
| Callout title, "Next up" | Headline | 13/16 | Bold |
| Body, row titles | Body | 13/16 | Regular; Semibold when due today |
| Meta · eyebrow, footer | Callout 12/15 · Subheadline 11/14 | — | `.secondary` · eyebrow Semibold |
| Code, paths | Callout monospaced | 12/15 | SF Mono, selectable |

Footnote and Caption (10 pt) are **never used**, which also gives the CJK floor of 11 pt. All dates, times, weeks, counts and progress use `.monospacedDigit()`. zh-CN: no serif, italics, tracking or case transforms; multi-line paragraphs `.lineSpacing(2)`; emphasis is weight only.

### 4.4 Geometry and elevation

Spacing on a 4 pt grid (§10.2). Radii: windows, sheets, popovers, controls, sidebar, inspector are **system** (never hard-coded); ours are callouts/tiles 12 (`.containerShape(.rect(cornerRadius: 12))`), row fills 8, inner shapes `ConcentricRectangle(corners: .concentric(minimum: .fixed(6)), isUniform: true)`, capsules `.capsule`; rule `inner = max(min, outer − padding)`; AppKit on 27: `NSViewCornerConfiguration(.containerConcentric)`. Elevation: page → flat fills (no content shadows) → glass chrome → presentations (system).

### 4.5 Icons (SF Symbol ↔ lucide; all verified, Appendix B)

Mac: `.symbolRenderingMode(.hierarchical)` in content. Web: `<LucideProvider strokeWidth={1.75} absoluteStrokeWidth>`.

| Concept | SF Symbol → lucide |
|---|---|
| This Week / app · lamp lit | `lamp.desk` → `LampDesk` · `lamp.desk.fill` → `LampDesk` + 6 px dot |
| courses · course | `book.pages` → `BookOpenText` · `book.closed` → `BookText` |
| policies: No AI · not set · learning aid · citation · no restrictions | `hand.raised` → `Hand` · `questionmark.circle` → `CircleQuestionMark` · `lightbulb` → `Lightbulb` · `quote.opening` → `Quote` · `checkmark.circle` → `CircleCheck` |
| hidden / shown · past course | `eye.slash` → `EyeOff` / `eye` → `Eye` · `clock.arrow.circlepath` → `History` |
| sources · sync · connect · settings | `folder.badge.gearshape` → `FolderSync` · `arrow.triangle.2.circlepath` → `RefreshCw` · `cable.connector` → `Cable` · `gearshape` → `Settings` |
| OK · not synced · attention · failure | `checkmark.circle(.fill)` → `CircleCheck` · `circle.dashed` → `CircleDashed` · `exclamationmark.triangle` → `TriangleAlert` · `xmark.octagon` → `OctagonX` |
| token · secret · privacy · personal use | `key` → `KeyRound` · `lock` → `LockKeyhole` · `lock.shield` → `ShieldCheck` · `person.crop.circle.badge.checkmark` → `UserRoundCheck` |
| Canvas · folder · add folder · feed · deadline | `graduationcap` → `GraduationCap` · `folder` → `Folder` · `folder.badge.plus` → `FolderPlus` · `calendar` → `CalendarDays` · `calendar.badge.clock` → `CalendarClock` |
| countdown · final 2 h | `clock` → `Clock` · `clock.badge.exclamationmark` → `ClockAlert` |
| file · page · syllabus · announcement · link · modules | `doc.text` → `FileText` · `doc.richtext` → `StickyNote` · `text.book.closed` → `BookMarked` · `megaphone` → `Megaphone` · `link` → `Link2` · `square.stack.3d.up` → `Layers` |
| waiting · not downloaded · can't be read · download | `hourglass` → `Hourglass` · `arrow.down.circle` → `CircleArrowDown` · `minus.circle` → `CircleMinus` · `square.and.arrow.down` → `Download` |
| website · in browser · copy · terminal · config | `safari` → `Compass` · `arrow.up.forward.app` → `ExternalLink` · `doc.on.doc` → `Copy` · `terminal` → `SquareTerminal` · `curlybraces` → `Braces` |
| plan · prev / next / back to now | `list.bullet.clipboard` → `ClipboardList` · `chevron.backward` → `ChevronLeft` / `chevron.forward` → `ChevronRight` / `arrow.uturn.backward` → `Undo2` |
| sidebar · inspector · search · info | `sidebar.leading` → `PanelLeft` · `sidebar.trailing` → `PanelRight` · `magnifyingglass` → `Search` · `info.circle` → `Info` |
| diagnostics · bug · offline · DB error · empty · remove · data | `stethoscope` → `Stethoscope` · `ladybug` → `Bug` · `wifi.slash` → `WifiOff` · `externaldrive.badge.exclamationmark` → `DatabaseZap` · `tray` → `Inbox` · `trash` → `Trash2` · `internaldrive` → `HardDrive` |
| language · reminders · help | `globe` → `Globe` · `bell` → `Bell` · `questionmark.circle` → `CircleQuestionMark` |

**Empty states:** one SF Symbol at 44 pt (hierarchical, `.tertiary`) in `ContentUnavailableView`, no illustrations. The lamp is the exception: `lamp.desk` 56 pt on a pool on the welcome window and S5, **unlit** in S3. Errors never glow.

**App icon [M3]** (Icon Composer `.icon`, plus a pre-masked squircle `.icns` because a non-squircle legacy icon gets a grey plate on 26+): blue-hour gradient oklch(0.30 0.05 255 → 0.22 0.04 258); an open book (0.97 0.01 90, opaque); a warm pool (lamplight, 70 %) on the left page; the lamp head as the **only Liquid Glass layer**. Check tinted and clear variants in the 26 and 27 previews; flatten for Windows `.ico` and Linux PNGs. Compiling `.icon` needs `actool` (Xcode).

## 5. Motion

Presets (§10.4): **calm** `.smooth(duration: 0.45)` · **quick** `.snappy(duration: 0.3)` · **lively** `.bouncy` (only the one "Sync finished" bounce) · content `.smooth` 0.35 s (week), 0.25 s (section), 0.4 s (onboarding step). Under Reduce Motion everything becomes a 0.2 s crossfade or appears instantly (glass transitions `.identity`, no `numericText` rolling, no symbol effects).

| Element | macOS |
|---|---|
| capsule in/out · width/text | `glassEffectTransition(.materialize)`; quick; `.numericText()` counts |
| notice fuse · attention split · return capsule | union id on/off; `.glassEffectTransition(.matchedGeometry)` [verify; fallback `.materialize`]; quick |
| scrubber selection · lamp on/off | fill `matchedGeometryEffect`, quick · wash opacity 0↔1 + `endRadiusFraction` 0.6→0.75, calm |
| week change · section · step | `.push(from: .trailing/.leading)` 0.35 s + `.numericText(value:)` numeral · crossfade 0.25 s · `.push(from: .trailing)` 0.4 s |
| first-sync pool · row hover | opacity follows progress (calm) · `.easeOut(duration: 0.15)`, none under Reduce Highlighting Effects |
| copied · sync finished · ignite | `.symbolEffect(.replace)` 2 s · `.symbolEffect(.bounce, value:)` once · `PhaseAnimator` (§6.6) |
| sidebar selection | additive `CASpringAnimation(perceptualDuration: 0.3, bounce: 0.15)` (`PLMotion.quickSpring`), started after the page's first drawn frame (§2.3); Reduce Motion: jump |
| section picker thumb | additive `CASpringAnimation(perceptualDuration: 0.25, bounce: 0)` (`PLMotion.sectionSpring`), started in the input's own handler (a model change: its update; a cross-link: the new page's first frame), no gate (§3.2.1); Reduce Motion: jump |
| glass press, sidebar column, inspector, sheets | system (27 adds a click bounce to interactive glass) |

Never animated: blur radius, the minute countdown, compliance text; no idle loops.

## 6. Signature moments

### 6.1 One light: the lamp band [M1]

The band holding *now* is the **first view of the page's `ScrollView`, full-bleed across the detail column, and ignores the top safe area**, so it touches the toolbar, sidebar and inspector edges and `backgroundExtensionEffect` mirrors the warm light under their glass (the Landmarks hero pattern). Leaving the current week turns it off, glow under the sidebar included.

```swift
struct LampWash: View {                                   // content layer, decorative
  var lit: Bool
  @Environment(\.colorScheme) private var scheme
  var body: some View {
    EllipticalGradient(colors: [PLColor.lampWash.opacity(scheme == .dark ? 0.17 : 0.28), .clear],
                       center: UnitPoint(x: 0.06, y: 0), startRadiusFraction: 0, endRadiusFraction: lit ? 0.75 : 0.6)
      .opacity(lit ? 1 : 0)
  }
}
struct LampBand<Content: View>: View {
  var lit: Bool
  @ViewBuilder var content: Content
  @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
  @Environment(\.colorSchemeContrast) private var contrast
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  private var flat: Bool { reduceTransparency || contrast == .increased }
  var body: some View {
    VStack(alignment: .leading, spacing: 8) { content }
      .padding(.leading, flat && lit ? 12 : 0)
      .overlay(alignment: .leading) { if flat && lit { Rectangle().fill(PLColor.lampRule).frame(width: 3) } }
      .frame(maxWidth: 760, alignment: .leading).padding(.horizontal, 40).padding(.vertical, 24)  // text: reading column
      .frame(maxWidth: .infinity)                                                                  // band: FULL-BLEED
      .background(alignment: .topLeading) {
        LampWash(lit: lit && !flat)
          .backgroundExtensionEffect(isEnabled: !flat)   // [26] on the wash only, never on the text
          .ignoresSafeArea(edges: .top)                  // under the toolbar
          .accessibilityHidden(true)
      }
      .animation(reduceMotion ? nil : .smooth(duration: 0.45), value: lit)
  }
}
```
One pool per screen by construction; always next to a word ("Today", "This week" / 今天, 本周); dark peak raised from 10 % to 17 %. [verify on 26 and 27: the wash reaches under the toolbar and mirrors under sidebar and inspector; fallback: move `.ignoresSafeArea(edges: .top)` to the `ScrollView` and pad the band's text by the top safe-area inset.]

### 6.2 One floating accessory bar: the lamp switch [M1; fuse/split M2]

The capsule materialises when a sync starts ("◔ Syncing 2 of 3 · Canvas", determinate ring), shows "✓ Sync finished · 3 new materials" for 4 s, and leaves. A **notice** with a neutral follow-up (Undo ⌘Z, Details, Try Again, Check Again, Sync Now) adds a second capsule **fused** by `glassEffectUnion`. **Attention** (a fix the student must make) releases the union and buds off the tinted **fix bubble**; the capsule gets an inline × (plain button) and Esc dismisses. A click opens a popover with per-source progress. It replaces every Tauri toast; VoiceOver hears start, end and new problems only.

```swift
struct AccessoryBar: View {                               // in .safeAreaBar(edge: .bottom)
  var showsScrubber: Bool
  @Environment(AppModel.self) private var model
  @Namespace private var glass
  var body: some View {
    GlassEffectContainer(spacing: 12) {                   // the window's only custom container
      VStack(spacing: 14) {
        StatusCapsule(state: model.capsule, ns: glass)
        if showsScrubber { WeekScrubber(ns: glass, weeks: model.weeks, current: model.currentWeek, total: model.totalWeeks) }
      }
    }.padding(.bottom, 12)
  }
}
struct StatusCapsule: View {
  let state: CapsuleState; var ns: Namespace.ID           // .hidden when idle; Equatable
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @Environment(\.appearsActive) private var appearsActive
  var body: some View {
    let fused = state.isNotice
    HStack(spacing: 14) {                                  // resting gap 14 > container spacing 12: separate unless unioned
      if state.isVisible {
        Button(action: state.showDetails) { CapsuleLabel(state: state) }.buttonStyle(.plain)
          .glassEffect(.regular.interactive(), in: .capsule)
          .glassEffectID("status", in: ns)
          .glassEffectUnion(id: fused ? "status" : nil, namespace: ns)
          .glassEffectTransition(reduceMotion ? .identity : .materialize)
          .accessibilityAddTraits(.updatesFrequently)
      }
      if let neutral = state.neutralAction {              // same Glass + same shape → may join the union
        Button(action: neutral.run) { Text(neutral.title).padding(.horizontal, 14).padding(.vertical, 8) }.buttonStyle(.plain)
          .glassEffect(.regular.interactive(), in: .capsule)
          .glassEffectID("status.action", in: ns)
          .glassEffectUnion(id: fused ? "status" : nil, namespace: ns)
      }
      if let fix = state.fixAction {                      // the one tinted control; never in a union
        Button(action: fix.run) { Text(fix.title).fontWeight(.semibold).padding(.horizontal, 14).padding(.vertical, 8) }.buttonStyle(.plain)
          .glassEffect(.regular.tint(.accentColor).interactive(), in: .capsule)
          .glassEffectID("status.fix", in: ns)
          .glassEffectTransition(reduceMotion ? .identity : .matchedGeometry)
      }
    }
    .frame(maxWidth: 520)
    .opacity(appearsActive ? 1 : 0.6)
    .animation(reduceMotion ? .easeInOut(duration: 0.2) : .snappy(duration: 0.3), value: state)
  }
}
```
Whether a union may mix tinted and untinted glass is unverified, so the fix bubble never joins one. Tune gaps on device [verify]. Fallback on misbehaving 26.x builds: one capsule with an inline button.

### 6.3 Week scrubber and the adjustable week line [M2]

On a course's This Week section the bar also holds a glass scrubber: all available weeks at a glance, the selected one filled, *now* marked. Scrub away and "↩ This Week" splits off; return and it merges back. The toolbar ‹ › and ⌘[ / ⌘] remain the command path.

```swift
struct WeekScrubber: View {
  var ns: Namespace.ID
  @Environment(CourseUIState.self) private var ui
  let weeks: [UInt32]; let current: UInt32?; let total: UInt32?          // available_weeks, current_week, total_weeks (§13)
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  var body: some View {
    HStack(spacing: 14) {
      ViewThatFits(in: .horizontal) { WeekStrip(weeks: weeks, current: current); CompactWeekStepper() }  // ‹ 1 2 [3] … › | ‹ Week 5 of 12 ›
        .padding(.horizontal, 8).frame(height: 36)
        .glassEffect(.regular.interactive(), in: .capsule)
        .glassEffectID("scrubber", in: ns)
        .accessibilityElement(children: .contain)       // NOT .ignore: every week stays reachable by Tab
        .accessibilityLabel(Text("course.week.switcherLabel"))
      if let current, ui.selectedWeek != nil, ui.selectedWeek != current {
        Button("This Week", systemImage: "arrow.uturn.backward") { ui.selectedWeek = nil }
          .buttonStyle(.plain).padding(.horizontal, 12).frame(height: 36)
          .glassEffect(.regular.interactive(), in: .capsule)
          .glassEffectID("return", in: ns)
          .glassEffectTransition(reduceMotion ? .identity : .matchedGeometry)
      }
    }
  }
}
```
Week buttons: plain, mono, ≥ 26 pt; selected = `.fill.secondary` capsule inset 4 (a fill on glass); current = 4 pt `.primary` dot; empty weeks `.tertiary`; label "Week 5" / "第 5 周", value "this week" / "selected", `.isSelected`. Unknown week: a trailing "Recent" item. **VoiceOver:** the header week line is the one adjustable element (`.accessibilityAdjustableAction` steps `available_weeks`: "Week 5 of 12, October 6 to 12, in 2 weeks"); the scrubber keeps real buttons for Full Keyboard Access.

### 6.4 The Contents page [M1; leaders and rolling numerals M3]

"Your courses" reads like a book's contents page: `DEMO205  Foundations of Sample Data ········· Week 4`. Code Subheadline Semibold; name Body (one line, original language, `.help`); 1 pt `.tertiary` dotted leader (`Path`, `StrokeStyle(lineWidth: 1, dash: [1, 3])`); week right-aligned, Title 3 mono. Second line: next deadline and "▪ Learning aid only · 12 of 14 readable" (No AI → `hand.raised` "No AI · materials not shared"; unknown → "—" + `courses.card.setTerm`; hidden → `.secondary` + `eye.slash`; past courses last). Rows are plain Buttons with a radius-8 hover fill, the course context menu and one combined VoiceOver element.

### 6.5 Quiet countdown and the menu bar lamp

**Next up:** only when a deadline is ≤ 24 h away; `TimelineView(.everyMinute)`; minutes, never seconds; no red, no pulse; under 2 h only the glyph changes (`clock` → `clock.badge.exclamationmark`); never re-announced. **Menu bar lamp [M3]:** `lamp.desk.fill` while something is due within 24 h; no count, no blink; can be turned off.

### 6.6 Welcome ignite [M3]

```swift
PhaseAnimator([0.0, 1.15, 1.0], trigger: appeared) { glow in LampHalo().opacity(glow) } animation: { _ in .smooth(duration: 0.3) }
```
Once, on first appearance; skipped under Reduce Motion; a content gradient, not glass.

## 7. Accessibility and localization

### 7.1 VoiceOver and keyboard

- **Contents row** (combined): "DEMO101, Intro to Demo Studies. Week 4, this week. Next: Quiz 3, Saturday 9 AM. AI policy: Learning aid only. 12 of 14 materials readable by your AI app." **Sidebar course row:** "DEMO099, Orientation Placeholder, week 2, No AI" (+ ", source needs attention"). **Sidebar:** "Sidebar, list"; rows are static text with "selected" on the current destination, headers are headings, Sources & Sync's value "1 source needs attention"; ↑/↓ and type-select announce the new row (pointer, menu and VoiceOver presses don't), and so does keyboard focus entering the list; the capsule is hidden (§2.3). **Deadline row:** full date — "Problem Set 2, questions 1 to 3. DEMO205, Assignment. Due today at 11:59 PM, in 58 minutes."
- **Section picker** (§3.2.1): "Course sections", a tab group of three tabs ("This Week, selected, tab, 1 of 3"; four with Explain [M3], "1 of 4"); focus on the key segment, VO-Space selects; the glass thumb is not an element.
- **Capsule:** "Sync status: syncing 2 of 3, Canvas", hint "Shows details"; `AccessibilityNotification.Announcement` on start, finish, new problems and "Copied" only. **Week line [M2]:** adjustable, announces the new week. Every icon-only toolbar item is labelled.
- Section and day headings `.isHeader`; rotors **Deadlines** (This Week) and **Materials** (course); lamp wash, leaders and ribbon dots `.accessibilityHidden(true)`; plan items static ("…, 30 minutes, not done yet"); the disabled No AI switch's note is visible text and its hint.

**Mixed languages [verify]:** backend English (evidence, progress, warnings) is an `AttributedString` with `languageIdentifier = "en"`, mirroring Tauri's `lang="en"`. It is unverified that SwiftUI `Text` passes this to VoiceOver; test in zh-CN, and if the voice does not switch, render those strings through an `NSViewRepresentable` whose attributed string carries AppKit's accessibility language attribute (`NSAccessibilityLanguageTextAttribute`; [verify the Swift spelling]).

**Keyboard:** everything reachable with Full Keyboard Access (each scrubber week, the capsule's buttons after the content); Tab/⌃F6 cycle panes; sidebar: one stop; ↑/↓ (⇧⌃⌘ alike), ⌥↑/↓ first/last, Home/End scroll, type-select, holding ↑/↓ previews (§2.3); section picker: a stop only with Full Keyboard Access on (AppKit's `NSControl` rule), ←/→ move the key segment (wrap), Space selects (§3.2.1); lists ↑/↓, Return, Space [M3], ⌘C; sheets `.defaultFocus` on the first field, Return default, Esc cancel, destructive dialogs default to Cancel; Esc dismisses the capsule's attention state.

### 7.2 System settings honoured

| Setting | Effect |
|---|---|
| Reduce Transparency | system glass frosts itself (the sidebar capsule and the section picker's thumb too: system glass); lamp → 3 pt rule + label; `backgroundExtensionEffect(isEnabled: false)` |
| Increase Contrast | system glass goes black/white (the sidebar capsule's edge from system glass [verify]); the section picker's track 14.9 % and labels 100 % ink; dynamic colours use IC variants; full-strength separators; callouts get 1 pt borders; lamp → rule |
| Show Borders (`\.accessibilityShowBorders`, honoured on 27) | borderless content buttons → `.bordered`; callouts, footer, plain rows get 1 pt `.separator` outlines; the sidebar capsule gets a 1 pt outline; the section picker's track (not its thumb) a 1 pt 12.5 % ink outline |
| Reduce Motion | §5; `\.accessibilityPrefersCrossFadeTransitions` [26.4] where available; the sidebar capsule and the section picker's thumb jump |
| Reduce Highlighting Effects (`\.accessibilityReduceHighlightingEffects` [26.4], gated) [M2] | no hover fills; focus rings and selection remain |
| Differentiate Without Color | already met (glyph + words everywhere; the lamp has a label) |
| Inactive window | `@Environment(\.appearsActive)`: the accessory bar dims to 60 %; sidebar icons `.secondary`, no focus ring |

### 7.3 Localization

- **One source:** the i18next JSON. `scripts/gen-apple-strings.mjs` emits `en.lproj` / `zh-Hans.lproj` `.strings` + `.stringsdict` (`_one`/`_other` → plurals, `{{x}}` → `%1$@`, `{{product}}` pre-substituted) and a typed `L10n.swift`; CI fails on a diff; `CFBundleLocalizations = [en, zh-Hans]`.
- **`mac.*` namespace** in the same files for Mac-only strings and HIG title-case English actions ("Sync Now", "Add Source…", "Replace Token…", "Save AI Policy", "Show in Finder"); the existing `i18n.test.ts` parity test covers them. Tauri keeps sentence case; zh-CN is identical in both.
- Swift maps codes (`SourceErrorKind`, `AppErrorKind`, `McpNoteCode`, `WeekNoteKind`, `AiMaterialsState`, `TextStatus`, `EventKind`), never English. Backend English is shown verbatim only where Tauri shows it, tagged English.
- Dates via `Date.FormatStyle` / `IntervalFormatStyle` / `RelativeFormatStyle` in the active locale and calendar ("9月26日 星期五", 24-hour where the locale uses it, "58分钟后"). Full-sentence keys only, never fragments; "·" is safe in both languages; zh uses full-width "：" "，".
- No uppercase, small caps, italics or tracking. Nothing below 11 pt. No fixed text widths: sidebar 200 pt fits "数据来源与同步" + its count, and trailing text never wraps (the title truncates first); compliance text wraps (`.fixedSize(horizontal: false, vertical: true)`), never truncates.
- IME: search on committed text only; test search and token fields with Pinyin.
- **Language switching, stated accurately:** view content switches live; SwiftUI commands, standard menus, system dialogs and the menu bar extra follow `AppleLanguages` and switch after **Reopen Now**.

New keys are marked *new* where they are introduced in §3; Mac-only ones start with `mac.`.

```text
Z1 · zh-CN, sidebar and This Week (East-Asian width 2)
╭──────────────────────┬────────────────────────────────────────────────────────────────╮
│ ● ● ●          [|] ░░│░░ 本周 ░░░░░░░░░░░░░░░░░░░░░░░░░░░         ↻   ⌕ 搜索          │
│ [▪ 本周          ]░░░│░░░░ 9月26日 星期五                                             │  ← sans Semibold; no serif for zh
│ 课程               ░ │░░░░ 未来 7 天有 3 个截止日期 · 今天有 2 项学习任务             │
│  ▪ DEMO101  第 4 周  │░░░░ ▪ 接下来  Problem Set 2 · DEMO205                          │
│  ▪ DEMO205  第 4 周  │░░        今天 23:59 截止 · 还有 58 分钟   [打开课程]           │  ← 24-hour clock from locale
│  ▪ DEMO099  第 2 周  │                                                                │
│ 配置                 │     未来 7 天                          3 个截止日期            │
│  ▪ 数据来源与同步  1 │     今天      23:59  Problem Set 2      DEMO205  作业          │  ← 200 pt sidebar fits
│  ▪ 连接 AI 应用      │     明天      09:00  Quiz 3             DEMO101  小测          │
│                      │                                                                │
│                      │     我的课程                                                   │
│                      │     DEMO101  Intro to Demo Studies ············ 第 4 周        │  ← names stay in their language
│ ✓ 2 小时前同步       │              仅限辅助学习 · 共 14 份资料，AI 应用可读取 12 份  │  ← footer 11 pt (CJK floor)
╰──────────────────────┴────────────────────────────────────────────────────────────────╯
```

### 7.4 Test matrix (definition of done per screen)

Light/dark × normal/Increase Contrast × Reduce Transparency, on 26.x and 27.x; VoiceOver in en and zh-CN; Full Keyboard Access; Reduce Motion; Show Borders (27); Reduce Highlighting (26.4+); zh-CN at 760 × 520 and at the default size; `-NSDoubleLocalizedStrings YES`; a +35 % pseudo-locale; deuteranopia/protanopia simulation of This Week and Sources; the glass-lint grep and a §4.1 screenshot review.

## 8. Windows and Linux (Tauri 2 + React 19 + Tailwind v4 + shadcn)

**Share the language, not the chrome.** Shared: two layers, one light, type-first hierarchy, concentric geometry, the accessory bar (capsule, scrubber), the springs, one tinted action, icon meanings, compliance copy. Native: title bar and caption buttons, Snap Layouts, window corners and shadow (Windows 8 px, 0 when snapped), dialogs, notifications, tray menu, scrollbars, OS theme and accessibility, Ctrl shortcuts. Never: traffic lights, glass on the sidebar (Microsoft: opaque vertical panes; Mica showing through is fine), SVG refraction, animating a blur radius.

**Backdrop, decided in Rust at startup [M1-web].** `"create": false` on the main window, built in `setup()` via `WebviewWindowBuilder::from_config`. Windows 11 build ≥ 22621 (`windows_version::OsVersion::current().build`; declare `windows-version = "0.1"` as a direct `cfg(windows)` dependency) → `.transparent(true)` + `Effect::Mica` through Tauri's built-in effects API (not the `window-vibrancy` crate: duplicate-symbol link failure). Tauri ignores the effect's result, so the gate is ours; 21H2, Windows 10 and all Linux get an **opaque** window (WebKitGTK 2.54 + NVIDIA transparent windows fail). Pass the result via `initialization_script`. Sync the in-app theme with `setTheme()` (`@tauri-apps/api/app`, capability `core:app:allow-set-app-theme`) so the title bar and Mica follow it. Test on real hardware: Mica turns solid on low-end GPUs, in Battery Saver, with transparency off and **when the window is inactive**.

**`<html>` attributes:** `data-platform` (macos · windows · linux) · `data-backdrop` (mica · none) · `data-transparency` (auto · reduced; Settings ▸ Appearance ▸ Transparency / 透明效果, all platforms, since WebKitGTK lacks `prefers-reduced-transparency`) · `data-contrast` (auto · more; Settings ▸ Appearance ▸ Increase contrast / 增强对比度, Linux, since GTK3 WebKit lacks `prefers-contrast`) · `data-window-active` (focus/blur; dims the accessory bar).

| Surface | Windows 11 ≥ 22621 (Mica) | Windows 10 / 21H2 · Linux (opaque) |
|---|---|---|
| base · sidebar | Mica; body and sidebar transparent, no CSS glass | `surface.content` · `surface.sidebar` + `border-r` `rule` |
| content column | **inset LayerFill card** `#80FFFFFF` / `#4C3A3A3A`, 8 px top-left radius | flush `surface.content` |
| in-page toolbar row (48 px) | in the card; `.pl-glass` only once content scrolls under it (`mask-image` edge) | same |
| accessory bar, dialogs, popovers | `.pl-glass` with the **near-opaque Mica tint** (backdrop-filter cannot see through transparent pixels) | `.pl-glass` with real blur |
| lamp band | CSS radial gradient inside the content column only | same; rule when reduced / more contrast |

Measured: `text.secondary` on LayerFill over a #F3F3F3 / #202020 Mica approximation 5.70 / 6.41:1; over the glass tint composite 5.93 / 7.39:1.

```css
/* chrome.css — functional layer only; tokens from the generated tokens.css (§10) */
.pl-glass{ position:relative; isolation:isolate; background:var(--pl-glass-tint);
  box-shadow: inset 0 1px 0 var(--pl-glass-rim), inset 0 0 0 1px color-mix(in oklab, var(--pl-glass-rim) 40%, transparent),
              0 0 0 .5px var(--pl-glass-edge), var(--pl-glass-shadow); }
@supports (backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px)){
  .pl-glass{ -webkit-backdrop-filter: blur(24px) saturate(1.8); backdrop-filter: blur(24px) saturate(1.8); } }
.pl-glass::before{ content:""; position:absolute; inset:0; z-index:-1; border-radius:inherit; pointer-events:none;
  background: linear-gradient(to bottom, oklch(1 0 0 / .28), transparent 45%); }       /* sheen under the text */
html[data-backdrop="mica"]{ --pl-glass-tint: oklch(0.985 0 0 / .92); }
html[data-backdrop="mica"].dark{ --pl-glass-tint: oklch(0.22 0.008 265 / .90); }
html[data-transparency="reduced"] .pl-glass{ backdrop-filter:none; -webkit-backdrop-filter:none; background:var(--pl-color-surface-popover); }
@media (prefers-reduced-transparency: reduce){ .pl-glass{ backdrop-filter:none; -webkit-backdrop-filter:none; background:var(--pl-color-surface-popover); } }
html[data-contrast="more"] .pl-glass{ background:var(--pl-color-surface-popover); outline:1px solid var(--pl-color-text-primary); }
@media (prefers-contrast: more){ .pl-glass{ background:var(--pl-color-surface-popover); outline:1px solid var(--pl-color-text-primary); } }
@media (forced-colors: active){ .pl-glass{ background:Canvas; color:CanvasText; border:1px solid CanvasText; backdrop-filter:none; } }
html[data-window-active="false"] .pl-accessory{ opacity:.6; }
.pl-lamp-band{ background: radial-gradient(ellipse 70% 120% at 6% 0%, var(--pl-color-lamp-wash), transparent 75%); }
html[data-transparency="reduced"] .pl-lamp-band, html[data-contrast="more"] .pl-lamp-band{ background:none; box-shadow: inset 3px 0 var(--pl-color-lamp-rule); }
@media (prefers-reduced-transparency: reduce), (prefers-contrast: more){ .pl-lamp-band{ background:none; box-shadow: inset 3px 0 var(--pl-color-lamp-rule); } }
:lang(zh-CN) .pl-prose{ line-height:1.6; }
/* shadcn → --pl-color-*: background=surface-content foreground=text-primary muted-foreground=text-secondary border=rule
   card=surface-raised popover=surface-popover primary=accent primary-foreground=on-accent */
```
`@supports` stays plain CSS (nesting inside a Tailwind `@utility` is unverified). Never gate on `@supports (backdrop-filter: url(#x))` (WebKitGTK claims support, then drops the blur). At most three glass surfaces visible at once; no glass on rows or anything that scrolls. Tailwind: `@theme inline` maps `--color-content`, `--color-raised`, `--color-ink`, `--color-ink-secondary`, `--color-rule`, `--radius-callout|control|dialog|row`, `--ease-calm|quick` to the `--pl-*` variables.

**Screens (web milestones):** **IA change [M2]** to match the Mac — sidebar This Week · Courses (rows + compact week) · Setup · Settings (stays a route), see X1. **This Week [M2]:** same content, `.pl-lamp-band`, the `.pl-glass` capsule replaces sonner toasts (Undo 6 s, Ctrl+Z while visible). **Course [M2]:** shadcn `Tabs` as a segmented control; inspector = right panel (`w-80 bg-raised border-l border-rule`) at ≥ 1100 px, else `Sheet side="right"` "Course settings"; scrubber = sticky `.pl-glass` capsule, return capsule by FLIP transform + opacity. **Sources · Connect [M2]:** `rounded-callout bg-raised` sections, live `Progress`, `AlertDialog` for Remove (Cancel default); Connect keeps the Windows `steps.quitFirst` (tray → Quit), no running-app check; Linux shows the AppImage warning. **Settings [M2]:** + Transparency, Week starts on, AI access by course; Linux + Increase contrast. **Onboarding [M2]:** full-window route, 560 px column, sticky step bar (`.pl-glass` secondary, solid accent primary). **Tray [M3]:** native tray icon + native menu ("This week: 3 due", next 3 deadlines as disabled items, Sync now, Open PageLamp, Quit); optional on Linux (AppIndicator varies).

**Fonts/type:** Inter Variable with `opsz` (`@fontsource-variable/inter/opsz.css`, OFL) → `"Microsoft YaHei UI"` (Windows) / `"Noto Sans CJK SC"` (Linux); mono Cascadia Mono/Consolas · DejaVu/Noto Sans Mono; web ramp §10.3 (min 12 px); Semibold, never Bold; no serif. The Tauri macOS legacy build keeps `system-ui`. **Motion:** §10.4 `linear()` springs, transform/opacity only, `prefers-reduced-motion` → 200 ms crossfades. **Keys:** Ctrl+R / F5 sync (`preventDefault`), Ctrl+[ / Ctrl+] weeks [verify accelerator suppression], Ctrl+Shift+T current week, Ctrl+1/2/3, Ctrl+Shift+N add source, Ctrl+Alt+I course settings; avoid Ctrl+Shift+I and Alt+←/→. **Window:** `minWidth` 900 → 500 (Snap Layouts); sidebar collapses to icons < 760 px. **Accent:** brand accent (white-label), unlike the Mac.

```text
X1 · Windows 11 (build ≥ 22621, Mica), Tauri build, This Week
╭───────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ ▪ PageLamp                                                                            ─    ▢    ✕     │
├──────────────────────┬────────────────────────────────────────────────────────────────────────────────┤
│                      │ ┌────────────────────────────────────────────────────────────────────────────┐ │
│ ▪ This Week          │ │ This Week · Sep 26 – Oct 2                        ↻ Sync now   ⌕ Search    │ │
│                      │ │░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░                                         │ │
│ Courses              │ │░░░░░ Friday, September 26                                                  │ │
│  ▪ DEMO101    Wk 4   │ │░░░░░ 3 deadlines in the next 7 days · 2 plan tasks today                   │ │
│  ▪ DEMO205    Wk 4   │ │░░░           Due today 11:59 PM · in 58 min           [ Open course ]      │ │
│                      │ │     Next 7 days                                3 deadlines · 1 class       │ │
│ Setup                │ │     Today      11:59 PM  Problem Set 2 — q. 1–3   DEMO205  Assignment      │ │
│  ▪ Sources & sync  1 │ │     Your courses                                                           │ │
│  ▪ Connect AI app    │ │     DEMO101  Intro to Demo Studies ····················· Week 4            │ │
│  ▪ Settings          │ │                 (( ✓ Sync finished · 3 new materials ))                    │ │
│ ✓ Synced 2 h ago     │ └────────────────────────────────────────────────────────────────────────────┘ │
╰──────────────────────┴────────────────────────────────────────────────────────────────────────────────╯
```

## 9. Risks and cut list

| Risk | Mitigation |
|---|---|
| Glass creeps into content | CI grep (§1.3); §4.1 checklist; screenshot review in light/dark/IC/RT |
| Spill does not render | first-view, full-bleed band ignoring the top safe area; [verify]; fallback §6.1; the design reads without it |
| Union / matched-geometry differ on 26 vs 27 | only same-Glass capsules unioned; fallback: one capsule with an inline button |
| Custom material rows miss `List` behaviour | §3.2 (MaterialRow) spec + UI tests (focus, ↑/↓, Return, double-click, Space, context menu, rotor) |
| Sidebar capsule: glass on the sidebar glass looks faint | on-device review light/dark and with several accents; fallback: a neutral tint (white 25 %, dark 8 %), never the accent |
| Custom sidebar misses list behaviour | `SidebarNavigation` tests (the rules measured on the 27.2 list) + the on-device list in `apps/macos/README.md` |
| Custom section picker misses the system control's keyboard or VoiceOver behaviour | parity measured against the system control (§3.2.1 table: keys, focus, the tree VoiceOver reads), `SegmentedNavigation` tests, the user checks U1–U8 in `apps/macos/README.md`; fallback: Debug ▸ Course Pages Use the System Section Picker, then a revert |
| Capsule motion under load | `SidebarMotionGate` (page first, slide on an idle frame), coalesced page switches, `perf-probe.sh capsule` |
| Lamp read as warning | hue 78 vs 55, wash vs glyph channels, always a word; colour-blind and night-time dark-mode tests |
| Closed inspector hides compliance | AI status line, Course menu, Privacy table, Connect popover; hallway test "turn off AI reading for DEMO205" |
| zh toolbar crowding | `visibilityPriority` [26.1]; 26.0 drops the website item < 900 pt |
| CLT-only toolchain (`@State`, `actool`, String Catalogs) | `ViewState` alias, `.strings` generator, squircle `.icns`; the owner's Xcode-licence decision unblocks the rest |
| Mica / WebKitGTK gaps | legible without blur; near-opaque tint; in-app Transparency/Contrast; hardware tests |
| Tauri parity drift | one DTCG token file, one string source (incl. `mac.*`), this spec; Tauri macOS legacy frozen to fixes |
| Intel Macs on 26 | one container per window, no glass in lists, one gradient per page |

**Cut order (first to go):** serif titles → day ribbon → welcome ignite and pool warming → Contents leaders and rolling numerals → menu bar lamp logic, then the MenuBarExtra → search → term strip → week scrubber (toolbar ‹ › stays) → capsule fuse/split → Connect running check and access popover (Privacy table stays) → client picker (stacked sections) → Quick Look for Canvas files.

**Never cut:** the AI disclosure and its gate; the Canvas notice above the token field; honest No AI withholding (header, callout, "Not shared" rows, disabled switch + note); per-course AI access; Replace for expired tokens/feeds; the "can count as viewing" dialog; the "titles and dates only" note; temporary-location and quarantine warnings; diagnostic preview before copy; VoiceOver, keyboard, RT/IC/RM paths; both locales.

## 10. Design tokens (W3C DTCG 2025.10)

**Files** (`apps/desktop/tokens/`): base `pagelamp.tokens.json` (light · normal contrast · macOS) + context files for the resolver modifiers **theme** (light · dark), **contrast** (normal · increased), **platform** (macos · windows · linux), **backdrop** (none · mica), **brand**, in `pagelamp.resolver.json`. `scripts/gen-tokens.mjs` (zero-dependency) emits `src/tokens.css` and `…/Chrome/PLTokens.swift`; CI fails on a diff. Each row is one token and maps 1:1: `color.text.secondary` → `--pl-color-text-secondary` → `PLTokens.Color.textSecondary`. Colours are `{"colorSpace":"oklch","components":[L,C,h],"alpha":a,"hex":…}` (all in sRGB). "system" = the Mac uses the platform value; the generator skips it for Swift (`$extensions["dev.pagelamp"].macos = "system"`). Springs are `$type: "transition"` with preset, bounce, stiffness, damping and settle time in `$extensions["dev.pagelamp"]`; CSS gets a generated `linear()` curve.

### 10.1 Colour (OKLCH `L C h`, hex)

| Token | Light (base) | Dark | IC light | IC dark | macOS |
|---|---|---|---|---|---|
| `color.accent` | 0.47 0.08 252 `#385d86` | 0.78 0.07 252 `#97bbe4` | 0.38 0.10 252 `#104375` | 0.85 0.07 252 `#add1fb` | `AccentColor` asset only |
| `color.on-accent` | 0.985 0 0 `#fafafa` | 0.20 0.02 252 `#0f171f` | 1 0 0 `#ffffff` | 0.13 0.02 252 `#03080f` | system |
| `color.surface.content` | 0.99 0.002 85 `#fcfcfa` | 0.21 0.004 260 `#17181a` | 1 0 0 `#ffffff` | 0.13 0 0 `#070707` | system |
| `color.surface.raised` | 0.965 0.004 85 `#f5f3f0` | 0.245 0.006 260 `#1f2123` | 0.95 0 0 `#eeeeee` + border | 0.20 0 0 `#161616` + border | `.fill.tertiary` |
| `color.surface.sidebar` (opaque only) | 0.965 0.004 85 `#f5f3f0` | 0.23 0.006 260 `#1b1d20` | 0.97 0 0 `#f5f5f5` | 0.18 0 0 `#121212` | system glass |
| `color.surface.popover` | 0.995 0 0 `#fdfdfd` | 0.26 0.006 260 `#222427` | 1 0 0 | 0.13 0 0 | system |
| `color.text.primary` | 0.22 0.012 260 `#171b20` | 0.94 0.005 260 `#e9ebef` | 0.10 0 0 `#030303` | 1 0 0 | `.primary` |
| `color.text.secondary` | 0.50 0.012 260 `#5f636a` | 0.74 0.010 260 `#a7abb1` | 0.36 0.01 260 `#3a3d42` | 0.86 0.01 260 `#cdd1d8` | `.secondary` |
| `color.text.tertiary` (decorative) | 0.66 0.010 260 `#8f9298` | 0.56 0.010 260 `#71757a` | 0.50 0.01 260 `#606369` | 0.72 0.01 260 `#a1a5ab` | `.tertiary` |
| `color.rule` | text.primary @ 10 % | white @ 10 % | text.primary @ 35 % | white @ 40 % | `.separator` |
| `color.lamp.wash` | 0.86 0.10 78 `#f5c984` @ **28 %** | 0.74 0.11 72 `#d69f58` @ **17 %** | — (rule) | — (rule) | `PLColor.lampWash` |
| `color.lamp.rule` | 0.60 0.12 78 `#a77612` | 0.84 0.12 80 `#f4c26a` | 0.50 0.10 78 `#825b0c` | 0.88 0.10 82 `#f9d28a` | `PLColor.lampRule` |
| `color.status.success` | 0.52 0.11 155 `#267b4c` | 0.74 0.12 155 `#66c189` | 0.42 0.10 155 `#095c34` | 0.84 0.12 155 `#87e2a8` | `PLColor.success` |
| `color.status.warning` | 0.62 0.15 55 `#c9690c` | 0.80 0.13 60 `#fba962` | 0.50 0.13 50 `#9b4805` | 0.86 0.10 65 `#ffc48a` | `PLColor.warning` |
| `color.status.danger` | 0.56 0.19 27 `#cc3430` | 0.70 0.17 22 `#f66c6d` | 0.46 0.17 27 `#a21a1b` | 0.80 0.11 22 `#fda19e` | `PLColor.danger` |
| `color.glass.tint` | 1 0 0 @ 62 % | 0.24 0.01 265 `#1d1f24` @ 55 % | → `surface.popover` | → `surface.popover` | system |
| `color.glass.tint` (backdrop mica) | 0.985 0 0 @ 92 % | 0.22 0.008 265 `#191b1e` @ 90 % | → `surface.popover` | → `surface.popover` | — |
| `color.glass.rim` / `color.glass.edge` | white @ 75 % / black @ 10 % | white @ 16 % / black @ 50 % | outline `text.primary` | outline `text.primary` | system |
| `color.mica.layer-fill` | `#80FFFFFF` | `#4C3A3A3A` | `#FFFFFF` | `#000000` | — |

Contrast (WCAG 2; composites in gamma-encoded sRGB like browsers), light / dark / IC light / IC dark: text.primary on content 16.82 / 14.86 / 20.59 / 20.12 · text.secondary on content 5.83 / 7.68 / 10.86 / 13.15 · on raised 5.42 / 7.04 / 9.39 / 11.83 · on the lamp peak 5.21 / 5.68 · accent on content 6.62 / 8.89 / 10.03 / 12.78 · on-accent on accent 6.52 / 9.08 / 10.03 / 12.77 · success 5.07 / 8.08 / 8.06 / 12.89 · warning (glyph only) 3.72 / 9.20 / 6.31 / 12.92 · danger 4.97 / 6.14 / 7.78 / 10.30 · lamp.rule 3.90 / 10.74 / 6.09 / 13.94 · tertiary (decorative) 3.02 / 3.81.

### 10.2 Radius, spacing and sizes

| Token | macOS | Tauri macOS | Windows | Linux |
|---|---|---|---|---|
| `radius.callout` (tiles, cards) | 12 pt | 12 | 8 | 12 |
| `radius.row` | 8 | 8 | 4 | 8 |
| `radius.inner-min` | 6 | 6 | 4 | 6 |
| `radius.control` / `radius.popover` / `radius.dialog` | system | 8 / 12 / 14 | 4 / 8 / 8 | 9 / 15 / 15 |
| `radius.mica-card` | — | — | 8 (top-left) | — |
| `radius.capsule` · `radius.window` | `.capsule` · system | 9999 · system | 9999 · system (8; 0 snapped) | 9999 · system |

`space.1…12` = 4 · 8 · 12 · 16 · 20 · 24 · 32 · 40 · 48 (pt / px). `layout.measure` 760 · `layout.gutter` 40 (24 narrow) · `layout.section-gap` 40 (course 32) · `layout.title-to-rule` 12 · `layout.row-padding` 8 · `layout.day-group-gap` 20 · `layout.deadline-day-column` 120 · `layout.sheet-inset` 24 · `layout.welcome-column` 520 (web 560) · `layout.capsule-max` 520. `size.window.main` 1180 × 760, min 760 × 520 (web 1200 × 800, min 500 × 600) · `size.sidebar` 200 / 232 / 300 (web 232; icons < 760) · `size.inspector` 260 / 300 / 360 (web panel 320 at ≥ 1100) · `size.window.welcome` 680 × 620 · `size.settings.width` 600 · `size.menubar.width` 340 · `size.accessory.height` 36 · `size.week-button.min` 26 (web 28) · `size.toolbar-row` web 48 · `size.glyph.empty` 44 · `size.glyph.lamp` 56 · `size.step-number` 22.

### 10.3 Typography

| Token | macOS (SwiftUI style, pt) | Windows / Linux (px, weight) |
|---|---|---|
| `font.large-title` | `.largeTitle` 26/32 (course name Semibold) | 28/36 Semibold |
| `font.title-1` | `.title` 22/26 | 20/28 Semibold |
| `font.title-2` | `.title2` 17/22 | 18/24 Semibold |
| `font.title-3` | `.title3` 15/20 Semibold | 16/22 Semibold |
| `font.headline` | `.headline` 13/16 Bold | 14/20 Semibold |
| `font.body` | `.body` 13/16 | 14/20 |
| `font.callout` | `.callout` 12/15 | 13/18 |
| `font.subheadline` | `.subheadline` 11/14 (eyebrow Semibold) | 12/16 (eyebrow Semibold) |
| `font.mono` | `.callout.monospaced()` 12/15 | 13/18 |
| `font.family.sans` · `.mono` | system (SF Pro → PingFang SC) · SF Mono | Inter Variable → YaHei UI / Noto Sans CJK SC · Cascadia Mono / DejaVu Sans Mono |
| `font.design.title` | `.serif` for date and welcome titles, Latin locales [M3] | sans |
| `font.min` · `font.cjk.paragraph` | 11 pt · `.lineSpacing(2)` | 12 px · `line-height: 1.6` |

### 10.4 Motion (stiffness = (2π/d)², damping = 4π(1 − bounce)/d; CSS settle = |x − 1| < 5 × 10⁻⁴)

| Token | SwiftUI | d (s) | bounce | stiffness | damping | CSS settle |
|---|---|---|---|---|---|---|
| `motion.calm` | `.smooth(duration: 0.45)` | 0.45 | 0 | 194.96 | 27.93 | 716 ms |
| `motion.quick` | `.snappy(duration: 0.3)` | 0.30 | 0.15 | 438.65 | 35.60 | 445 ms |
| `motion.lively` | `.bouncy` | 0.50 | 0.30 | 157.91 | 17.59 | 868 ms |
| `motion.week` | `.smooth(duration: 0.35)` | 0.35 | 0 | 322.27 | 35.90 | 556 ms |
| `motion.section` | `.smooth(duration: 0.25)` | 0.25 | 0 | 631.65 | 50.27 | 398 ms |
| `motion.step` | `.smooth(duration: 0.4)` | 0.40 | 0 | 246.74 | 31.42 | 636 ms |
| `motion.hover` · `motion.reduced` | `.easeOut(duration: 0.15)` · `.easeInOut(duration: 0.2)` | — | — | — | — | 150 ms ease-out · 200 ms ease-in-out |

Each spring token is also `PLMotion.<name>Spring` (duration, bounce) for `CASpringAnimation(perceptualDuration:bounce:)` (motion the render server runs: the sidebar capsule and the section picker's thumb).

### 10.5 Glass levels

| Token | macOS | Web normal | Web Mica mode | Web reduced · IC · forced colours |
|---|---|---|---|---|
| `glass.regular` | `.glassEffect(.regular, in:)` | `glass.tint` + `blur(24px) saturate(1.8)` + rim `inset 0 1px 0` + rim @ 40 % inset ring + edge `0 0 0 .5px` + shadow `0 10px 30px -12px` (black 22 % / 50 %) + sheen | tint 92 % / 90 % (blur ineffective over transparent pixels) | opaque `surface.popover` · + 1 px `text.primary` outline · `Canvas`/`CanvasText` + border |
| `glass.interactive` | `.regular.interactive()` | hover tint +4 %, `:active` `scale(.98)` with quick | same | no scale under reduced motion |
| `glass.prominent` | `.regular.tint(.accentColor).interactive()` / `.glassProminent` | `color-mix(in oklab, accent 88%, transparent)` + blur, `on-accent` text | accent 96 % | solid accent |
| `glass.container.spacing` · `glass.gap` | 12 · 14 | — · 14 | — · 14 | — |
| `glass.budget` | 1 container, ≤ 4 shapes per window, plus the sidebar selection capsule and the section picker's thumb | ≤ 3 glass surfaces visible | same | same |
| `glass.clear` | not used | not used | — | — |

Swift output: each colour becomes `Color(nsColor: NSColor(name: nil) { $0.bestMatch(from: [.aqua, .darkAqua, .accessibilityHighContrastAqua, .accessibilityHighContrastDarkAqua]) … })` with the four hex values; the prototype's `PLTokens.Glass` group is not emitted to Swift (it would shadow SwiftUI's `Glass`).

## 11. Component inventory

Mac code in `apps/macos/Sources/PageLamp/` (glass only in `Chrome/`); web in `apps/desktop/src/components/`.

| Component (M) | SwiftUI sketch | Web counterpart |
|---|---|---|
| `RootView` (1) | `NavigationSplitView(columnVisibility:)` + `.searchable(placement: .toolbar)`; S2 gate | `AppShell` |
| `SidebarList`, `SidebarRows`/`SidebarRow`, `SidebarSelectionGlass` (Chrome), `SidebarFooterStatus` (1) | custom `LazyVStack` source list (`SidebarLayout`, `SidebarNavigation`, `SidebarMotionGate` in PageLampModel), `NSGlassEffectView` capsule moved by Core Animation, footer in `.safeAreaInset(edge: .bottom)` | `Sidebar`, `SyncPill` as a text row |
| `PageLampCommands`, toolbar sets (1) | `CommandMenu` + `@FocusedValue`; `ToolbarItem`, `ControlGroup`, `ToolbarSpacer`, `.visibilityPriority` [26.1] | key handler, in-page toolbar row |
| `AccessoryBar`, `StatusCapsule`, `SyncDetailsPopover` (1; fuse 2) | §6.2; `.popover` with plain rows | `.pl-accessory`, `StatusCapsule.tsx`, `Popover` |
| `WeekScrubber` (2) | §6.3 | sticky `WeekScrubber.tsx` |
| `LampBand`, `LampWash`, `ReadingColumn` (1) | §6.1 | `.pl-lamp-band`, `max-w-[760px] mx-auto` |
| `PageHeader`, `SectionHeader`, `Callout`, `CodeBlock` (1) | `Text` styles + `.isHeader`; fills in `.rect(cornerRadius: 12)` + `.containerShape`; selectable mono text + copy | `Section`, `Notice`, `CodeBlock` |
| `NextUpLine` (1), `DayRibbon` (3) | `TimelineView(.everyMinute)`; plain buttons + `ScrollViewReader` | `NextUp`, `DayRibbon` |
| `Next7DaysSection`, `StudyPlanSection`, `ContentsSection` (1) | `Grid` + rotor; `DisclosureGroup`; plain-button rows + `Path` leader | `ThisWeek`, `StudyPlanCard`, `CourseList` |
| `CourseHeader`, `AIStatusLine`, `WeekLine`, `CourseSectionPicker` + `GlassSegmentedControl` (Chrome) (1) | inside `LampBand`; `.accessibilityAdjustableAction`; a custom `NSControl` with the shared glass thumb over a SwiftUI track (`SegmentedLayout`, `SegmentedNavigation` in PageLampModel), the system pop-up menu when narrow | `CourseHeader`, `Tabs` |
| `MaterialList`, `MaterialRow` (1; Space 3) | `.focusable`, `@FocusState`, `.onMoveCommand`, `.onKeyPress`, `.onTapGesture(count: 2)`, `.contextMenu`, `.quickLookPreview` | `MaterialList` |
| `TermStrip` (2), `EvidenceList` (1) | plain buttons; `AttributedString` + `languageIdentifier` [verify] | `TermStrip`, `lang="en"` list |
| `CourseInspector` (1 read-only; 2) | `Form(.grouped)`: `Picker(.radioGroup)`, `TextEditor`, `Toggle(.switch)`, `DatePicker(.field)` | right panel / `Sheet` |
| `SourceSection`, `SyncProgressRow` (1) | `Section`, `LabeledContent`, `ProgressView(value:total:)`, `DisclosureGroup` | `SourceCard`, `SyncProgressRow` |
| `AddSourceSheet`, `ChoiceTile`, `SecretField`, `CanvasNotice`, `DisclosureGate` (2) | `.sheet` + `.presentationSizing(.form)`; radio buttons; `SecureField` ⇄ `TextField`; `fileImporter`; `dropDestination` | `AddSourceDialog` family |
| `ReplaceSecretSheet`, `RemoveSourceDialog`, `DownloadConfirmation` (2) | `.sheet`; `.confirmationDialog` + `.dialogIcon` | `ReplaceSecretDialog`, `AlertDialog` |
| `DisclosureBlock`, `DiagnosticPreviewSheet`, `EmptyState` (1) | privacy `Callout`; `.sheet` + mono `ScrollView`; `ContentUnavailableView` | `AiDisclosure`, report dialog, `Empty` |
| `ClientPicker`, `InstallSteps` (1), `RunningAppNotice`, `ReviewAccessPopover` (2) | `Picker`; numbered steps; `NSWorkspace` observer; `.popover` | Connect pieces |
| `CourseAccessTable` (2) | `Table` + `.switch` toggles | Settings table |
| `WelcomeFlow`, `WelcomeStepBar` (2) | `ScrollView` + `.safeAreaBar` + `GlassEffectContainer` | `OnboardingPage` |
| `MenuBarWeekView` (3) | `VStack` + `Divider`, plain rows, `.bordered`, `SettingsLink` | native tray menu |
| `PLTokens` / `PLColor` (1) | generated (§10) | `tokens.css` |

## 12. Milestone cut

**M1 — shell and reading (mock data allowed).** Mac: main window + Settings scenes (a debug menu adds mock sources; no welcome yet); sidebar and footer; toolbars; menus for existing commands; **This Week** (lamp band with spill, Next up, Next 7 days, plan, Contents without leaders); **course detail read-only** (header band, AI status line, picker, week line, toolbar ‹ ›, materials with Mac list behaviour, Deadlines, Timeline without strip, read-only inspector); **Connect** (full static flow, disclosure, S15); **Sources list** (status, Sync All / per-source sync with live progress, problem callouts, busy-disabled buttons); **Settings basics** (language, appearance, Data, Privacy text, Help with diagnostic preview, About); basic capsule (syncing / finished / attention); S1–S4, S7 and S9 display, S11, S12, S14, S15; tokens pipeline, strings generator with `mac.*`, glass-lint CI. Tauri: tokens, backdrop decision, `<html>` attributes, theme sync, `.pl-glass` and fallbacks, restyle. *Exit:* every M1 screen passes the §7.4 test matrix.

**M2 — full parity (gate for students).** Mac: welcome window; Add Source (Canvas notice, disclosure gate), Replace, Remove, folder drop; downloads; inspector editing incl. the No AI consequence line; UndoManager; capsule fuse/split and bubbles; week scrubber and return capsule; term strip; hidden and past courses; S5, S6, S10, S13, S16, S17; search; Connect running check, copy variants, access popover; Privacy table; Week starts on; Reduce Highlighting Effects; shared preferences (§13 #1). Tauri: IA change, capsule instead of toasts, scrubber, inspector panel/sheet, Settings additions, `minWidth` 500, LayerFill card. *Exit:* the `mac-scope.md` §1 parity list is complete and every never-cut item ships.

**M3 — glance, reminders, Quick Look, polish.** Mac: MenuBarExtra + Open at Login; Reminders (`planned_reminders`, local notifications); Quick Look incl. Canvas files; day ribbon; ignite; pool warming; leaders and rolling numerals; serif titles; the one bounce; `AccentColor` asset and Icon Composer icon (Xcode). Tauri: native tray menu.

## 13. Facade additions needed (backend/maintainer decision; each has an M1 fallback)

| # | Addition | Why | Fallback | M |
|---|---|---|---|---|
| 1 | Shared preferences in core: `ai_disclosure_acknowledged_at`, `show_hidden_courses`, `onboarding_skipped` | today only in Tauri `localStorage`; rule 8's acknowledgement must be shared by Mac, Tauri, CLI | Mac `UserDefaults`, asking the disclosure once more (documented) | 2 |
| 2 | `this_week(now, horizon_days, week_starts_on) -> WeekDigest` (day groups deadlines vs classes, due count, next ≤ 24 h, today's plan items, plan meta, per-course week line) | one grouping for This Week, menu bar, reminders, Tauri | Swift port of `thisWeek.ts` (M1 only) | 2 |
| 3 | `CourseTimeline.total_weeks: Option<u32>` | "Week 4 of 12", scrubber range, term strip | `available_weeks` only | 2 |
| 4 | `WeekMaterials.week_start` / `week_end: Option<NaiveDate>` | "Sep 22 – 28" (teaching-week dates are core logic) | omit the range | 2 |
| 5 | `McpClientConfig.entry_content` / `servers_key_content` | Copy Entry Only / mcpServers Section without per-shell JSON reshaping | port `snippet.ts` with its tests | 2 |
| 6 | typed `SourceRecord` config (`base_url`, `path`, `term_start`, `account_name`) | `config` is `serde_json::Value` (a JSON string over UniFFI) | one Swift adapter | 1 |
| 7 | `material_local_path(id)` | Quick Look / Show in Finder for Canvas downloads (`MaterialView` has no path) | folder `file://` only | 3 |
| 8 | `planned_reminders(now, horizon)` with stable ids | reminders planned in core, delivered by Swift | no Reminders tab | 3 |
| 9 | optional `SourceSyncResult.new_materials` | "· 3 new materials" | "Sync finished" | 3 |

Not needed: `VERSION` — use `AppStatus.version` (`DoctorReport.version` when the DB is down). `McpClientPresence` means *configured*; no installed-app detection is designed. The UniFFI wrapper (`crates/pagelamp-ffi`) is plumbing, not a facade change.

## Appendix A. API availability

- **26.0 (target, no gate):** `NSGlassEffectView`, `glassEffect`, `Glass.regular/.tint/.interactive`, `GlassEffectContainer`, `glassEffectID`, `glassEffectUnion`, `glassEffectTransition`, `.buttonStyle(.glass/.glassProminent)`, `backgroundExtensionEffect`, `safeAreaBar`, `scrollEdgeEffectStyle`, `ConcentricRectangle`, `ToolbarSpacer`, `sharedBackgroundVisibility`, `dropDestination(for:isEnabled:action:)` + `DropSession`.
- **26.1:** `ToolbarItem.visibilityPriority`. **26.4:** `\.accessibilityReduceHighlightingEffects`, `\.accessibilityPrefersCrossFadeTransitions`. **27:** `.pickerStyle(.tabs)` (else `.segmented`), `NSViewCornerConfiguration`; automatic on 27: interactive bounce, accent sidebar icons, edge-to-edge sidebar, tighter corners, hidden menu icons, Show Borders honoured. All gated with `#available`.
- **≤ 15, no gate:** `Window`, `Settings`, `MenuBarExtra(isInserted:)`, `.menuBarExtraStyle(.window)`, `SMAppService` (13); `.inspector`, `InspectorCommands`, `SettingsLink`, `@Observable`, `.onKeyPress` (`onKeyPress(keys:phases:)` with `.repeat` / `.up`), `.focusable`, `PhaseAnimator`, `ContentUnavailableView`, `.fill` styles, `NSApp.activate()`, `CASpringAnimation(perceptualDuration:bounce:)`, `NSView.displayLink(target:selector:)`, `AccessibilityNotification.Announcement` (14); `defaultLaunchBehavior`, `restorationBehavior`, `windowBackgroundDragBehavior` (scene modifiers), `Tab`, `searchFocused` (15); `\.sidebarRowSize` (13); `quickLookPreview`, `\.accessibilityShowBorders`, `ScrollViewReader` (11); `\.appearsActive`, `NSColor(name:dynamicProvider:)`, `NWPathMonitor`, `UNCalendarNotificationTrigger`, `NSWorkspace` APIs (≤ 10.15).
- **Not used:** `Glass.clear` (policy); iOS-only `tabViewBottomAccessory`, `ToolbarOverflowMenu`, `toolbarMinimizeBehavior`, `.topBarPinnedTrailing`, `SearchToolbarBehavior.minimize`.

## Appendix B. Verification log (2026-09-26)

- Three probe files typecheck with `xcrun swiftc -typecheck -parse-as-library -sdk MacOSX27.0.sdk -target arm64-apple-macos26.0` (exit 0; CLT, `ViewState` alias): `probe.swift` and `probe2.swift` exercise every SwiftUI/AppKit construct named here, and `probe3.swift` compiles every Swift snippet in this document verbatim against stub views. Negative checks: ungated `.pickerStyle(.tabs)` and `\.accessibilityReduceHighlightingEffects` fail as expected; `windowBackgroundDragBehavior` is a scene (not view) modifier.
- Every SF Symbol named here resolves via `NSImage(systemSymbolName:)` on macOS 27.2; every lucide name (and `LucideProvider`) exists in the repo's lucide-react 1.48.0.
- Contrast from OKLCH → sRGB (CSS Color 4) and WCAG 2; springs from the SwiftUI preset formulas (the 0.5 s presets reproduce the research's 796 / 742 ms).
- Facade facts checked in `crates/pagelamp-app/src/{lib,sync,diagnostics}.rs` and `crates/pagelamp-core/src/{model,views}.rs`; copy from `apps/desktop/src/i18n/locales/{en,zh-CN}`.
- Sidebar probes (macOS 27.2 only; a native `List(selection:)` sidebar and a source-list `NSTableView` side by side): rows 32 / 40 pt at the medium / large Sidebar icon size; at small the table's `rowHeight` is 24 but the SwiftUI list uses automatic row heights, so its rows are 25–27 pt (about 5 pt above and below the tallest glyph; the custom list keeps 24, §2.3); titles 11 / 13 / 15 pt at x 42 / 46 / 48, section headers 19 pt tall with a 13 pt gap, 11 pt Semibold at x 14, row 0 at the 52 pt toolbar safe area; the selection is a per-row `NSVisualEffectView` (material `.selection`, radius 8) that turns solid `controlAccentColor` with a white title when the list has focus in a key window; the segmented control's selected thumb is a glass SDF + `glassBackground` layer stack (what `NSGlassEffectView` draws); keys: ↑/↓ with ⇧ ⌃ ⌘ alike, ⌥↑/↓ first/last, Home/End scroll only, Page Up/Down ignored; type-select resets after 1.15 s < t ≤ 1.2 s (2 × (key-repeat delay + interval)); VoiceOver reads a lazy stack with `.contain` and a label as "list".
- Section picker probes (2026-09-27, macOS 27.2, 60 Hz; the system control at HEAD 3dca928 and behind the Debug switch, same binary, window 1400 × 900, DEMO205, mock data): geometry, label ink and colour (light, dark, simulated Increase Contrast, Reduce Transparency and Show Borders, disabled, inactive; ink read from upright `cacheDisplay` renders: `CALayer.render(in:)` draws SwiftUI's label layers upside down, which once made the labels look 0.5 pt low and led to drawing them 0.5 pt high), the focus ring's ink and colour, and every recorded keyboard state (Tab × 14, clicks with and without focus elsewhere, ←/→ with each modifier, Space and modified Space, ↓ and the keys that go on, counted by `NSWindow.noResponder(for:)`, the page's scroll offset, focus away and back, an inactive window) are identical to the system control's (§3.2.1 table), with Full Keyboard Access off and on (on per process: a `DYLD_INSERT_LIBRARIES` library answering `AppleKeyboardUIMode` = 2 to CFPreferences, as AppKit 27.2 ignores the `-AppleKeyboardUIMode 2` launch argument); the one difference is the system control's key-segment snap-back on SwiftUI updates (row 26). The notifications VoiceOver listens to were recorded out of process with an `AXObserver` (row 36). The accessibility tree was read out of process with the AX API, as VoiceOver reads it: same position, roles, descriptions, values, frames (the segments are the slots, 91 / 81; an in-process legacy read of the system control gives 91.5 / 92 / 91.5), actions, settable attributes, hit tests, press, focus and disabled behaviour; in-process legacy reads of an `NSControl` subclass without a cell say `AXUnknown` and are not what VoiceOver sees. ViewThatFits switches to the menu at the same widths (≤ 852 English, ≤ 822 Chinese, inspector open) and widening doesn't slide. `perf-probe.sh segment`: every change slides, the spring starting 2–7 ms after the input is handled (a model change: with its update's commit), each read within 0.4 pt of the ideal `sectionSpring` curve; with simulated Reduce Motion every trigger jumps. Resizing 1400 → 760 in one step while opening the inspector raises AppKit's "more Update Constraints passes" exception with either picker (pre-existing).
- Still **[verify] on device:** the sidebar capsule's look on the sidebar glass (light, dark, accents, Reduce Transparency, Increase Contrast, the 27 glass slider), the section picker's user checks U1–U8 (§3.2.1, `apps/macos/README.md`) and the other items listed in `apps/macos/README.md`; the spill under sidebar/inspector; union and matched-geometry looks on 26 vs 27; Cancel as default in Remove; VoiceOver language switching; Reopen Now; the Claude Desktop bundle id; the menu bar extra's container shape; login items on ad-hoc builds; WebView accelerator suppression.

## Appendix C. Review findings → resolution

- **Both reviews:** spill in a centred column → full-bleed first view (§6.1); custom paper → system backgrounds, paper tokens web-only (§4.2); dark lamp 10 % → 17 % (§10.1); 10 pt Chinese → 11 pt floor (§4.3); status in three places → roles split (§2.3); 6 s Undo → `UndoManager` + "Undo ⌘Z" (§2.7); root `.tint` → none (§4.2); VoiceOver language → [verify] + fallback (§7.1); too quiet / no week overview → spill, fuse/split capsule, scrubber, term strip, ignite (§6); serif mixed-script → date/welcome only (§4.3); `VERSION` → `AppStatus.version` (§13).
- **native-maximal:** glass-on-glass slab, glass in the content hero, toolbar "dl" morph, IDs on button-style glass, live menu-language claim → §1.3, §3.6, §2.5, §7.3.
- **glance-dashboard:** scrubber `.ignore`, colour-coded policies, over-budget glass, lamplight everywhere, Mica tray flyout → §6.3, §3.2, §1.3, §6.1, §8.
- **Grafts adopted:** `this_week` digest, Week starts on, configured-client preselection, running-app check, copy variants, Privacy table, `mac.*` namespace, Reduce Highlighting Effects, LayerFill card, ignite, `-NSDoubleLocalizedStrings`, data-contrast / window-active / forced-colors.