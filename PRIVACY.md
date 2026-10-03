# Privacy

*Last updated: 2026-09-30 · applies to PageLamp v0.1. Items marked **(v0.3)** describe the v0.3
pre-releases and later; v0.1.0 doesn't have them.*

PageLamp is a local app. There is no PageLamp server, account, analytics or telemetry. The people
who build PageLamp never receive your data.

## What PageLamp stores, and where

| Data | Where | Why |
|---|---|---|
| Course list, modules, material titles and text, announcements, deadlines, your study plan, your per-course settings (AI policy, AI access, term dates, hidden; **(v0.3)** whether its materials may be shared with AI services) | a SQLite database in your data folder (macOS `~/Library/Application Support/dev.PageLamp.PageLamp`, Windows `%APPDATA%\PageLamp\PageLamp\data`, Linux `$XDG_DATA_HOME/pagelamp`, or `PAGELAMP_HOME`) | so your AI app can answer questions about your courses |
| Files you ask PageLamp to download from Canvas | the `files/` folder next to the database | so their text can be indexed |
| Canvas access token, calendar-feed link | your operating system's keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service) — never in the database, logs or AI output | to sync on your behalf |
| **(v0.3)** Your update settings, the result of the last update check, and the version you last ran | the same database | to know when the next check is due and to show "What's new" once |
| **(v0.3)** A backup of the database, made before an update changes its format | `pagelamp.db.v<N>.bak` next to the database, readable only by you; only the newest is kept, and it is deleted when you remove your last source | so a failed update can be undone. It holds the same course data as the database |
| **(v0.3)** For each course you removed: its name and code, its ID at its source (the Canvas course ID or the folder's name) and which source it came from, why and when you removed it, when its data is to be or was deleted, your choices for deleting it, whether its downloaded files still wait for the Trash, whether Canvas restricts access to it by date, and your settings for it (AI policy and its note, AI access, your answer about sharing its materials with AI services, whether it was hidden, the dates you set, and its calendar's dates without break labels or week topics); no material text | the same database, until you undo the removal, a restore brings the course back, you choose *Forget*, or you remove its source | so a sync doesn't add the course back, and restoring it brings your settings back |
| **(v0.3)** The model providers you add (name and address), which model each AI feature uses, your monthly AI budget, and which AI disclosures you have read | the same database | to run PageLamp's AI features the way you chose |
| **(v0.3)** API keys for model providers | your operating system's keychain — never in the database, logs or AI output | to use the provider on your behalf |
| **(v0.3)** What PageLamp's AI features wrote for you, with its label (provider, model, date) and which materials were sent (their ids, never their text) | the same database; only the latest few of each kind are kept | so you can read it again and see where it came from |
| **(v0.3)** One line per AI run: when, provider, model, feature, tokens and estimated cost | the same database, deleted after 13 months | for the usage and budget screens |
| **(v0.3)** Your reminder settings (which reminders, on which day and at what time, and your answer to *Keep PageLamp in the tray and start it at login*) and the reminders already shown, with when each was shown (each by an id made of the reminder's kind and a deadline's id and due time, or of a date) | the same database; shown ids more than 60 days old are removed the next time a reminder is marked shown | so each reminder is shown once |
| **(v0.3)** Whether to prepare your weekly note on Mondays, and the last Monday PageLamp tried to prepare it by itself | the same database | so PageLamp tries it by itself at most once a Monday |
| **(v0.3)** A login item, if you turn on *Keep PageLamp in the tray and start it at login* | macOS `~/Library/LaunchAgents/dev.pagelamp.desktop.plist`; Windows the `PageLamp` value in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (Windows may also keep a `PageLamp` value under `…\Explorer\StartupApproved\Run`); Linux `~/.config/autostart/PageLamp.desktop` | to start PageLamp in the background when you log in |

PageLamp does **not** store assignment instructions or submissions — only assignment titles, due
dates and links. It never reads your university password.

## What leaves your computer

- **Sync (only when you start it):** PageLamp connects to your LMS (read-only requests with your
  own token) and/or downloads your calendar feed. Course folders are read locally.
- **When you ask your AI app a question:** your AI app (Claude, ChatGPT, Codex, …) reads the course
  information it needs from PageLamp on your computer and sends it to that AI provider **under your
  own account and that provider's terms**. What the provider stores or uses for training depends on
  your account settings with them — check them.
- **(v0.3) When you use PageLamp's own AI features** (for example *Read the syllabus*): PageLamp
  sends the model you chose the course information that feature needs, only when you start it, and
  only after you have read what that model receives.
  - With an API key, it goes to that provider (for example OpenAI or Anthropic) under your API
    account and that provider's terms. What the provider keeps or uses for training depends on those
    terms and your settings with them.
  - With a model on this computer (for example Ollama or LM Studio), it stays on your computer —
    unless you set the model's address to another computer, which then receives it.
  - *Read the syllabus* sends the text of the course materials it reads. No model gets the materials
    of a course whose AI access you turned off or whose AI policy is "No AI", and a model in the
    cloud doesn't get them if you answered "No, it's not allowed" to *Is sharing this course's
    materials allowed?*. If you haven't answered that question, or answered "Not sure", PageLamp
    reminds you once per course (again after *Remove all AI data*), after the first reading that
    sent that course's materials to a model in the cloud.
  - *Plan your study* sends the structure of your courses, never material text: for each visible,
    active course (or each one you pick), its name, whether its AI settings let AI read its
    materials, its current week, the titles and kinds of this and next week's materials (or of the
    last 14 days' materials when its weeks aren't known), and the titles, kinds and dates of its
    calendar items (deadlines, exams, classes) in the days you plan, with PageLamp's ids for the
    course and those materials, which include your Canvas site's address or your folder and file
    names. Courses whose AI access is off, whose AI policy is "No AI" or that you answered "No, it's
    not allowed" for are planned from this structure too; hidden and removed courses are left out.
    What you type in *Anything to focus on?* (up to 500 characters) is sent with it and not kept.
  - *Explain a week* sends the text of one week of a course's materials (or of its last 14 days'
    materials when the course's weeks aren't known), with the course's name, the week number and the
    materials' titles. It never runs for a hidden course or a course whose AI access you turned off
    or whose AI policy is "No AI", nor with a model in the cloud for a course you answered "No, it's
    not allowed" for. It leaves out links to other sites, materials without readable text, materials
    whose titles look like graded work (assignments, quizzes, exams…) unless you include them, and
    whatever goes over its length limit. The one-time sharing reminder above can come after an
    explanation instead, if it sent that course's materials to a model in the cloud first.
  - The *Weekly note* sends the same kind of structure, never material text, whatever each course's
    AI settings: your visible, active courses (with where each is in its term, such as a break), the
    calendar items (deadlines, exams, classes) in the next 7 days of your other visible courses, and
    your study plan's progress (how many of the last 7 days' items you ticked, and today's items'
    titles and whether they're ticked). You start it with *Write my weekly note* (*Write a new note*
    once you have one). If you turn on *Prepare my weekly note when I open PageLamp on Monday*,
    which works only with a model you added yourself (an API key or a model on this computer),
    PageLamp also starts it itself on Mondays while it runs (at launch, or within the hour if it's
    already running), unless you already wrote one that day: one try a Monday whatever the result,
    and never when its estimated cost would take you over your monthly budget (a model without a
    known price isn't limited by the budget).
  - Listing a provider's models asks that provider for them, and *Test* sends it one tiny request.
- **(v0.3) Reminders** stay on your computer: PageLamp works them out from what it already has, with
  no model and no network request, and shows them as notifications only if you turned them on (see
  below). A reminder notification holds only a course code (or the course's name if it has no code)
  with a deadline's title and the day and time it is due, or a count of deadlines or of today's plan
  items — never the text of your materials. The one notification you get when you turn reminders on
  only says that they are on. The tray icon and the login item make no network requests of their
  own.
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
- **(v0.3) AI:** in *Settings → AI models*, remove a provider (its key is deleted from the
  keychain), choose which model each feature uses, and set a monthly budget. For each course you
  can answer *Is sharing this course's materials allowed?*. *Remove all AI data* deletes your API
  keys, your model choices, the usage history, the AI settings, everything PageLamp's AI wrote, and
  the pre-update backup of the database (which may hold them); your courses and your answers about
  sharing stay.
- **(v0.3) Reminder notifications, the tray icon and starting at login:** off until you answer *Yes,
  remind me* in onboarding or turn on *Settings → Reminders → Keep PageLamp in the tray and start it
  at login*. Until then, reminders that came due show only inside PageLamp, under *Since you last
  opened PageLamp*. With it on, PageLamp shows a tray icon (on Linux only if an AppIndicator library
  is installed), closing its window keeps it running, and a login item (see the table above) starts
  it in the background, in the tray, when you log in. You can then turn reminders for deadlines,
  your week and today's study plan on or off one by one (the first two are on by default). Turning
  the switch off stops the notifications and removes the tray icon and the login item; from the next
  time you open PageLamp, reminders that are still due show only inside it, under *Since you last
  opened PageLamp*. Hidden and removed courses get no deadline reminders. Uninstalling PageLamp
  removes the login item on Windows. On macOS and Linux it stays behind: turn the switch off before
  you delete PageLamp, or delete that file yourself.
- **(v0.3) Plans, explanations and weekly notes:** *Delete…* removes one explanation or weekly
  note. Turn off *Settings → Weekly note → Prepare my weekly note when I open PageLamp on Monday*
  to stop the Monday note. A plan you chose to use (*Use this plan*) becomes your study plan:
  *Remove all AI data* keeps it, with its AI label. Removing a course doesn't delete the weekly
  notes or plan drafts that mention it.
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

PageLamp does not generate content itself in v0.1. **(v0.3)** It can also write with a model you
choose (for example, reading a syllabus for the course's dates): what it writes is labelled
AI-generated, with the provider, the model and the date, and a syllabus reading is a proposal you
check before accepting it. **(v0.3)** Study plans, explanations and weekly notes carry the same
label, and PageLamp drops plan tasks and weekly-note focus items whose wording looks like doing
graded work (such as "Write A2's answers"). Text your AI app produces is AI-generated by that app.
PageLamp tells your AI app to cite course sources, tutor rather than produce graded work, and
respect each course's AI policy — but you are responsible for following your course and university
rules.

## Logs and diagnostic reports

PageLamp writes log files only on your computer, in the `logs/` folder of your data folder, and
deletes them after 7 days. Logs record what PageLamp did (e.g. "synced 5 courses", request paths and
status codes) — never tokens, API keys, calendar-feed links, signed download links, course text or
what was sent to a model; your home
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
