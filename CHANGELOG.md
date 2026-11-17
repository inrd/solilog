# Changelog

Notable changes to Solilog, newest first. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
the scheme in AGENTS.md ("Versions and releases").

## [Unreleased]

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
