# Privacy

*Last updated: 2026-10-03 · applies to PageLamp v0.1. Items marked **(v0.3)** describe the v0.3
pre-releases and later; v0.1.0 doesn't have them.*

PageLamp is a local app. There is no PageLamp server, account, analytics or telemetry. The people
who build PageLamp never receive your data.

## What PageLamp stores, and where

| Data | Where | Why |
|---|---|---|
| Course list, modules, material titles and text, announcements, deadlines, your study plan, your per-course settings (AI policy, AI access, term dates, hidden) | a SQLite database in your data folder (macOS `~/Library/Application Support/dev.PageLamp.PageLamp`, Windows `%APPDATA%\PageLamp\PageLamp\data`, Linux `$XDG_DATA_HOME/pagelamp`, or `PAGELAMP_HOME`) | so your AI app can answer questions about your courses |
| Files you ask PageLamp to download from Canvas | the `files/` folder next to the database | so their text can be indexed |
| Canvas access token, calendar-feed link | your operating system's keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service) — never in the database, logs or AI output | to sync on your behalf |
| **(v0.3)** Your update settings, the result of the last update check, and the version you last ran | the same database | to know when the next check is due and to show "What's new" once |
| **(v0.3)** Your automatic sync setting, and when PageLamp last synced or tried to | the same database | to know when the next sync is due |
| **(v0.3)** A backup of the database, made before an update changes its format | `pagelamp.db.v<N>.bak` next to the database, readable only by you; only the newest is kept, and it is deleted when you remove your last source | so a failed update can be undone. It holds the same course data as the database |

PageLamp does **not** store assignment instructions or submissions — only assignment titles, due
dates and links. It never reads your university password.

## What leaves your computer

- **Sync:** PageLamp connects to your LMS (read-only requests with your own token) and/or
  downloads your calendar feed. Course folders are read locally. A sync runs when you press Sync
  (or run `pagelamp sync`).
- **(v0.3) Automatic sync:** while the PageLamp app is running, it also syncs by itself, twice a
  day unless you choose once a day or off (see below). A sync that fails is tried again later. It
  never downloads files.
  - When nobody is at the app, PageLamp checks your token and asks Canvas only for your course
    list (with each course's syllabus), your deadlines and your courses' announcements. It makes
    no request for a course's modules, pages, file list or assignments. Canvas keeps its own
    records, and we can't promise that these requests leave none.
  - When you open PageLamp, bring it to the front or change this setting, and the last full sync
    is old enough or a newly found course hasn't been read yet, it runs the same sync as the Sync
    button. Canvas may record a full sync as your activity in each course, as it would if you
    pressed Sync yourself.
  - PageLamp's MCP server gives your AI app no way to start a sync and tells it not to run one
    for you. PageLamp doesn't sync by itself while its app is closed.
- **When you ask your AI app a question:** your AI app (Claude, ChatGPT, Codex, …) reads the course
  information it needs from PageLamp on your computer and sends it to that AI provider **under your
  own account and that provider's terms**. What the provider stores or uses for training depends on
  your account settings with them — check them.
- **(v0.3) Update check:** once a day PageLamp downloads a small file from GitHub to see whether
  there's a new version. GitHub sees your IP address and your PageLamp version, as with any
  download; nothing about your courses is sent. The request carries only `User-Agent:
  PageLamp/<version>` and standard `Accept` headers: no cookies, and nothing about you or your
  computer in the address. PageLamp always asks before installing an update, which it then
  downloads from GitHub too. On Linux `.deb`/`.rpm` installs it only shows a download link. You can
  turn the daily check off (see below).
- Nothing else. PageLamp never contacts any other server.

## Your controls

- **Per course:** turn off *"Let my AI app read this course's materials"*, or mark the course's AI
  policy as "No AI" — PageLamp then shares no material text for that course (deadlines, structure
  and your study plan remain available for planning). CLI: `pagelamp course ai-access <course> off`.
- **Hide a course** to keep it out of your AI app entirely: `pagelamp course hide <course>`.
- **(v0.3) Automatic sync:** *Sources & sync → Automatic sync* (Off, Once a day, Twice a day).
  With it off, PageLamp syncs only when you press Sync. CLI: `pagelamp sync --auto off`.
- **(v0.3) Update checks:** *Settings → Updates → Check for updates automatically*. With it off,
  PageLamp never checks on its own; *Check now* still works.
- **Remove a source** (`pagelamp sources remove <id>` or *Sources & sync → Remove*) deletes what
  was synced from it — its courses with your AI-policy and term settings for them (course folder
  and Canvas), its deadlines and events (calendar feed), and for Canvas the course files you
  downloaded — plus its token or feed link in the keychain. Your own course folder and anything in
  Canvas or your LMS calendar are never changed.
- **Delete everything:** remove each source first (so its token or feed link is deleted from the
  keychain), then quit PageLamp and delete the data folder above. The desktop app also keeps its
  display preferences (theme, language, onboarding done) in its own app storage under
  `dev.pagelamp.desktop`; they contain no course data.

## AI disclosure

PageLamp does not generate content itself in v0.1. Text your AI app produces is AI-generated by
that app. PageLamp tells your AI app to cite course sources, tutor rather than produce graded work,
and respect each course's AI policy — but you are responsible for following your course and
university rules.

## Logs and diagnostic reports

PageLamp writes log files only on your computer, in the `logs/` folder of your data folder, and
deletes them after 7 days. Logs record what PageLamp did (e.g. "synced 5 courses", request paths and
status codes) — never tokens, calendar-feed links, signed download links or course text; your home
folder is shown as `~`. A diagnostic report (*Copy diagnostic report* in the app, or
`pagelamp report`) is created only when you ask for it, shows you its full content first, and
replaces course names with "Course 1", "Course 2". From v0.3 it also lists the result of the last
update check and of the last database update and backup, as codes only. Nothing is ever sent automatically — you decide
whether to share a report, e.g. in a GitHub issue.

To replace names of courses you have since renamed or removed, PageLamp keeps a small list of course
names, codes and folder names in `course-aliases.json` in your data folder (readable only by you,
never included in a report). Each entry is deleted 30 days after the course was last seen.

## Questions

Open an issue at https://github.com/Euswbnix/pagelamp/issues (don't include personal data), or see
[SECURITY.md](SECURITY.md) for security reports.
