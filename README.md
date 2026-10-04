# 💡 PageLamp

**A reading lamp for your courses — so your AI app knows what you're learning.**

[![CI](https://github.com/Euswbnix/pagelamp/actions/workflows/ci.yml/badge.svg)](https://github.com/Euswbnix/pagelamp/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

> **v0.1.0 is PageLamp's first public release.** It doesn't update itself: when a new version is
> out, install it from the [releases page](https://github.com/Euswbnix/pagelamp/releases/latest)
> (on macOS, open PageLamp once), then quit your AI app completely and open it again. Known issues
> in v0.1.0 — a wrong week for some Canvas terms, and old courses that keep syncing — have
> workarounds in the [release notes](https://github.com/Euswbnix/pagelamp/releases/tag/v0.1.0).
> Please [report anything odd](https://github.com/Euswbnix/pagelamp/issues/new/choose).

![PageLamp demo](docs/assets/demo.gif)

PageLamp keeps a local, searchable picture of your courses — which week each course is in, this
week's materials, upcoming deadlines — and hands it to the AI app you already use (Claude Desktop,
Claude Code, Codex, or ChatGPT desktop in Work/Codex mode) through
[MCP](https://modelcontextprotocol.io). Ask *"What's happening in my courses this week?"*,
*"Explain this week's lecture and cite the slides"*, or *"Make me a study plan for the next two
weeks"* — without re-uploading anything.

[中文说明 ↓](#中文说明)

## Principles

- **Local-first.** Your course data lives in a database on your computer. There is no PageLamp
  server online (its MCP server runs only on your computer, started by your AI app), no account,
  and nothing is collected. See [PRIVACY.md](PRIVACY.md).
- **Your AI, your account.** In v0.1 PageLamp calls no AI model itself: your own AI app reads your
  local course data when you ask it something. PageLamp never resells or pays for AI usage, and
  never reads, stores or sends the credentials (passwords or tokens) you use to sign in to your AI
  account.
- **Read-only and learning-first.** PageLamp never submits, posts or marks anything, and never
  fetches assignment instructions to solve them. It tells your AI app to tutor, cite its sources and
  respect each course's AI policy — and for courses you mark "No AI", it doesn't share the materials.
- **Not tied to one LMS.** A course folder plus your LMS's calendar feed (iCal) works with most
  systems. If your school uses Canvas, you can also sync from it with a personal access token you
  create in Canvas ([how](#canvas-access-tokens)).

## Install

### Desktop app (recommended)

Download the installer for your system from the
[latest release](https://github.com/Euswbnix/pagelamp/releases/latest): for macOS the `.dmg`
(`aarch64` = Apple silicon, `x64` = Intel), for Windows the `-setup.exe` (or the `.msi`), for
Linux the `.deb`, `.rpm` or `.AppImage` (Windows and Linux: x86_64 only). The desktop app includes
the `pagelamp` command-line tool your AI app needs.

- **macOS:** open the `.dmg` and **drag PageLamp into Applications**, then open it from there —
  your AI app is pointed at that location, so don't run PageLamp from the disk image or Downloads.
  The app is signed with a Developer ID and notarized by Apple, so it opens like any other app: the
  first time, macOS asks whether to open an app downloaded from the internet — click **Open**.
- **Windows:** the installers, the app and the `pagelamp.exe` inside it are signed (Azure Artifact
  Signing), so Windows shows the maintainer as the verified publisher. SmartScreen may still show
  "Windows protected your PC" for a new release until the signing certificate has built up
  reputation: click **More info**, check that the publisher shown is the maintainer — the same
  verified publisher on every PageLamp release, not "Unknown publisher"
  ([how to check](SECURITY.md#verifying-downloads)) — then click **Run anyway**. The `-setup.exe`
  installs for your account only (no administrator rights needed).
- **Linux:** the `.deb`/`.rpm` also install `pagelamp` as `/usr/bin/pagelamp`. The `.AppImage` runs
  the app, but your AI app can't use the `pagelamp` inside it (the AppImage moves to a new place
  every time it starts) — use the `.deb`/`.rpm` or the command-line archive for that. Linux
  packages aren't code-signed: check them against `SHA256SUMS`
  ([how](SECURITY.md#verifying-downloads)). On Linux, saving a Canvas token or calendar-feed link
  (in the app or the command-line tool) needs a desktop keyring (GNOME Keyring or KWallet); course
  folders work without one.

On macOS and Windows the desktop app doesn't add `pagelamp` to your PATH. You don't need it to
connect your AI app — *Connect your AI app* shows the full path. To run the `pagelamp …` commands
in this README, use that full path — on macOS, for example,
`/Applications/PageLamp.app/Contents/MacOS/pagelamp doctor`.

### Command-line tool only

**From a release.** Download the archive for your system from the
[latest release](https://github.com/Euswbnix/pagelamp/releases/latest), unpack it, and put
`pagelamp` (`pagelamp.exe` on Windows) in a folder on your PATH (e.g. `~/.local/bin`). On macOS the
binary is signed and notarized; macOS checks that with Apple the first time it runs, so be online
for that first run. On Windows `pagelamp.exe` is signed. Check with `pagelamp --version`. On
Linux, saving a Canvas token or calendar-feed link needs a desktop keyring (GNOME Keyring or
KWallet); course folders work without one.

**From source.** You need Rust 1.90 or newer ([rustup.rs](https://rustup.rs)) and a C compiler
(Xcode Command Line Tools on macOS, Visual Studio Build Tools on Windows, `build-essential` on
Linux). On Windows also install NASM, or set `AWS_LC_SYS_PREBUILT_NASM=1`. Then:

```bash
git clone --branch v0.1.0 https://github.com/Euswbnix/pagelamp
cd pagelamp
cargo install --path apps/pagelamp-cli --locked
```

This builds the released version (use the tag of the latest release) and installs `pagelamp` into
`~/.cargo/bin`. `main` is the development branch: a build from it may move your database to a
newer format that the released app can't open (a backup is kept next to it).

## Quick start

1. **Put your course files in one folder**, one sub-folder per course. Week folders help PageLamp
   tell which week each course is in:

   ```
   Courses/
     CHEM101 General Chemistry/
       course.toml          ← optional: term_start = 2026-09-08
       Week 1/  lecture01.pdf  notes.md
       Week 2/  lecture02.pptx
     MATH102 Calculus II/
       Week 1/ …
   ```

   PDF, PowerPoint (`.pptx`), Word (`.docx`), Jupyter notebooks, Markdown, text, HTML and source
   code are indexed. Older `.ppt`/`.doc` files and scanned PDFs without a text layer are listed but
   not searchable.

2. **Add your calendar feed** for deadlines (optional): in Canvas, open **Calendar → Calendar Feed**
   and copy the link; other LMSs call it "export calendar" or iCal. It's private — PageLamp keeps it
   in your system keychain.

3. **Add them and sync** — in the desktop app's onboarding, or:

   ```bash
   pagelamp folder add ~/Courses --term-start 2026-09-08
   pagelamp ical add            # paste the feed link when asked (optional)
   pagelamp canvas add --base-url https://canvas.school.edu   # optional, your own token only
   pagelamp sync
   pagelamp courses
   ```

4. **Connect your AI app.** Open *Connect your AI app* in the desktop app, or, with the
   command-line tool, run `pagelamp mcp-config claude-desktop` (also `claude-code`, `codex`, or
   `generic` for any other MCP client that can start a local server), and follow the steps. For
   Claude Desktop, **quit it completely before editing its config file** — it saves over the file
   when it quits, so changes made while it's open are lost.
   Then restart your AI app and ask: *"Using PageLamp, where is each of my courses this week?"*

| AI app | How it connects | Plans |
|---|---|---|
| Claude Desktop | local MCP server in `claude_desktop_config.json` | every Claude plan, including Free |
| ChatGPT desktop (Work/Codex mode), Codex CLI | `[mcp_servers.pagelamp]` in `~/.codex/config.toml` | Codex is included in every ChatGPT plan, including Free and Go (limits vary by plan) |
| Claude Code | the `claude mcp add …` command printed by `pagelamp mcp-config claude-code` | a paid Claude plan (Pro or higher; not Free) |

This table is about your own AI app reading PageLamp over MCP; PageLamp doesn't run any of these
apps. Plans are set by each vendor and can change — check their plans pages.

### Canvas access tokens

To sync straight from Canvas, create a personal access token in Canvas:

1. In Canvas, open **Account → Settings**, scroll to **Approved Integrations** and click
   **+ New Access Token**.
2. Fill in the purpose (e.g. "personal study planner, read-only") and pick an expiry date. Canvas
   shows the longest expiry it allows.
3. Click **Generate Token** and copy the token right away: Canvas shows it only once.
4. In PageLamp, open **Sources & sync → Add source → Canvas access token**, enter your Canvas address
   (the address you open Canvas at, e.g. `https://canvas.school.edu`) and paste the token. With the
   command-line tool, `pagelamp canvas add --base-url https://canvas.school.edu` asks for it.
   PageLamp checks the token with Canvas, then stores it in your system keychain.

If you don't see **+ New Access Token**, your school has turned off student tokens — use the course
folder + calendar feed instead. The token acts as you in Canvas, so keep it to yourself. When it
expires, create a new one and use **Replace token** on the source; to end PageLamp's access, delete
the token on the same Canvas page.

Canvas sync never downloads files unless you ask, because downloads can count as "viewed" in
module requirements. Canvas records a sync like any other access: your course access report (which
instructors can see) may list each area PageLamp reads — modules, pages, assignments, files and
announcements — plus each page it reads (only new or changed ones). Reading doesn't complete module
requirements; downloading files can.

(v0.3) While the PageLamp app is running it also syncs by itself, twice a day unless you choose
once a day or off. With nobody at the app it checks your token and asks Canvas only for your
course list (with each course's syllabus), your deadlines and your courses' announcements; it
makes no request for a course's modules, pages, file list or assignments. The full sync described
above runs when you start a sync, or when you open PageLamp or bring it to the front and the last
full sync is old enough (or a newly found course hasn't been read yet). An automatic sync never
downloads files.

### Course AI policies

Many universities don't allow generative AI in a course unless the instructor permits it — check
your syllabus. Record each course's policy in PageLamp (`pagelamp course policy CHEM101
learning-aid`, or the course's *AI policy* tab; values: `unknown`, `prohibited`, `learning-aid`,
`allowed-with-citation`, `unrestricted`). Your AI app sees it and adjusts; for courses marked
`prohibited` ("No AI"), PageLamp doesn't share the materials' text — titles, deadlines and your
study plan stay available for planning. You can also turn sharing off for any course
(`pagelamp course ai-access CHEM101 off`, or the switch on the course's *AI policy* tab).

## Troubleshooting and reporting problems

**Desktop app:** *Settings → Help & feedback → Copy diagnostic report* shows you the report first;
check it, then paste it into a [GitHub issue](https://github.com/Euswbnix/pagelamp/issues/new/choose).
If PageLamp closed unexpectedly, the next launch offers the same button.

**Command line:** `pagelamp doctor` checks your setup (data folder, database, keychain, sources,
AI apps). `pagelamp report --out pagelamp-report.md` writes a report to attach to an issue — read
it first. Reports contain your PageLamp version, OS, setup checks and recent log lines; course names
are replaced by "Course 1", "Course 2", and tokens, calendar-feed links and course text are removed.

**Logs** stay on your computer, in the `logs/` folder of your data folder (kept for 7 days) — open
it with *Settings → Help & feedback → Open logs folder*, or see the path printed by `pagelamp doctor`.
If your AI app can't reach PageLamp, also check the AI app's own logs — for Claude Desktop on macOS:
`~/Library/Logs/Claude/mcp.log` and `~/Library/Logs/Claude/mcp-server-pagelamp.log`.

### Canvas sync

If a Canvas sync fails or looks incomplete, run it again with diagnostics:

```bash
pagelamp sync -v
```

(or set `PAGELAMP_LOG=debug`). You'll see one line per Canvas request — for example
`GET /api/v1/courses/1234/modules → 200 (85 ms, rate limit remaining 690)` — plus Canvas's own error
message when a request fails. At the end, a per-course summary shows what was read.

- **"Canvas rejected the access token"** — the token expired or was revoked. Create a new one
  (Canvas: Account → Settings → Approved Integrations → + New Access Token) and run
  `pagelamp sources update-secret <source id>` (see `pagelamp sources`), or use *Replace token* in the
  desktop app.
- **"… not available (not available to you in Canvas)"** — that part of the course is hidden from
  students (for example the Files tab). PageLamp uses what is visible and keeps what it already has.
- **"Canvas kept throttling requests"** — wait a few minutes and sync again.

**Never paste** your Canvas access token, your calendar feed link, or copies of course materials into
an issue — even if something seems to be missing from the report.

## How it works

```
your AI app (Claude Desktop / Claude Code / Codex / ChatGPT desktop in Work/Codex mode)
        │  local MCP (stdio)
        ▼
pagelamp mcp  ──reads──▶  pagelamp.db (on your computer)  ◀──writes── pagelamp sync
                                                               ▲
                              course folder · calendar feed · Canvas (read-only)
```

`pagelamp mcp` never connects to Canvas or the internet: it reads the database on your computer
and writes to it only when your AI app asks it to save something for you (in v0.1, a study plan).
Syncing is a separate step (with your own token for Canvas): you start it, or (v0.3) the PageLamp
app does while it runs, under your setting. The MCP server gives your AI app no way to start a
sync and tells it not to run one for you.

- `crates/` — Rust core: data model and SQLite store, text extraction, Canvas / folder / iCal
  sources, the MCP server, the shared layer the command-line tool and apps call, and model access
  for v0.3 (`pagelamp-llm`, in development).
- `apps/pagelamp-cli` — the `pagelamp` command.
- `apps/desktop` — Tauri 2 + React desktop app (the one in the releases).
- `apps/macos` — native macOS app, a developer preview that isn't in the releases.
- `docs/` — [architecture](docs/ARCHITECTURE.md) and design notes.

## Roadmap

- **v0.1** (released September 2026): your AI app reads your courses over MCP; signed builds for
  macOS and Windows.
- **v0.3** (in development) is planned to add automatic updates, course weeks that follow the real
  teaching dates, finished courses grouped under "Past" and removable (with undo), weekly
  reminders, and PageLamp itself writing study plans, plus weekly explanations that cite your
  course materials, using a model you choose: your own API key or a model running on your
  computer. With a model, it can also read a course's syllabus
  and suggest dates for you to confirm. Connecting your own AI app over MCP stays available.

PageLamp's own features use your API key or a model on your computer; they don't run on a
ChatGPT plan. To use a ChatGPT plan with PageLamp, connect Codex or ChatGPT desktop as your AI app
over MCP (see the table above). Whether a Claude plan could be used for these features is
something we're asking Anthropic; it will be offered only if they confirm in writing that it's
allowed. Plans can change before a release — the [CHANGELOG](CHANGELOG.md) lists what is already
done.

## Contributing

Bug reports, ideas and pull requests are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md).
Security issues: see [SECURITY.md](SECURITY.md).

## 中文说明

> v0.1.0 是 PageLamp 的第一个正式版本，还不会自动更新：有新版本时从 [Releases](https://github.com/Euswbnix/pagelamp/releases/latest) 下载安装（macOS 上先打开一次 PageLamp），然后完全退出并重新打开你的 AI 应用。v0.1.0 的已知问题（部分 Canvas 学期周数不对、往期课程仍被同步）及临时解决办法见[发布说明](https://github.com/Euswbnix/pagelamp/releases/tag/v0.1.0)。

PageLamp（"读书灯"：为每门课点一盏读书灯）把你的课程——每门课讲到第几周、本周材料、截止日期——整理成一份**存在你电脑上**的课程知识库，
再通过 MCP 交给你已经在用的 AI 应用（Claude Desktop、Claude Code、Codex，或 ChatGPT 桌面版的 Work/Codex 模式）。它只读：从不提交、发布或标记任何东西，
也从不为了解题去抓取作业要求；会提醒 AI 以辅导为主、标注出处，并遵守每门课的 AI 政策。

**安装**：从 [Releases](https://github.com/Euswbnix/pagelamp/releases/latest) 下载对应系统的安装包（macOS 的 `.dmg`：`aarch64` 对应 Apple 芯片，`x64` 对应 Intel；Windows 的 `-setup.exe`）。
macOS 请先把 PageLamp **拖进「应用程序」文件夹**再打开（AI 应用会指向这个位置，不要直接在磁盘映像或「下载」里运行）。
macOS 版已用 Developer ID 签名并经过 Apple 公证，可以直接打开（第一次打开时 macOS 会问是否打开从互联网下载的应用，点「打开」即可）；Windows 版的安装包、应用和其中的 `pagelamp.exe` 也已签名（Azure Artifact Signing）。新版本刚发布时 SmartScreen 仍可能提示「Windows 已保护你的电脑」（签名证书还在积累信誉）：先点「更多信息」，确认显示的发布者是维护者本人、与以往 PageLamp 版本相同（而不是「未知发布者」，核对方法见 [SECURITY.md](SECURITY.md#verifying-downloads)），再点「仍要运行」。
Linux 请用 `.deb`/`.rpm`（会同时安装 `/usr/bin/pagelamp`）；`.AppImage` 能运行应用，但 AI 应用用不了里面的 `pagelamp`。Linux 包没有代码签名，请用 `SHA256SUMS` 核对（[方法](SECURITY.md#verifying-downloads)）；保存 Canvas 令牌或日历订阅链接需要 GNOME Keyring 或 KWallet。

**上手**：
1. 把课件放进一个文件夹，每门课一个子文件夹，里面可以按「Week 1」「Week 2」分周（支持 PDF、.pptx、.docx、Markdown、文本等；旧版 .ppt/.doc 和扫描版 PDF 只列出、不能搜索）；
2. （可选）在 Canvas 的 **Calendar → Calendar Feed** 复制日历订阅链接，用来导入截止日期；
3. 在桌面应用的引导页添加文件夹和日历订阅并同步；
4. 打开「连接 AI 应用」，按提示把 PageLamp 加到 Claude Desktop 等应用里（改 Claude Desktop 配置前要先**完全退出** Claude Desktop），重新打开后问：「用 PageLamp 看看我这周各门课在讲什么？」这一步是让你自己的 AI 应用通过 MCP 读取 PageLamp；PageLamp 本身不会运行这些应用。

**课程 AI 政策**：很多大学规定，除非任课老师允许，课程里不能使用生成式 AI——请先看 syllabus。在课程的「AI 使用规定」页记录每门课的政策（命令行：`pagelamp course policy`）；标为「禁止使用 AI」的课，PageLamp 不会把课件正文交给 AI（标题、截止日期和学习计划仍可用于规划）。任何课程都可以在同一页关掉「允许 AI 应用读取这门课的资料」。

**遇到问题**：在「设置 → 帮助与反馈 → 复制诊断报告…」先查看再复制报告（课程名已替换为 Course 1、Course 2，令牌和日历链接已去除），贴到 [GitHub issue](https://github.com/Euswbnix/pagelamp/issues/new/choose)。不要贴令牌、日历订阅链接或课件。

**Canvas 访问令牌**：想直接从 Canvas 同步，需要在 Canvas 里生成一个个人访问令牌：
1. 在 Canvas 中打开 **Account → Settings**，往下找到 **Approved Integrations**，点 **+ New Access Token**；
2. 用途（Purpose）如实填写，例如“个人学习规划，只读”，并选好到期日期（Canvas 会显示允许的最长期限）；
3. 点 **Generate Token**，马上复制令牌——它只显示一次；
4. 在 PageLamp 里打开「数据来源与同步 → 添加数据来源 → Canvas 访问令牌」，填入你平时打开 Canvas 用的地址（例如 `https://canvas.school.edu`），粘贴令牌；命令行用 `pagelamp canvas add --base-url https://canvas.school.edu`。PageLamp 会先向 Canvas 验证令牌，再把它保存在系统钥匙串中。

如果看不到 **+ New Access Token**，说明你的学校关闭了学生访问令牌，请改用「课程文件夹 + 日历订阅」。令牌代表你本人访问 Canvas，不要交给别人；过期后生成一个新的，在这个来源上点「更换访问令牌」；想停止 PageLamp 的访问，在同一个 Canvas 页面删除这个令牌即可。


同步时 Canvas 会像记录其他访问一样记录下来：课程访问报告（老师能看到）里可能出现 PageLamp 读取的模块、页面、作业、文件和公告列表，以及它读取的每个页面（只读新增或有变化的页面）。读取页面不会完成模块要求；下载文件可能会，所以默认不下载。

（v0.3）PageLamp 应用运行期间也会自己同步，默认每天两次，可以改成每天一次或关闭。你不在应用前时，它先验证令牌，然后只向 Canvas 请求你的课程列表（含每门课的教学大纲）、截止日期和各门课的公告，不请求任何课程的模块、页面、文件列表或作业。上面说的完整同步在你发起同步时进行，或者在你打开 PageLamp、把它切到前台，而上次完整同步已经够久（或有新发现的课程还没读过）时进行。自动同步从不下载文件。

PageLamp 的 MCP 服务从不连接 Canvas 或互联网：它读取你电脑上的数据库，只在你的 AI 应用请求保存内容时写入（v0.1 中只有学习计划）；同步是单独的一步，用的是你自己的 Canvas 令牌：由你发起，或者（v0.3）由运行中的 PageLamp 应用按你的设置发起。MCP 服务不给你的 AI 应用任何发起同步的途径，并告诉它不要替你同步。

**隐私**：数据只存在你的电脑上；只有你向 AI 提问时，AI 读取的课程内容才会发到你自己的 AI 账号。v0.1 的 PageLamp 自己不调用任何 AI 模型；它从不转售或代付 AI 用量，也从不读取、保存或发送你登录 AI 账号所用的凭据（密码或令牌）。详见 [PRIVACY.md](PRIVACY.md)。

**路线图**：v0.1（2026 年 9 月发布）：AI 应用通过 MCP 读取你的课程，macOS 和 Windows 安装包已签名。v0.3（开发中）计划加入：自动更新；按真实上课日期推算周次；已结束的课程归入「往期课程」，也可以删除（可撤销）；每周提醒；以及由 PageLamp 用你选择的模型（你自己的 API key 或本机模型）直接生成学习计划，以及标注出处的每周讲解；有了模型，它还能读取教学大纲，提出日期建议由你确认。通过 MCP 连接你自己的 AI 应用会继续可用。这些功能使用你自己的 API key 或本机模型，不使用 ChatGPT 套餐；想用 ChatGPT 套餐，可以把 Codex 或 ChatGPT 桌面版作为你的 AI 应用，通过 MCP 连接 PageLamp。能否改用 Claude 套餐，我们正在询问 Anthropic，只有对方书面确认允许才会提供。发布前计划可能调整，已完成的内容见 [CHANGELOG](CHANGELOG.md)。

**声明**：PageLamp 是独立的开源项目，与 OpenAI、Anthropic、Instructure（Canvas）以及任何大学都没有关联，也没有得到它们的认可或赞助。文中提到 Claude、ChatGPT、Codex、Canvas，只是为了说明 PageLamp 能配合哪些应用和服务使用；Claude 是 Anthropic 的商标，ChatGPT 和 Codex 是 OpenAI 的商标，Canvas 是 Instructure, Inc. 的商标。

## License

[Apache-2.0](LICENSE). PageLamp is an independent open-source project. It is not affiliated with,
endorsed by, or sponsored by OpenAI, Anthropic, Instructure or any university. Claude is a
trademark of Anthropic; ChatGPT and Codex are trademarks of OpenAI; Canvas is a trademark of
Instructure, Inc. They are named here only to say which apps and services PageLamp works with.
