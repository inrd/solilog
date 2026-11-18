# Changelog

Notable changes to Solilog, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
the scheme in AGENTS.md ("Versions and releases").

## [Unreleased]

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
