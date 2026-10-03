# Privacy

*Last updated: 2026-09-30 · applies to PageLamp v0.1. Items marked **(v0.3)** describe the v0.3
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
| **(v0.3)** A backup of the database, made before an update changes its format | `pagelamp.db.v<N>.bak` next to the database, readable only by you; only the newest is kept, and it is deleted when you remove your last source | so a failed update can be undone. It holds the same course data as the database |
| **(v0.3)** For each course you removed: its name and code, its ID at its source (the Canvas course ID or the folder's name) and which source it came from, why and when you removed it, when its data is to be or was deleted, your choices for deleting it, whether its downloaded files still wait for the Trash, whether Canvas restricts access to it by date, and your settings for it (AI policy and its note, AI access, your answer about sharing its materials with AI services, whether it was hidden, the dates you set, and its calendar's dates without break labels or week topics); no material text | the same database, until you undo the removal, a restore brings the course back, you choose *Forget*, or you remove its source | so a sync doesn't add the course back, and restoring it brings your settings back |

PageLamp does **not** store assignment instructions or submissions — only assignment titles, due
dates and links. It never reads your university password.

## What leaves your computer

- **Sync (only when you start it):** PageLamp connects to your LMS (read-only requests with your
  own token) and/or downloads your calendar feed. Course folders are read locally.
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
- **(v0.3) Update checks:** *Settings → Updates → Check for updates automatically*. With it off,
  PageLamp never checks on its own; *Check now* still works.
- **Remove a source** (`pagelamp sources remove <id>` or *Sources & sync → Remove*) deletes what
  was synced from it — its courses with your AI-policy and term settings for them (course folder
  and Canvas), its deadlines and events (calendar feed), and for Canvas the course files you
  downloaded — plus its token or feed link in the keychain. Your own course folder and anything in
  Canvas or your LMS calendar are never changed.
- **(v0.3) Remove a course** (*Remove from PageLamp…* on the course's *Settings* tab, or
  `pagelamp course remove <course>`) takes it out of every list and your AI app at once and stops
  syncing it. Unless you choose *Delete now*, nothing is deleted for 7 days, and *Undo* (also in
  *Settings → Removed courses*, or `pagelamp course restore <id>`) puts it back. After that, at the
  next sync (or with `pagelamp course purge`), PageLamp deletes the course's data: its modules,
  materials and announcements with their text and search index, the deadlines synced with it, its
  calendar, and what PageLamp's AI features wrote for that course alone (such as explanations and
  calendar proposals). Calendar-feed events linked to it only lose that link, and your study plan
  keeps its items for the course, hidden while PageLamp remembers the removal. For Canvas courses,
  the files you downloaded go to the system Trash (the Recycle Bin on Windows) unless you tick *Keep
  downloaded files*; if that fails, they stay where they are and PageLamp tries again at each sync.
  PageLamp deletes them for good only if you choose *Delete files permanently* after a move to the
  Trash failed (or `pagelamp course purge --permanent`), or if you remove the course's Canvas source
  before its data is deleted, since removing a source deletes the files downloaded for it. Your own
  course folder and anything in Canvas are never changed. *Also delete the pre-update backup*
  deletes that backup for good when the course's data is deleted (an undo keeps it). The backup is a
  copy of your whole database from before the last update, so it holds every course you had then,
  not just this one; the box is ticked by default once the backup is 14 days old. A small record of
  the course stays (see the table above) until you choose *Forget*, a restore brings the course
  back, or you remove its source; after *Forget*, the next sync adds the course back if its source
  still lists it. *Restore* tries to bring a deleted course back by syncing it again and puts your
  saved settings back; it can't if Canvas no longer lists the course or restricts access to it, or
  while you're offline. Its files come back as not downloaded, and announcements older than 120 days
  don't come back.
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
