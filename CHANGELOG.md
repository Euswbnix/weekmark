# Changelog

All notable changes are listed here. The project follows [Semantic Versioning](https://semver.org/)
(0.x: anything may change between minor versions).

## [Unreleased]

### Added
- Course weeks that follow the real teaching dates: a Canvas term that is really an enrollment
  window (for example the University of Toronto's May–January "Fall" term) is no longer used to
  count weeks; PageLamp uses the course's own dates, the dates you set, and the week numbers in the
  materials your instructor posts, and shows teaching, reading week, exams and ended.
- Past courses: finished courses move to a collapsed "Past" group (in the app and in
  `pagelamp courses`) and their deadlines stay listed; "I'm still taking this" undoes it. Dates
  you set in v0.1 that match the synced term are cleared, and remaining ones ask you to check them
  once.
- Files are read in a separate, resource-limited process, so one bad PDF can't stop a sync; the
  diagnostic report counts the files that can't be read, by reason.
- A backup of the database before an update changes its format.
- A new look for the desktop app: course pages read like paper with thin dividers, the toolbar
  turns to glass as you scroll, sync status sits in a small capsule, and a warm band marks this
  week. On Windows 11 22H2 and later the window uses Mica; "Reduce transparency" in Settings (or
  the system setting) makes every surface solid. The typeface is now Inter.
- MCP: your AI app can propose a course's term dates. The `course_calendar` prompt has it read the
  syllabus and call the new `propose_course_calendar` tool; PageLamp keeps only the dates the
  material's own words state, and you accept or dismiss the proposal in the app. At most 3 per
  course per day, and never for a course whose materials you don't share with AI.

### Changed
- MCP: tools declare all four annotation hints (read-only, destructive, idempotent, open-world).
- A course file whose text can't be read now says why, in the app's language: no text found (for
  example a scanned PDF), too large, password-protected, damaged or an old format, or it hit the
  file reader's time or memory limit; the week's count of files your AI app can read leaves them
  out.
- MCP: `course_overview` and `week_materials` report `course.term_start`/`term_end` as the dates
  that count weeks (first class to exams end or last class), or null when none are known — never
  an enrollment-window term — with the new `course.term_dates_source`. `week_materials` gains
  `phase`; `course_overview` gains `lifecycle`, and its timeline gains the phase and its structure
  (teaching segments, breaks, exams end, anchor). `list_courses` gains `phase`, `lifecycle` and
  `outside_term`.
- `pagelamp courses` groups courses into Current, Upcoming and Past; past courses (also in
  `--json`) are listed with `--past` or `--all`. `pagelamp course term` dates mean the first and
  last day of classes.
- Releases: one universal macOS disk image for Apple silicon and Intel; Windows ships only the
  per-user installer (no MSI).

### Fixed
- A file that became locked or was moved, and hasn't been read again, no longer shows its old
  text anywhere: not in search, not to your AI app, not in PageLamp's own AI features.
- A course that PageLamp lists as ended, inactive or not started no longer reports a current
  week. Without usable dates, the week number of the last material posted (even years ago)
  was shown as the current week in the app, `pagelamp courses` and to your AI app
  (`list_courses`, `course_overview`, `week_materials`); these now say "Ended", "Inactive" or
  "Starts …" instead. Asking for a week by its number still works, and "I'm still taking this"
  brings the week back. A course PageLamp can't place yet (no dates, but something happened
  in it recently) can still show a week from its latest week-numbered material, marked low
  confidence.

## [0.1.0] — 2026-09-28

First public release.

### Added
- Local course knowledge base (SQLite + full-text search) with text extraction for PDF, PowerPoint
  (.pptx), Word (.docx), Jupyter notebooks, Markdown, HTML, plain text and source code; citations point to page,
  slide, cell or section.
- Sources: course folder (one sub-folder per course, week folders, optional `course.toml`),
  calendar feed (iCal/webcal, e.g. Canvas Calendar Feed), and Canvas with a personal access token
  (read-only; personal use only; files downloaded only on request).
- "Which week is this course in" inference with confidence and evidence.
- MCP server (`pagelamp mcp`) with 9 read-only tools plus `save_study_plan` (saves only to your local database), and the
  `weekly_review`, `catch_up` and `study_plan` prompts, for Claude Desktop, ChatGPT desktop /
  Codex and Claude Code.
- Per-course AI policy and AI-access switch; material text is withheld for "No AI" courses.
- `pagelamp` CLI and the desktop app (onboarding, sources & sync, courses, course detail,
  "Connect your AI app", settings), in English and 简体中文, light and dark.
- Signed and notarized macOS builds: the app (`.dmg`) and the standalone `pagelamp` binary are
  signed with a Developer ID and notarized by Apple (the ticket is stapled to the `.dmg` and the
  app), so they open without the System Settings → Privacy & Security steps.
- Signed Windows builds: the installers (`-setup.exe` and `.msi`), the app with its `pagelamp.exe`,
  the uninstaller and the standalone `pagelamp.exe` are signed with Azure Artifact Signing and
  timestamped, so Windows shows a verified publisher instead of "Unknown publisher".

### Upgrading from an earlier build
- After installing a new version, quit your AI app completely and open it again so it starts the new
  `pagelamp` (on macOS, open PageLamp once first so macOS lets the new build run).

### Known limitations
- On Windows, SmartScreen may still warn about a new release until the signing certificate has
  built up reputation (see the README). Linux builds are not code-signed; check downloads against
  `SHA256SUMS`.
- Canvas can report a course's term much wider than its classes (University of Toronto's Fall term,
  for example, runs from May to January), so PageLamp may show the wrong week, such as "Week 22"
  in late September. Fix it in the course's **Timeline** tab (*Wrong week? Set this course's term
  dates*). v0.3 will work the week out from the syllabus, the schedule and the published notes.
- Courses from past terms keep syncing when their instructor never closed them in Canvas. Hide them
  (course → *Settings* → *Hide this course*): hidden courses stay hidden after each sync and are never
  shown to your AI app. Removing finished courses is planned for v0.3.
- No reminders/notifications yet (planned for v0.3).
- Canvas access uses a personal token until an institution-approved sign-in is available.
- On macOS and Windows the desktop app doesn't put `pagelamp` on your PATH (use the full path shown
  in *Connect your AI app*); the Linux `.AppImage` can't serve your AI app — use the `.deb`/`.rpm`.
- Older `.ppt`/`.doc` files and scanned PDFs (no text layer) are listed but not searchable.
- Windows and Linux builds are x86_64 only.
- Text extraction runs in the sync process. PDFs are checked for decompression bombs and their image
  data is never decoded, but page content is still parsed in memory and LZW-only streams aren't
  measured, so a very dense or deliberately crafted PDF can use a lot of memory (an isolated
  extraction worker is planned for v0.2).
- Installer file names and the OS-level app version show `0.1.0` for every 0.1.0 beta; the real
  version is in *Settings → About* or `pagelamp --version`.
