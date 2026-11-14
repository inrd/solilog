# Solilog: conventions for contributors (human or agent)

Solilog is a keyboard-first, offline, single-user issue tracker: a GPUI desktop
app and a CLI over one SQLite database. The UX goal is speed and
customizability: **everything is a command**, every command is bindable, and
every domain command is also scriptable from the CLI (primarily for AI agents).

## Layout

```
crates/solilog-core/   domain types, DB, every domain operation. NO gpui. Fast to build/test.
crates/solilog/        one binary: GUI (src/ui/) + CLI (src/cli.rs)
  assets/keymap.json   default key bindings
  src/ui/actions.rs    every GUI action, grouped by namespace
  src/ui/keymap.rs     keymap loading (defaults + user file, hot reload)
  src/ui/picker.rs     reusable filter-list popup (command palette, status picker…)
  src/ui/workspace.rs  main view; action handlers
  src/ui/workspace/    parts of the main view (sidebar, …)
```

`solilog` with no args opens the GUI; `solilog <command>` runs the CLI.

## The two kinds of command

| | Domain operation | UI action |
|---|---|---|
| e.g. | create issue, set status, list | open palette, move selection |
| implemented in | `solilog-core` (`Db` methods) | GPUI action handlers |
| arguments | explicit (`IssueId`, `Status`) | implicit context (selection, focus) |
| CLI | **yes** | no |
| palette + bindable | yes, through a UI action wrapper | yes |

**Rule: business logic lives only in `solilog-core`.** GUI handlers and CLI
subcommands are thin adapters: they gather arguments (from the selection, a
picker, or argv), call the core, and present the result. If you find yourself
writing SQL or validation in `crates/solilog`, move it to core.

## Checklist: adding a feature

1. **Core**: add the operation as a `Db` method (plus types in `issue.rs` or a
   new module). Writes return the resulting object. Add unit tests (in-memory DB).
   Schema changes: **append** to `MIGRATIONS`, never edit existing entries.
2. **CLI**: add a subcommand in `cli.rs`. It must work with `--json` (serialize
   the core type) and print the result of writes. Doc comments become `--help`;
   write them for an agent who has never seen the code.
3. **GUI action**: declare it in `ui/actions.rs` under the right namespace,
   with a `///` doc comment (shown as its palette description and in
   `solilog keymap actions`). Use a struct with serde fields for parameters
   (see `issue::SetStatus`); make parameters optional where a picker can ask
   for them interactively.
4. **Handler**: register it with `.on_action(...)` on the element whose key
   context it belongs to (`Workspace`, `IssueList`, …). The palette lists exactly
   the actions available from the focused element, so placement decides when it shows.
5. **Default binding** (optional) in `assets/keymap.json`. The
   `default_keymap_is_valid` test checks that every name/argument/keystroke there is valid.
6. Mouse interactions must go through the same actions (e.g. click → select →
   `window.dispatch_action(...)`), never through separate logic.

## Data model

- **Issue**: title, status, labels, projects.
- **Label**: a free-form, hierarchical tag. The name is a `/`-separated path
  (`acme/reverb`). Every ancestor exists as a label of its own and is created
  on demand. An issue may carry any number of labels. Filtering by a label
  also matches its sublabels (`acme` matches `acme/reverb`), but issues only
  carry the labels they were given. Names are case-insensitive and keep the
  case they were created with. Renaming a label moves its sublabels with it.
- **Project**: a body of work with a goal (`reverb v1.1`): status
  (`planned`, `active`, `done`, `archived`), description, optional target
  date. Issues can be in any number of projects, or none; being outside a
  project is normal. A project's default labels are added to issues when they
  join it and kept when they leave. Done/archived projects are hidden from
  pickers and default listings.
- **View**: a named, saved `IssueFilter` (statuses, open only, labels (all
  must match), labels to exclude, projects (any may match) or no project).
  Renaming a label or project updates views; deleting one makes the views
  that use it fail until they are saved again.

## Naming

One concept, one name, consistent across surfaces:

| core | CLI | action | palette |
|---|---|---|---|
| `Db::set_status` | `solilog issue set-status <id> <status>` | `issue::SetStatus` | "Issue: Set Status" |

- Action namespaces are snake_case nouns (`issue`, `issue_list`, `workspace`);
  action names are PascalCase verbs. Palette labels are derived automatically.
- Namespaces in `PALETTE_HIDDEN_NAMESPACES` (navigation, text editing, picker
  internals) are bindable but don't appear in the palette.
- Stable keys (`in_progress`) are used in the DB, JSON and the CLI; labels
  (`In Progress`) are display-only. Issue ids display as `SOL-12`; parsing
  accepts `SOL-12` or `12`.

## CLI contract (agents depend on this)

- `--json` on every command; JSON field names and enum keys are a stable API.
  Add fields freely; don't rename or remove them.
- Default output is a compact, human-readable form.
- Exit 0 on success. On failure exit non-zero with the message on stderr
  (`{"error": "..."}` with `--json`). Usage errors come from clap (exit 2).
- Long text can come from stdin with `-`.
- Respect `SOLILOG_DB` (database path); tests and agents use it for isolation.

## Data & state

- **The database is the source of truth.** In-memory GUI state is a cache: after
  any write, call `reload()` instead of patching the cache by hand. The GUI
  polls `PRAGMA data_version` and reloads when another process (e.g. the CLI)
  writes.
- WAL mode plus a busy timeout make concurrent GUI and CLI use safe. Keep
  transactions short.
- Errors in the GUI go through `Workspace::report` (shown in the error bar);
  never `unwrap` on IO or DB results.

## Keymap

- Format: a JSON array of `{ "context"?, "bindings": { keystrokes: action } }`
  sections. Comments and trailing commas are allowed. An action is
  `"ns::Name"`, `["ns::Name", { args }]`, or `null` to unbind.
- Defaults: `crates/solilog/assets/keymap.json` (compiled in). User overrides:
  `~/.config/solilog/keymap.json` (`$SOLILOG_CONFIG_DIR` / `$XDG_CONFIG_HOME`
  are respected). The file is reloaded automatically when it changes.
- Contexts: `Workspace` > `Sidebar` | `IssueList` | `NewIssue > TextInput`; `Picker` for popups.

## Commands

```sh
cargo test --workspace                 # all tests
cargo test -p solilog-core             # fast: no gpui
cargo clippy --workspace --all-targets
cargo run                              # GUI
cargo run -- issue ls --json           # CLI
SOLILOG_DB=/tmp/scratch.db cargo run -- issue create "test"   # isolated DB
```

Keep the build free of warnings and clippy lints.
