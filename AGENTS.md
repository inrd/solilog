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

- **Issue**: title, description, status, labels, projects, checklist, attachments. The description is
  plain text, stored as written apart from whitespace cleanup
  (`normalize_description`), so it can be rendered as Markdown later. An issue can be archived: it
  keeps everything but is left out of listings, views and counts
  (`Db::list_archived_issues` lists them). Deleting an issue is permanent.
  Issues have a user-defined order (`Db::move_issue`, among issues with the
  same status); new issues, and issues whose status changes, go first. The
  GUI list groups issues by status (`Status::LIST_ORDER`), each in that order.
- **Checklist**: an issue's ordered list of one-line items (`ChecklistItem`:
  text, done), each with a global id (`ChecklistItemId`, shown as a plain
  number). It only tracks progress (`Issue::checklist_progress`, shown as
  `done/total` in the list): unchecked items never prevent closing an issue.
  New items go last; `Db::move_checklist_item` reorders within one checklist.
- **Attachment**: a file copied into the attachments directory next to the
  database (`solilog.db` → `solilog-attachments/SOL-12/<name>`; see
  `attachments_dir_for`), so the database and that directory move together.
  The database lists them (`Attachment`: id, name, size, `path` computed from
  the database location); stray files in the directory are ignored. Names
  are unique per issue, ignoring case (taken ones get ` (2)`); existing
  files are never overwritten. Deleting an issue deletes its directory;
  archiving keeps it. In-memory databases can't have attachments: tests use
  a temporary database file.
- **Label**: a free-form, hierarchical tag. The name is a `/`-separated path
  (`acme/reverb`). Every ancestor exists as a label of its own and is created
  on demand. An issue may carry any number of labels. Filtering by a label
  also matches its sublabels (`acme` matches `acme/reverb`), but issues only
  carry the labels they were given. Names are case-insensitive and keep the
  case they were created with. Renaming a label moves its sublabels with it.
  Labels are listed as a tree with siblings in a user-defined order (new ones
  go last; `Db::move_label` reorders among siblings only, sublabels follow).
  A label can have a color from a fixed palette (`LabelColor`, `Db::set_label_color`);
  its chips and sidebar dot use it. Sublabels do not inherit it.
- **Project**: a body of work with a goal (`reverb v1.1`): status
  (`planned`, `active`, `done`, `archived`), description. Issues can be in any number of projects, or none; being outside a
  project is normal. A project's default labels are added to issues when they
  join it and kept when they leave. Done/archived projects are hidden from
  pickers and default listings; archived ones are also left out of the
  sidebar (they are in the archive). Projects have a user-defined order (new ones
  go last; `Db::move_project`, drag and drop in the sidebar); listings show
  open projects first, each section in that order.
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
- Errors in the GUI go through `Workspace::report` (shown in the footer);
  never `unwrap` on IO or DB results.

## Keymap

- Format: a JSON array of `{ "context"?, "bindings": { keystrokes: action } }`
  sections. Comments and trailing commas are allowed. An action is
  `"ns::Name"`, `["ns::Name", { args }]`, or `null` to unbind.
- Defaults: `crates/solilog/assets/keymap.json` (compiled in). User overrides:
  `~/.config/solilog/keymap.json` (`$SOLILOG_CONFIG_DIR` / `$XDG_CONFIG_HOME`
  are respected). The file is reloaded automatically when it changes.
- Contexts: `Workspace` > `Sidebar` | `IssueList` | `Archive` | `IssueDetail` | `NewIssue > TextInput`;
  `IssueDetail > Checklist > ChecklistItem > TextInput` for checklist items
  (attachment commands are registered on `IssueDetail`);
  `Picker` for popups. Multi-line text inputs also have `multiline`. Single-key
  bindings in a context that holds text fields need `&& !TextInput` (see `IssueDetail`).
- Issue commands act on the selected issues (`issue_list::ToggleSelected`,
  `SelectAll`, `ExtendSelectionDown/Up`; cmd/shift-click), or on the
  highlighted issue when none is selected. GUI handlers get them from
  `Workspace::target_issues` and apply changes in one go with
  `Db::update_issues` / `Db::delete_issues` (all or nothing).
- An open issue (`issue::Open`, Enter) replaces the issue list: title,
  description and checklist items are edited in place and saved as you type
  (`workspace/detail.rs`, `workspace/detail/checklist.rs`). Issue commands
  then act on it; checklist item commands act on the item being edited.
  Files dropped on it, or pasted (`ui/clipboard.rs` reads copied Finder
  files and images on macOS), become attachments.
- The archive (`workspace::OpenArchive`, palette only) replaces the issue list
  and lists archived projects and issues, with a context menu to delete them
  or bring them back.

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
