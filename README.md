# Solilog

A local planning and handoff hub for working with coding agents: a fast, lightweight native desktop app where you spec, prioritize, and review work, paired with a CLI over the same local database that agents read from and report back to.

Solilog lives outside any single tool, repository, or session. Different agents across your projects share one durable source of truth: issues hold the spec, an append-only worklog of what each agent tried or decided, and structured review handoffs with test results and commits. Offline, keyboard-first, single-user.

![Solilog](docs/screenshot.png)

## Agent workflow

Every `solilog` command takes `--json`, and `--help` is written for an agent that has never seen Solilog. Install the built-in skill so agents know to use it:

```sh
solilog skill install     # for every agent found: Antigravity, Claude Code, Codex, Cursor, OpenCode, pi
solilog skill status      # where it is, and whether it matches this version
solilog skill show        # what it says
```

From any project, mentioning an issue ("work on SOL-16") or your backlog ("what should I do next?") makes the agent reach for `solilog`:

- **Read in one call:** agents scan ready work with `issue ls --ready --compact --json` and pull an issue together with its projects, dependencies, referenced `SOL-n` issues, and notes via `issue context`.
- **Worklog, not spec:** as an agent works, it ticks checklist items and logs progress, decisions, and handoffs in append-only **notes**, leaving your description alone. If text is ever overwritten by mistake, revision history (`shift-h` / `issue history`) lets you restore it.
- **Authorship and review tracking:** agents identify themselves with `--actor` or `SOLILOG_ACTOR` (e.g. `claude:fix-auth`). Changes made outside the app are attributed, marked subtly in the UI, recorded in an activity log, and collected in the sidebar's **Unreviewed** view until you clear them (`r` / `shift-r`).
- **Review handoff with receipts:** when finished, the agent doesn't close the issue — it hands it to you (`In Review`, at the top of the list) with a receipt of the checks it ran (pass or fail) and the commits and files it changed. You approve it in the app (`cmd-enter`, marking it done) or send it back to Todo with a note (`cmd-backspace`).

![Issue detail with review receipt and unreviewed changes](docs/screenshot-detail.png)

The skill goes in `~/.claude/skills/solilog/` (Claude Code), `~/.agents/skills/solilog/` (Codex, Cursor, OpenCode, pi), and `~/.gemini/config/skills/solilog/` (Antigravity); `--agent claude` (repeatable) picks agents. `make install` and **Workspace: Install Command Line Tool** install it too. Opening a newer version of the app updates any skill Solilog previously installed (or run `solilog skill install`; `solilog` reminds you in the terminal when it's out of date). It never overwrites a skill it didn't write. `solilog skill uninstall` removes it.

## Install

```sh
make install      # macOS: builds Solilog.app, installs it in /Applications,
                  #   links the `solilog` command into ~/.local/bin
                  # Linux: installs `solilog` in ~/.local/bin, with a desktop
                  #   entry and an icon in ~/.local/share (app launcher)
make uninstall    # removes them; your database is kept
```

`BIN_DIR=…`, `APP_DIR=…` (macOS) and `DATA_DIR=…` (Linux) change where they go. Building from source requires Rust 1.89 or newer (`rustup update stable`); pass `CARGO="cargo +<toolchain>"` or `RUSTUP_TOOLCHAIN=<toolchain>` to use a specific toolchain. `make` lists the other targets (tests, lints, bundle only, …).

With only `Solilog.app` (not built with `make install`), move it to Applications, open it, and run **Workspace: Install Command Line Tool** from the command palette: it links the `solilog` command into `/usr/local/bin` (asking for an administrator password if needed) and installs the agent skill. Run it again after moving the app.

On Linux, building needs a C compiler and the development packages of GPUI's dependencies (as for [Zed](https://zed.dev/docs/development/linux)): on Debian/Ubuntu, `sudo apt install build-essential pkg-config libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libfontconfig-dev`; a build error names any other missing package. Running it needs Vulkan (`libvulkan1` and a driver; Mesa provides one for most GPUs).

## Command line

`solilog` with no arguments opens the app; with a command, it works on the same data from the terminal. `solilog --help` lists the commands.

```sh
solilog issue create "Fix the thing"
solilog issue ls --open --json
solilog issue ls --ready --compact --json    # open issues with nothing open to wait for
solilog issue context SOL-1 --json           # issue + projects, dependencies, refs, notes
solilog issue set-status SOL-1 done
pbpaste | solilog issue edit SOL-1 -d -      # description from stdin
solilog checklist add SOL-1 "Write tests" "Update docs"
solilog checklist check 1                    # item ids are shown by `issue show`
solilog attachment add SOL-1 crash.log       # copied next to the database
solilog note add SOL-1 "Tried X, fails on Y" # the issue's worklog
solilog issue request-review SOL-1 -m "Fixed X" --pass "cargo test" --commit a1b2c3d
solilog issue create "Crash on load" -l acme/reverb -l bug
solilog issue ls -l acme                     # also matches sublabels like acme/reverb
solilog issue add-dependency SOL-3 SOL-1     # SOL-3 depends on SOL-1
solilog label ls
solilog project create "Reverb v1.1" -l acme/reverb
solilog issue create "Preset crash" -p "reverb v1.1"   # gets acme/reverb too
solilog issue ls -l acme --no-project                  # work outside any project
solilog view save "Acme bugs" --open -l acme -l bug    # saved filter
solilog issue ls --view "acme bugs"
solilog label add-path acme/reverb ~/code/reverb       # where its code lives
solilog where                                          # label or project of this folder
solilog issue ls --here --open                         # its issues
```

The database is `~/Library/Application Support/Solilog/solilog.db` on macOS, `~/.local/share/Solilog/solilog.db` on Linux (`solilog db-path` prints it). Set `SOLILOG_DB` to use another file.

## Backups

The database and its attachments are backed up automatically after changes, at most once an hour, as zip archives you can open without Solilog. Older ones are thinned out: everything from the last day, then one per day for two weeks, one per week for three months, one per month for a year. They go next to the database (`solilog-backups/`) unless you choose another folder, such as a synced one: "Backup: Choose Directory" in the palette, or `solilog backup set-dir ~/Dropbox/solilog`. The existing backups move with it.

```sh
solilog backup ls                    # newest first
solilog backup create                # now
solilog backup restore solilog-2026-11-14-153012-a1b2c3d4.zip
```

Restoring (also "Backup: Restore" in the palette) backs up the current data first, so it can be undone.

## Keyboard

Everything is a command: press `cmd-k` to open the command palette and search for it. Most commands also have a key. Below, `cmd` means `ctrl` on Linux.

### Getting around

| Key | Action |
|---|---|
| `cmd-k` | Command palette |
| `cmd-n` / `c` | New issue |
| `j` / `k` (or arrows) | Move down / up |
| `h` / `l` | Focus the sidebar / the issue list |
| `cmd-b` | Show or hide the sidebar |
| `m` (or right-click) | Context menu of the highlighted row |

### Issues

Commands act on the selected issues, or on the highlighted one when none is selected.

| Key | Action |
|---|---|
| `Enter` (or double-click) | Open the issue |
| `x` | Select or deselect it (also `cmd`/`shift`-click) |
| `shift-j` / `shift-k` | Extend the selection down / up |
| `cmd-a` | Select all |
| `s` | Set status |
| `l` / `shift-l` | Add / remove a label |
| `p` / `shift-p` | Add to / remove from a project |
| `b` / `shift-b` | Add / remove a dependency (an issue to finish first) |
| `y` | Copy the issue id (`SOL-12`) |
| `r` / `shift-r` | Mark agent or CLI changes as reviewed / mark all reviewed (in the Unreviewed view) |
| `cmd-enter` / `cmd-backspace` | Approve an issue in review (it becomes done) / send it back to Todo with a note |
| `shift-h` | Earlier text of the issue's title, description and checklist, to restore it |
| `alt-up` / `alt-down` | Reorder |
| `z` / `shift-z` | Collapse or expand a status section (or click its header) / expand all sections |

### An open issue

Title, description and checklist are edited in place and saved as you type. `Escape` goes back.

Below the checklist, notes are the issue's worklog: what you or an agent tried, decided or left for next time, each with its author and time. Notes are never edited; a note being written is added when you leave the issue.

An issue an agent handed off for review shows its latest receipt above the description: a summary, the checks run (✓ pass, ✗ fail), and the commits and files changed, with buttons to approve it or send it back (`cmd-enter` / `cmd-backspace`). Each handoff and your answer stay in the notes.

| Key | Action |
|---|---|
| `e` / `d` | Edit the title / the description |
| `a` | Add a checklist item (`Enter` goes to the next one) |
| `cmd-enter` | Check the item being edited |
| `alt-up` / `alt-down` | Move the item (or drag its handle) |
| `n` | Write a note (`cmd-enter` adds it) |
| `shift-a` | Attach a file |
| `cmd-z` / `cmd-shift-z` | Undo / redo while editing text (also `ctrl-y` on Linux) |

An issue id in the description (`SOL-12`) links to that issue and shows its title; a web address (`https://…`) links to that page. Click a link to open it (the issue, or the page in your browser); while editing, `cmd`-click it or press `alt-enter` with the cursor on it.

The issues it depends on, and those that depend on it, are listed under "Dependencies": click one to open it. An issue is *waiting* (a clock in the list, with the number of open dependencies) until every issue it depends on is done or cancelled; nothing changes its status by itself.

To delete a checklist item, empty it or press `cmd-shift-backspace`. Files dropped on the issue, or pasted with `cmd-v` (screenshots too), become attachments. Click an image to preview it (`←` / `→` for the issue's other images, `r` to show it in the file manager); other files open in the file manager.

### Filters and views

Done and cancelled issues are hidden at first; the buttons at the top right of the list toggle "Open" and each status. Press `f`, then:

| Key | Filter |
|---|---|
| `l` / `x` | By label / excluding a label |
| `p` / `n` | By project / issues in no project |
| `t` / `w` | Ready to start / waiting on an open dependency |
| `u` | Unreviewed agent or CLI changes (`g u` opens the Unreviewed view) |
| `s` | By status |
| `o` / `a` | Open issues only / all statuses |
| `c` / `r` | Clear / revert to the saved view |

`cmd-s` saves the filter as a view; `v` opens one. Saved views, projects and labels are in the sidebar; there, `space` (or the arrow before a label) collapses or expands a label's sublabels.

A project's issues come under a header with its status, progress, default labels, paths (folders where its code lives; click one to show it) and description; a label's, under one with its issue count and paths. Click the description (or run "Project: Edit Description") to edit it; it's saved as you type, and `Escape` goes back to the list.

### Your own bindings

Every command can be bound. Run "Workspace: Open Keymap" from the palette to edit `~/.config/solilog/keymap.json` (changes apply when you save). `solilog keymap defaults` prints the default bindings and `solilog keymap actions` lists every command.

### Themes

Run "Workspace: Set Theme" from the palette to switch between built-in themes (`Dark`, `Light`, `8086`, `MMX`, `SSE2`, `SSE2 Dark`) or your own JSON themes in `~/.config/solilog/themes/` (changes apply when you save). See [docs/THEMES.md](docs/THEMES.md) for the format and token reference.

| 8086 | MMX |
|---|---|
| ![8086 theme](docs/screenshot-8086.png) | ![MMX theme](docs/screenshot-mmx.png) |

## Development

```sh
make run                                  # the app, debug build
make demo                                 # the app on a fresh demo database
SOLILOG_DB=/tmp/scratch.db cargo run -- issue ls   # the CLI on a scratch database
```

`make` lists the other targets.
