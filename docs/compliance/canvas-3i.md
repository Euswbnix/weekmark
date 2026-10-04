# Canvas API Policy §3(i) and how PageLamp syncs

**Status: our reading, not confirmed with Instructure.** Recorded on 2026-10-03, as owner decision
D30 asks before PageLamp is published to any directory. This note says what PageLamp does and why;
it is not legal advice and makes no statement on Instructure's behalf.

## The clause, as we read it

The policy's own text is what counts: <https://www.instructure.com/policies/api-policy> (we read
the version effective 2025-08-12).

As we read it, §3(i) restricts access to Canvas APIs through MCP servers that Instructure hasn't
approved. Our understanding is that this rules out Canvas access from PageLamp's MCP server, and
anything that would let an MCP client reach Canvas through it. So all Canvas access happens in
sync, and no MCP tool call may cause one (docs/ARCHITECTURE.md §3 rule 1).

## What PageLamp does

- `pagelamp mcp` never opens a network connection. It reads the database on the student's
  computer, and writes to it only what the student's AI app asks it to save (a study plan).
- Canvas is read only by sync, in `pagelamp-canvas`: GET requests only, with the student's own
  access token, for the student's own use.
- A sync starts in two ways only:
  - the student starts it, in the PageLamp app or with `pagelamp sync`;
  - the running PageLamp app starts it, under the student's setting (automatic sync: twice a
    day, once a day or off): from its own timer, or when the student opens the app, comes back
    to it or changes that setting.
- No MCP tool call can cause, request or schedule a sync:
  - no tool starts a sync, or starts or raises the PageLamp app;
  - no `pagelamp://` link and no local port triggers one;
  - nothing an MCP client can write is read by the timer's rule;
  - the server's texts tell an AI app that can run commands or open apps not to sync for the
    student.
- PageLamp doesn't schedule its command-line tool (no launchd or Task Scheduler entry), and
  nothing syncs while the app is closed.
- A run the timer starts with nobody at the app makes no `/courses/:id/…` request: it checks
  the token and reads the course list, planner items and each course's announcements. A full
  sync runs when the student starts one, or opens the app or brings it to the front.
- No automatic run downloads files.

## Checked by tests

- `crates/pagelamp-mcp/tests/e2e.rs`, `no_tool_call_touches_what_the_automatic_sync_reads`: after
  every MCP tool was called, the sync setting, the attempts record, the light-sync record and the
  sources' sync times are unchanged. A new tool has to be added to it.
- The same file, `sync_status_reports_automatic_sync_and_what_each_source_needs`: the server's
  instructions, the `sync_status` description and every hint and note about old or unread data
  name no sync command.
- `crates/pagelamp-canvas/src/tests_sync.rs`, `every_request_is_an_allow_listed_get` and
  `a_user_level_sync_asks_for_no_course_and_removes_only_what_it_read` (the paths a run with nobody at the
  app asks for).
- `crates/pagelamp-app/tests/crate_graph.rs`: the MCP crate depends on neither the Canvas crate
  nor the facade that runs syncs.

## Not verified

- Whether Instructure reads §3(i) as we do.
- Whether course data a sync stored earlier, read later through a local MCP server, is outside
  the clause. We believe it is, since the MCP server never reaches Canvas; nobody at Instructure
  has confirmed it.
- The policy can change. Read it again, and update this note, before a release that changes how
  or when PageLamp syncs and before publishing PageLamp to a directory.
