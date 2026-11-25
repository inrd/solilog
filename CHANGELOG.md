# Changelog

Notable changes to Solilog, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
the scheme in AGENTS.md ("Versions and releases").

## [Unreleased]

### Added

- Themes: switch between the default Dark theme and the retro beveled MMX theme
  with "Workspace: Set Theme", or add custom JSON themes in
  `~/.config/solilog/themes/` (hot-reloaded on save; see `docs/THEMES.md`).
- Hovering a cropped issue title, checklist item, sidebar entry, project name,
  or picker item now shows the full text in a tooltip.

### Fixed

- Long wrapped descriptions (and note drafts) in the issue detail no longer
  shrink below their text and leak into the sections below when the window is
  wider than the issue body.
- Issue ids from SOL-100 on no longer wrap onto two lines in the issue list,
  the archive and an issue's dependencies: the id column widens to fit the
  longest id.
- The out-of-date agent skill note on the CLI now appears at most once a day
  (and can be silenced with `SOLILOG_NO_SKILL_HINT=1`, used by the release and
  demo scripts).

## [0.2.3] - 2026-11-23

### Added

- Review handoff: an agent that has finished an issue hands it to you with
  `solilog issue request-review`, which sets the new In Review status and
  adds a receipt to its notes: a summary, the checks it ran (pass or fail),
  and the commits and files it changed. Open the issue to see the receipt,
  then approve it (`cmd-enter`; it becomes done) or send it back to Todo with
  a note (`cmd-backspace`); also from the list, the context menu, or
  `solilog issue approve` / `send-back`. Issues in review come first in the
  list and are left out of `--ready`. The agent skill now has agents hand
  work off this way instead of asking in chat before closing.

- Notes on issues: a worklog of timestamped entries, each showing who wrote
  it, apart from the description and the checklist. Write and read them below
  the checklist of an open issue (`n`, then `cmd-enter` to add), or with
  `solilog note add`, `note ls` and `note rm`. `solilog issue context` includes
  them, and the agent skill tells agents to log progress and handoffs there
  instead of rewriting descriptions. Notes can't be edited; a note can be
  deleted by you or by the agent that wrote it.

- `solilog issue ls --compact --json` returns lightweight issue summaries
  (`id`, `title`, `status`, `labels`, `projects`, `checklist_progress`,
  `waiting_on`, `unreviewed`, `updated_at`), omitting descriptions, full
  checklists, attachments and authorship details so large listings stay small.
- Mark all unreviewed issues as reviewed at once in the Unreviewed view
  ("Issue: Mark All Reviewed", `shift-r`, header button, or
  `solilog issue review --all`).

### Changed

- The "Unreviewed" view now stays pinned in the sidebar even when empty,
  showing "Nothing to review." when there are no unreviewed changes.

### Fixed

- Opening a database whose directory is blocked by a sandbox (such as macOS
  Seatbelt) now reports a permission/sandbox access error instead of a
  misleading `File exists (os error 17)` error.
- Checklist items and the new-item field no longer disappear or stop responding
  to clicks when the window is wider than the issue body.
- The back button, issue id, author, and archived badge in the issue detail
  header now have spacing between them.

## [0.2.2] - 2026-11-21

### Added

- `solilog issue context <id>` (with `--json`) bundles an issue together with
  its projects, `depends_on` issues, and any `SOL-n` issues referenced in its
  description in one call.
- Authorship, change log and review tracking: every write records who made
  it (`user` in the app, `cli` by default on the CLI, or `--actor` /
  `SOLILOG_ACTOR`, e.g. `claude:fix-auth`) on revisions, issue text fields,
  checklist items and a per-issue activity log (`solilog issue activity`).
  Non-user writes mark an issue unreviewed until cleared ("Issue: Mark
  Reviewed", `r`, `solilog issue review`). Filter for unreviewed changes
  (`f u`, `g u`, sidebar "Unreviewed", `issue ls --unreviewed`); open issues
  show an unreviewed changes banner and subtle authorship marks on
  agent-written text.
- `solilog skill install` now supports Antigravity (`--agent antigravity`,
  detected via `~/.gemini/antigravity` and installed to
  `~/.gemini/config/skills/solilog/`).

## [0.2.1] - 2026-11-18

### Added

- Dependencies between issues: an issue can depend on others that must be
  finished first ("Issue: Add Dependency", `b`; `shift-b` removes one;
  `solilog issue add-dependency SOL-12 SOL-7`). An open issue is waiting
  while one of them is open (a clock in the list), and ready once none is.
  The open issue lists what it depends on and what depends on it. Filter
  by "ready" (`f t`, `issue ls --ready`) or "waiting" (`f w`,
  `--waiting`). Nothing changes a status by itself, and a cycle of
  dependencies is refused.

### Fixed

- The end of a long description no longer runs into the sections below it
  (attachments, checklist) when the window is wider than the issue's text.

## [0.2.0] - 2026-11-17

### Added

- Before a new version upgrades the database, it backs it up ("before
  upgrade to …" in the backup list), and that backup is kept at least 90
  days.
- macOS: "Workspace: Install Command Line Tool" in the command palette puts
  the `solilog` command on the PATH (`/usr/local/bin`) and installs the agent
  skill, for an app that wasn't installed with `make install`.
- When a new version of the app starts, it updates the agent skill wherever
  an older version installed it (and says so in the footer). It doesn't
  install the skill where it isn't, or touch a skill Solilog didn't write.
- Undo and redo in text fields: `cmd-z` / `cmd-shift-z` (`ctrl-z` /
  `ctrl-shift-z` or `ctrl-y` on Linux). Typing is undone a word at a time;
  a paste, a cut or deleting a selection is a step of its own. Switching to
  another issue, or a change made elsewhere (e.g. from the command line),
  starts a new history.
- "Project: Create" in the command palette, and a + next to the sidebar's
  Projects heading, create a project and show it.
- A header above the issue list shows the project it lists: its name, its
  status (click to change it) and how many of its issues are closed.
- A project's default labels can be added and removed in the app: in the
  project header, or with "Project: Add Label" and "Project: Remove Label".
- An issue keeps the earlier text of its title, description and checklist
  items (deleted items too), whether changed in the app or from the command
  line, so text overwritten by mistake can be brought back: `shift-h` (or
  "Issue: Show History") in the app, `solilog issue history` and
  `solilog issue restore` on the command line. Edits less than 10 minutes
  apart share one revision, except edits that remove much of the text;
  older revisions are thinned out over time.
- A project's description is in its header, edited in place and saved as
  you type, like an issue's ("Project: Edit Description"). A long one shows
  its first lines until you edit it.

### Changed

- Project descriptions are cleaned up like issue descriptions when saved
  (trailing spaces and blank lines at both ends are dropped, indentation is
  kept).

### Fixed

- Linux (Wayland): the window has a title, so GNOME's window list
  shows "Solilog" instead of an empty name.

## [0.1.0] - 2026-11-15

### Added

- Desktop app (macOS, Linux) and `solilog` command line over one local SQLite
  database, usable at the same time.
- Issues with a status, a description, a checklist and file attachments;
  archiving, multiple selection and a user-defined order within each status.
- Hierarchical labels with colors, projects with a status and default labels,
  and saved views.
- Everything is a command: command palette, rebindable keys in
  `~/.config/solilog/keymap.json`, reloaded as it changes.
- `--json` output on every CLI command, for scripts and AI agents, and a
  built-in agent skill (`solilog skill install`).
- Automatic backups of the database and attachments, with restore.
