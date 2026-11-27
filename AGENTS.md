# Solilog: conventions for contributors (human or agent)

Solilog is a keyboard-first, offline, single-user issue tracker: a GPUI desktop
app and a CLI over one SQLite database. The UX goal is speed and
customizability: **everything is a command**, every command is bindable, and
every domain command is also scriptable from the CLI (primarily for AI agents).

## Layout

```
crates/solilog-core/   domain types, DB, every domain operation. NO gpui. Fast to build/test.
  assets/skill/SKILL.md  the agent skill, compiled in (`solilog skill`)
  src/skill.rs         installing it into agents' skills folders (no DB: in core to test without gpui)
  src/cli_link.rs      the macOS app's /usr/local/bin/solilog link (same reason)
  tests/upgrades.rs    every release's database (tests/fixtures/v*.db) opens and keeps its data
crates/solilog/        one binary: GUI (src/ui/) + CLI (src/cli.rs)
  assets/keymap.json   default key bindings
  assets/themes/       built-in JSON themes (`dark.json`, `light.json`, `8086.json`, `mmx.json`, `sse2.json`, `sse2-dark.json`)
  assets/icons/        default SVG icons, plus `pixel/` variants for pixel-icon themes
  assets/fonts/        bundled fonts (`W95F.otf` + `OFL.txt`)
  src/ui/actions.rs    every GUI action, grouped by namespace
  src/ui/keymap.rs     keymap loading (defaults + user file, hot reload)
  src/ui/theme.rs      theme loading (built-ins + `~/.config/solilog/themes/`, hot reload, bevel helpers)
  src/ui/picker.rs     reusable filter-list popup (command palette, status picker…)
  src/ui/window_frame.rs  window root: title bar (native macOS, classic theme, or Wayland/Linux client decorations) and resize edges
  src/ui/workspace.rs  main view; action handlers
  src/ui/workspace/    parts of the main view (sidebar, …)
packaging/icon.svg     app icon, for both systems
packaging/macos/       Solilog.icns (from icon.svg via icon.sh), Info.plist
packaging/linux/       .desktop file (named after the app id, `APP_ID` in ui/mod.rs)
Makefile               common tasks, install (macOS bundle; Linux desktop entry)
CHANGELOG.md           user-visible changes per release; see "Versions and releases"
scripts/               demo database, README screenshots, release.sh, upgrade-fixture.sh
docs/THEMES.md         user guide and schema reference for JSON themes
docs/screenshot*.png   README images, generated: see "Screenshots"
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
   Schema changes: **append** to `MIGRATIONS`, never edit existing entries
   (see "Migrations").
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
   If a binding is added, changed or removed, update the key tables in the
   README's "Keyboard" section to match (they list the main bindings, not all of them).
6. **Agent skill**: if the change affects how an agent would work with
   Solilog (a new workflow, a renamed command or flag, a changed rule), update
   `crates/solilog-core/assets/skill/SKILL.md`. Keep it to when and how to use
   Solilog; command details belong in `--help`.
7. Mouse interactions must go through the same actions (e.g. click → select →
   `window.dispatch_action(...)`), never through separate logic.
8. **Themes**: new UI must read colors, radii, shadows, and bevel/focus helpers
   from `theme::current(cx)` rather than hardcoding hex colors or fixed rounded
   corners (use `highlight_text`/`selection_text` on highlighted/selected rows
   so light and classic themes stay legible). If a feature adds a new theme
   token or chrome option, add it to `Theme` in `src/ui/theme.rs`, set it in
   every built-in theme (`assets/themes/dark.json`, `assets/themes/light.json`,
   `assets/themes/8086.json`, `assets/themes/mmx.json`,
   `assets/themes/sse2.json`, `assets/themes/sse2-dark.json`),
   and document it in `docs/THEMES.md`. If a new SVG icon is added to
   `assets/icons/`, add a 16×16 pixel variant in `assets/icons/pixel/` and wire
   it into `assets::icon_for`.
9. **Changelog**: add a line for user-visible changes under `## [Unreleased]`
   in `CHANGELOG.md` (Added / Changed / Fixed / Removed), written for users.

## Data model

- **Issue**: title, description, status, labels, projects, checklist, attachments, notes. The description is
  plain text, stored as written apart from whitespace cleanup
  (`normalize_description`), so it can be rendered as Markdown later. An issue can be archived: it
  keeps everything but is left out of listings, views and counts
  (`Db::list_archived_issues` lists them). Deleting an issue is permanent.
  Issues have a user-defined order (`Db::move_issue`, among issues with the
  same status); new issues, and issues whose status changes, go first. The
  GUI list groups issues by status (`Status::LIST_ORDER`), each in that order.
  A status's section can be collapsed to its header (`Db::set_status_collapsed`,
  saved in the `settings` table; display only).
- **Note**: an entry in an issue's worklog (`note.rs`): text (stored as
  written, like a description), author, time. Apart from the description
  (the spec) so agents log progress and handoffs without rewriting it.
  Never edited; deleted only by the user or by its author
  (`Db::delete_note`). Listed oldest first, and in `Db::issue_context`.
  Adding or deleting one is an activity (unreviewed when an agent does it).
  Deleting an issue deletes its notes; archiving keeps them.
- **Checklist**: an issue's ordered list of one-line items (`ChecklistItem`:
  text, done), each with a global id (`ChecklistItemId`, shown as a plain
  number). It only tracks progress (`Issue::checklist_progress`, shown as
  `done/total` in the list): unchecked items never prevent closing an issue.
  New items go last; `Db::move_checklist_item` reorders within one checklist.
- **Revision**: the text of an issue's title, description or checklist
  item before it was overwritten, or an item before it was deleted
  (`revision.rs`): full copies, saved by the core write methods, so the GUI
  and the CLI both get them. A field gets no new revision while its last is
  under 10 minutes old, unless the edit is destructive (loses half the text
  or 200 characters); each new one thins out the field's older ones (the
  `thinning.rs` helper backups use too). `Db::restore_revision` saves the
  current text first; a deleted item comes back with its id (checklist item
  ids are never reused) where it was. Deleting an issue deletes its
  revisions; archiving keeps them.
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
  its chips and sidebar dot use it. Sublabels without a color of their own
  are shown in their nearest colored ancestor's (`Label::display_color`).
  The sidebar can collapse a label's sublabels (`label::ToggleCollapsed`;
  `Db::set_label_collapsed`, saved in the database).
- **Project**: a body of work with a goal (`reverb v1.1`): status
  (`planned`, `active`, `done`, `archived`), description. Issues can be in any number of projects, or none; being outside a
  project is normal. A project's default labels are added to issues when they
  join it and kept when they leave. Done/archived projects are hidden from
  pickers and default listings; archived ones are also left out of the
  sidebar (they are in the archive). Projects have a user-defined order (new ones
  go last; `Db::move_project`, drag and drop in the sidebar); listings show
  open projects first, each section in that order.
- **Backup**: a zip of the database (a `VACUUM INTO` snapshot) and its
  attachments, with a `manifest.json` (`backup.rs`). Made automatically
  (`Db::auto_backup`: after CLI writes, and every few minutes in the GUI)
  when the last one is an hour old and the data changed; each new backup
  thins out the older ones (`to_prune`). Opening a database with an older
  schema backs it up first, whatever the last backup's age (reason
  `before_upgrade`, kept at least 90 days). The backup directory and the
  database id are stored in the database (`settings` table; a restore keeps
  the current ones). The id is in backup file names: a shared directory
  (a synced folder) can hold other databases' backups, which are listed and
  restorable but never thinned out or moved. Restoring backs up first.
- **Dependency**: an issue depends on others (work to finish first;
  `dependency.rs`, `Db::add_dependencies`): `Issue::depends_on`, with the
  reverse in `dependents` and the open ones in `waiting_on`. No issue
  depends on itself, and dependencies never form a cycle (refused). An
  open issue is *waiting* while one of its dependencies is open (archived
  ones too: archiving isn't closing), *ready* otherwise
  (`IssueFilter::ready` / `waiting`). Informational only: nothing changes
  a status by itself, and closing an issue with open dependencies is
  allowed. Deleting an issue drops its links both ways.
- **Mapped path**: a directory where a label's or project's code lives
  (`path_map.rs`, `Db::add_label_path` / `add_project_path`), so an agent
  in a repository finds its issues (`Db::path_owner`, `solilog where`,
  `--here`). A label or project can have several; a path has one owner.
  A directory belongs to the deepest mapped path containing it (so
  sublabels mapped inside their parent's directory win). Stored `~/…` when
  under the home directory, so a restore on another machine with the same
  layout works; paths missing on this machine are kept (`exists: false`)
  and match nothing. Renaming carries them along; deleting the owner
  deletes them.
- **Decision**: a standing statement on one issue, label or project
  (`decision.rs`, `DecisionOwner`): text, author, time. It applies below
  where it is recorded (`Db::inherited_decisions`: an issue gets its own,
  its projects' and its labels' with their ancestors', most specific
  first), in `Db::issue_context` and `solilog where`. Never edited:
  `Db::replace_decision` ends it (`superseded`, `replaced_by`) for a new
  one on the same owner, `Db::withdraw_decision` ends it; ended ones are
  kept and listed on request. Reviewed one by one (`reviewed`): one an
  agent records or withdraws waits for the user
  (`Db::mark_decisions_reviewed`; marking an issue reviewed confirms its
  own). On an issue, recording, replacing and withdrawing are activities.
  Deleting the owner deletes them; renaming carries them along.
- **View**: a named, saved `IssueFilter` (statuses, open only, labels (all
  must match), labels to exclude, projects (any may match) or no project).
  Renaming a label or project updates views; deleting one makes the views
  that use it fail until they are saved again.
- **Search**: `Db::search` matches words (as prefixes) and phrases in
  titles, descriptions, checklist items and notes through FTS5 tables
  (`search.rs`); an issue matches when every term is somewhere in it.
  Open issues only unless asked (`include_closed`: closed and archived).
  Changes since a time: `Db::activities` (`ActivityQuery`, across issues)
  and `IssueFilter::updated_since` (not for saved views).

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
- `solilog skill install` puts the agent skill (`SKILL.md`, compiled in) in
  `~/.claude/skills/solilog/`, `~/.agents/skills/solilog/` (Codex, Cursor,
  OpenCode, pi), and `~/.gemini/config/skills/solilog/` (Antigravity); its
  frontmatter marker (`managed-by: solilog`) is how it recognizes its own files,
  and it never overwrites others. Its `version` is the crate version, filled in
  when installed. On start, the app updates it where an older version installed
  it (`update_outdated_skills`), nowhere else.

## Versions and releases

- Semver, `MAJOR.MINOR.PATCH`. The version lives only in the workspace
  `Cargo.toml` (`[workspace.package]`); the crates, `solilog --version`, the
  skill's `version` and the macOS bundle all take it from there.
- 0.x until the database schema and the CLI JSON contract are considered
  stable. In 0.x, a breaking release bumps the minor version (0.2.0 → 0.3.0)
  and anything else the patch version (0.2.0 → 0.2.1).
- **Breaking** means a change to the CLI JSON contract (renamed or removed
  fields, commands, flags or enum keys; adding is never breaking), or a
  release that can't open a database an earlier version made (an upgrade path
  dropped). New migrations aren't breaking: they upgrade older databases in
  place. Going back to an older version after that isn't supported (an older
  binary refuses a newer schema); restore a backup instead.
- A release is a tagged commit (`v0.2.0`) whose `CHANGELOG.md` has a section
  for it. Cut one with `make cut-release V=0.2.0` (`scripts/release.sh`):
  it needs a clean tree and notes under `## [Unreleased]`, runs `make check`,
  turns those notes into the version's section, bumps `Cargo.toml` and
  `Cargo.lock`, adds the release's upgrade fixture, commits and tags. It
  doesn't push.
- Each release adds `crates/solilog-core/tests/fixtures/v<version>.db`, a
  database made by that release (`scripts/upgrade-fixture.sh`); the
  `upgrades` test opens every one with the current code. Never edit or
  remove a fixture. When a feature adds data worth keeping, add it to the
  script and check it in the test for the fixtures that have it.

## Migrations

Releases run migrations on real databases, so a migration must never lose
data. `Db::migrate` backs the database up first, then runs every pending
migration in one transaction (a failure applies none and leaves
`user_version` as it was) with foreign keys **off**, and checks them
(`PRAGMA foreign_key_check`) before committing.

- Foreign keys are off because changing a column means rebuilding the table
  (create `x_new` with the full definition, copy, drop `x`, rename): with
  them on, dropping `x` cascade-deletes the rows that point to it
  (`issue_labels`, `checklist_items`…). `PRAGMA foreign_keys` is a no-op
  inside a transaction, so never set it in a migration; `migrate_with`
  does it around the transaction.
- So, in a migration, deleting rows doesn't cascade: delete the dependent
  rows too, or the check fails.
- A rebuilt table must keep its `PRIMARY KEY` and constraints
  (`CREATE TABLE … AS SELECT` drops them; if other tables point to it,
  the check fails).
- Full-text search (`search.rs`) is kept in sync by triggers on `issues`,
  `checklist_items` and `notes` (migration 23). Rebuilding one of those
  tables drops its triggers: create them again in the same migration
  (`search_triggers_exist` checks they exist).
- The pre-upgrade backup reads the old schema (`Db::snapshot`): keep it
  working with every schema since migration 15.

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

- Format: a JSON array of `{ "context"?, "os"?, "bindings": { keystrokes: action } }`
  sections. Comments and trailing commas are allowed. A section with `"os"`
  (`macos`, `linux`, `windows`) only applies on that system.
- Use `secondary-` (cmd on macOS, ctrl elsewhere) for shortcuts shared by
  every system; `cmd-` (the Super key off macOS) only in `"os": "macos"`
  sections, where platform conventions differ (text navigation). The
  `default_keymap_is_valid` test enforces it. An action is
  `"ns::Name"`, `["ns::Name", { args }]`, or `null` to unbind.
- Defaults: `crates/solilog/assets/keymap.json` (compiled in). User overrides:
  `~/.config/solilog/keymap.json` (`$SOLILOG_CONFIG_DIR` / `$XDG_CONFIG_HOME`
  are respected). The file is reloaded automatically when it changes.
- Contexts: `Workspace` > `Sidebar` | `IssueList` | `Archive` | `IssueDetail` | `NewIssue > TextInput`;
  `IssueDetail > Checklist > ChecklistItem > TextInput` for checklist items,
  `IssueDetail > NoteDraft > TextInput` for the new-note field
  (attachment commands are registered on `IssueDetail`);
  `workspace::Search` (`workspace/search.rs`) is a picker over `Db::search`
  (`Picker::searching`: its items come from the query);
  `ProjectHeader > TextInput` for the description of the project the list
  shows (`workspace/project_header.rs`; outside `IssueList`, so its
  single-key bindings don't fire while typing);
  `ImagePreview` for an image attachment shown over the window (outside
  `IssueDetail`, so the issue's single-key bindings don't apply); `Picker` for popups. Multi-line text inputs also have `multiline`. Single-key
  bindings in a context that holds text fields need `&& !TextInput` (see `IssueDetail`).
- Issue commands act on the selected issues (`issue_list::ToggleSelected`,
  `SelectAll`, `ExtendSelectionDown/Up`; cmd/ctrl/shift-click, via `Modifiers::secondary`), or on the
  highlighted issue when none is selected. GUI handlers get them from
  `Workspace::target_issues` and apply changes in one go with
  `Db::update_issues` / `Db::delete_issues` (all or nothing).
- An open issue (`issue::Open`, Enter) replaces the issue list: title,
  description and checklist items are edited in place and saved as you type
  (`workspace/detail.rs`, `workspace/detail/checklist.rs`). Issue commands
  then act on it; checklist item commands act on the item being edited.
  Files dropped on it, or pasted (`ui/clipboard.rs` reads files copied in
  the file manager, and images: from the pasteboard on macOS, from the
  clipboard text elsewhere), become attachments. Clicking an image
  attachment previews it (`attachment::Preview`, `workspace/detail/preview.rs`;
  arrows go through the issue's images, `r` shows it in the file manager);
  other files are shown in the file manager. Name it with
  `SHOW_IN_FILE_MANAGER` in the UI ("Finder" on macOS).
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
make bundle                            # target/release/Solilog.app
make install                           # /Applications + CLI link in ~/.local/bin
make cut-release V=0.2.0               # version, commit and tag a release
```

Keep the build free of warnings and clippy lints.

## Screenshots

The README images (`docs/screenshot*.png`) are generated: `make screenshot`
(`scripts/screenshot.sh`) opens the app on a fresh demo database
(`scripts/demo-db.sh`) and captures its window once per image (the terminal
needs the Screen Recording permission): the issue list
(`screenshot.png`), an open issue (`screenshot-detail.png`, the demo's
showcase issue) and the list in the classic themes (`screenshot-8086.png`,
`screenshot-mmx.png`). After any visible UI change, regenerate them and
commit the ones that changed with it. When a feature is worth showing, add
it to the demo data (the open issue's features to the showcase issue); a new
image goes in the script and the README. Keep that data fictional and
neutral: no real people, companies or projects.
