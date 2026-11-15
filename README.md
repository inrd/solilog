# Solilog

A fast, minimal, offline issue tracker for one. Native desktop app built in Rust with [GPUI](https://gpui.rs), plus a CLI that works on the same data (handy for scripts and AI agents).

![Solilog](docs/screenshot.png)

## Install

```sh
make install      # macOS: builds Solilog.app, installs it in /Applications,
                  #   links the `solilog` command into ~/.local/bin
                  # Linux: installs `solilog` in ~/.local/bin, with a desktop
                  #   entry and an icon in ~/.local/share (app launcher)
make uninstall    # removes them; your database is kept
```

`BIN_DIR=…`, `APP_DIR=…` (macOS) and `DATA_DIR=…` (Linux) change where they go. `make` lists the other targets (tests, lints, bundle only, …).

On Linux, building needs a C compiler and the development packages of GPUI's dependencies (as for [Zed](https://zed.dev/docs/development/linux)): on Debian/Ubuntu, `sudo apt install build-essential pkg-config libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libfontconfig-dev`; a build error names any other missing package. Running it needs Vulkan (`libvulkan1` and a driver; Mesa provides one for most GPUs).

## Command line

`solilog` with no arguments opens the app; with a command, it works on the same data from the terminal. `solilog --help` lists the commands.

```sh
solilog issue create "Fix the thing"
solilog issue ls --open --json
solilog issue set-status SOL-1 done
pbpaste | solilog issue edit SOL-1 -d -    # description from stdin
solilog checklist add SOL-1 "Write tests" "Update docs"
solilog checklist check 1                  # item ids are shown by `issue show`
solilog attachment add SOL-1 crash.log     # copied next to the database
solilog issue create "Crash on load" -l acme/reverb -l bug
solilog issue ls -l acme                   # also matches sublabels like acme/reverb
solilog label ls
solilog project create "Reverb v1.1" -l acme/reverb
solilog issue create "Preset crash" -p "reverb v1.1"   # gets acme/reverb too
solilog issue ls -l acme --no-project                  # work outside any project
solilog view save "Acme bugs" --open -l acme -l bug    # saved filter
solilog issue ls --view "acme bugs"
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

## Using Solilog with AI agents

The `solilog` command is made for coding agents as much as for people: every command takes `--json`, and `--help` is written for an agent that has never seen Solilog. To let your agents know the tracker exists, install the Solilog skill, built into the binary:

```sh
solilog skill install     # for every agent found: Claude Code, Codex, Cursor, OpenCode, pi
solilog skill status      # where it is, and whether it matches this version
solilog skill show        # what it says
```

Then, from any project, mentioning an issue ("work on SOL-16") or your backlog ("what should I do next?") makes the agent reach for `solilog`: it reads the issue before working on it, ticks checklist items as it goes, and asks before closing an issue.

The skill goes in `~/.claude/skills/solilog/` (Claude Code) and `~/.agents/skills/solilog/` (Codex, Cursor, OpenCode, pi); `--agent claude` (repeatable) picks agents. `make install` installs it too. After upgrading Solilog, run `solilog skill install` again to update it; `solilog` reminds you in the terminal when it's out of date. It never overwrites a skill it didn't write. `solilog skill uninstall` removes it.

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
| `y` | Copy the issue id (`SOL-12`) |
| `alt-up` / `alt-down` | Reorder |

### An open issue

Title, description and checklist are edited in place and saved as you type. `Escape` goes back.

| Key | Action |
|---|---|
| `e` / `d` | Edit the title / the description |
| `a` | Add a checklist item (`Enter` goes to the next one) |
| `cmd-enter` | Check the item being edited |
| `alt-up` / `alt-down` | Move the item (or drag its handle) |
| `shift-a` | Attach a file |

An issue id in the description (`SOL-12`) links to that issue and shows its title. Click it to open the issue; while editing, `cmd`-click it or press `alt-enter` with the cursor on it.

To delete a checklist item, empty it. Files dropped on the issue, or pasted with `cmd-v` (screenshots too), become attachments. Click an image to preview it (`←` / `→` for the issue's other images, `r` to show it in the file manager); other files open in the file manager.

### Filters and views

Done and cancelled issues are hidden at first; the buttons at the top right of the list toggle "Open" and each status. Press `f`, then:

| Key | Filter |
|---|---|
| `l` / `x` | By label / excluding a label |
| `p` / `n` | By project / issues in no project |
| `s` | By status |
| `o` / `a` | Open issues only / all statuses |
| `c` / `r` | Clear / revert to the saved view |

`cmd-s` saves the filter as a view; `v` opens one. Saved views, projects and labels are in the sidebar.

### Your own bindings

Every command can be bound. Run "Workspace: Open Keymap" from the palette to edit `~/.config/solilog/keymap.json` (changes apply when you save). `solilog keymap defaults` prints the default bindings and `solilog keymap actions` lists every command.

## Development

```sh
make run                                  # the app, debug build
make demo                                 # the app on a fresh demo database
SOLILOG_DB=/tmp/scratch.db cargo run -- issue ls   # the CLI on a scratch database
```

`make` lists the other targets.
