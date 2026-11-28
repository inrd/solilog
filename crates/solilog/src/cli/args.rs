//! The command line: clap definitions. Doc comments are the `--help` text.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use solilog_core::{
    Actor, Agent, AttachmentId, ChecklistItemId, Db, DecisionId, DecisionOwner, IssueFilter,
    IssueId, LabelColor, LinkId, LinkKind, NoteId, ProjectStatus, RevisionId, Status,
};

use super::run::current_dir;

#[derive(Parser)]
#[command(
    name = "solilog",
    version,
    about = "A fast, minimal, offline issue tracker for one.",
    long_about = "A fast, minimal, offline issue tracker for one.\n\n\
        Run without a command to open the desktop app. \
        The CLI and the app share the same database and can be used at the same time.",
    after_help = "Database: $SOLILOG_DB, or the platform data dir (see `solilog db-path`)."
)]
pub struct Cli {
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    pub json: bool,

    /// Who is making this change (e.g. `claude:fix-auth` or `user`).
    /// Falls back to `$SOLILOG_ACTOR`, then `cli`.
    #[arg(long, global = true, value_name = "ACTOR")]
    pub actor: Option<Actor>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Manage issues.
    #[command(subcommand, visible_alias = "i")]
    Issue(IssueCommand),
    /// Manage labels.
    ///
    /// Labels are free-form, hierarchical tags: `acme/reverb` is a sublabel of
    /// `acme`. Ancestors are created automatically. Filtering by a label also
    /// matches its sublabels.
    #[command(subcommand, visible_alias = "l")]
    Label(LabelCommand),
    /// Manage projects.
    ///
    /// A project is a body of work with a goal and a lifecycle (planned,
    /// active, done, archived). Issues can be in several projects, or none.
    /// A project's default labels are added to issues when they join it.
    #[command(subcommand, visible_alias = "p")]
    Project(ProjectCommand),
    /// Manage views: named, saved issue filters.
    ///
    /// List a view's issues with `solilog issue list --view <name>`.
    #[command(subcommand, visible_alias = "v")]
    View(ViewCommand),
    /// Manage issue checklists.
    ///
    /// A checklist is an ordered list of one-line items, each checked or not,
    /// on an issue. It tracks progress only: an issue can be done with
    /// unchecked items. Items have ids of their own (shown by `issue show`,
    /// and in its JSON as `checklist[].id`); commands on an item take that
    /// id. Every command prints the issue.
    #[command(subcommand, visible_alias = "c")]
    Checklist(ChecklistCommand),
    /// Manage files attached to issues.
    ///
    /// Attaching copies a file into the attachments directory, next to the
    /// database (`<db name>-attachments/SOL-12/<file name>`); the original
    /// is left alone. Names are unique within an issue: a taken name gets a
    /// number (`notes (2).txt`). Attachments have ids of their own (shown by
    /// `issue show`, and in its JSON as `attachments[].id`, with the file's
    /// `path`). Deleting an issue deletes its attachments.
    #[command(subcommand, visible_alias = "a")]
    Attachment(AttachmentCommand),
    /// Manage issue links: where an issue's work lives outside Solilog.
    ///
    /// A link is a `branch` (by name), a `pr` (a pull or merge request, by
    /// URL) or a `url` (any other page: an upstream bug, a design, docs),
    /// with an optional title. Record the branch and the PR as soon as they
    /// exist, so the next session and the user find the work. Commits and
    /// changed files aren't links: `issue request-review` records them.
    ///
    /// Links have ids of their own (shown by `issue show`, and in its JSON
    /// as `links[]`: `id`, `kind`, `target`, `title`, `author`,
    /// `created_at`). `issue context` includes them. Every command prints
    /// the issue.
    #[command(subcommand, visible_alias = "k")]
    Link(LinkCommand),
    /// Manage issue notes: an issue's worklog.
    ///
    /// Notes are timestamped entries, each attributed to who wrote it (see
    /// `--actor`), added one after the other, apart from the description
    /// and the checklist: what was tried, what failed, what was decided,
    /// what the next session should know, steering from the user. They are
    /// never edited: add a new note to correct one. `issue context` includes
    /// them. Notes have ids of their own (`id` in the JSON); a note can be
    /// deleted only by the user or by its author.
    #[command(subcommand, visible_alias = "n")]
    Note(NoteCommand),
    /// Manage decisions: standing statements on an issue, label or project
    /// that agents read before starting work there.
    ///
    /// A decision records something settled ("Keep the v1 API; breaking
    /// changes wait for 2.0") so later sessions don't reopen it. It applies
    /// where it is recorded and below: a label's decisions apply to its
    /// sublabels and their issues, a project's to its issues. `issue
    /// context` and `where` include the decisions that apply.
    ///
    /// Decisions are never edited: `replace` records a new one in place of
    /// an old one (kept, `superseded`), `withdraw` ends one. Record what the
    /// user decided, not your own choices. A decision recorded or withdrawn
    /// by an agent is unreviewed (`reviewed: false`) until the user
    /// confirms it; it still applies meanwhile.
    ///
    /// JSON: `id`, `owner` (`{"kind": "issue", "id": "SOL-12"}`, `{"kind":
    /// "label", "name": …}` or `{"kind": "project", "name": …}`), `text`,
    /// `author`, `created_at`, `state` (`in_force`, `superseded`,
    /// `withdrawn`), `replaces`, `replaced_by`, `ended_at`, `ended_by`,
    /// `reviewed`.
    #[command(subcommand, visible_alias = "d")]
    Decision(DecisionCommand),
    /// Search issues' titles, descriptions, checklist items and notes, best
    /// matches first.
    ///
    /// Every word must appear somewhere in the issue (not necessarily in
    /// the same field), as the start of a word, ignoring case and accents:
    /// `migr` finds "migration". `"quoted words"` must appear together.
    /// Open issues only, unless `--include-closed` (done, cancelled and
    /// archived issues too: past bugs and decisions) or `--status` asks for
    /// closed ones.
    ///
    /// JSON: an array of `{issue, matches}`: `issue` as in `issue list
    /// --compact`; each match has `field` (`title`, `description`,
    /// `checklist_item`, `note`), `id` (the checklist item's or note's, else
    /// null) and `snippet` (the matching text on one line, shortened around
    /// the match for long text, matched words in `**`). Read a hit with
    /// `issue context`.
    #[command(visible_alias = "s")]
    Search {
        /// Words to look for (several arguments are joined).
        #[arg(required = true, value_name = "QUERY")]
        query: Vec<String>,
        /// Also search done, cancelled and archived issues.
        #[arg(long, short = 'a')]
        include_closed: bool,
        /// Only issues of the label or project mapped to the current
        /// directory (see `solilog where`), on top of the other filters.
        #[arg(long)]
        here: bool,
        /// Only issues changed at or after this time (see `issue list --since`).
        #[arg(long, value_name = "TIME")]
        since: Option<String>,
        /// At most this many issues.
        #[arg(long, short = 'n', value_name = "N", default_value_t = 20)]
        limit: usize,
        #[command(flatten)]
        filter: FilterArgs,
    },
    /// List the valid issue statuses, or collapse a status's section in the
    /// GUI issue list.
    ///
    /// JSON rows: `key`, `label`, `closed` (done and cancelled issues are
    /// closed) and `collapsed` (the GUI hides that section's issues).
    Statuses(StatusesArgs),
    /// Show keymap information.
    Keymap(KeymapArgs),
    /// Print the label or project the current directory belongs to: the one
    /// whose mapped directory (see `label add-path`, `project add-path`) is
    /// the deepest containing it. Fails when there is none, listing mapped
    /// directories missing on this machine.
    ///
    /// JSON: `kind` (`label` or `project`), `name`, `path` (as stored,
    /// `~/…` under the home directory), `absolute`, `exists`, and
    /// `decisions`: those in force for that label (with its ancestors') or
    /// project, most specific first (see `solilog decision`). Use it to
    /// find the issues of the codebase you are in (`issue list --here`).
    Where {
        /// Look up this directory instead of the current one.
        dir: Option<PathBuf>,
        /// List every mapped directory instead (JSON: an array of the same
        /// rows).
        #[arg(long, conflicts_with = "dir")]
        all: bool,
    },
    /// Print the database path.
    DbPath,
    /// Back up the database and its attachments, or restore a backup.
    ///
    /// A backup is a zip archive in the backup directory, which can be
    /// opened without Solilog: `manifest.json` (what it is), `solilog.db`
    /// (the database) and `attachments/SOL-12/<file>`. The backup directory
    /// is next to the database by default (`solilog-backups/`); `backup
    /// set-dir` changes it, e.g. to a synced folder.
    ///
    /// Backups are made automatically after changes (from the CLI or the
    /// app), at most once an hour. Older ones are thinned out: every backup
    /// of the last day is kept, then one per day for two weeks, one per week
    /// for three months and one per month for a year. Only this database's
    /// backups are thinned out or moved; the directory can be shared.
    ///
    /// A new version of Solilog that upgrades the database backs it up
    /// first (`before_upgrade` in the JSON, with the schema from before);
    /// those backups are kept at least 90 days.
    #[command(subcommand)]
    Backup(BackupCommand),
    /// Teach coding agents (Antigravity, Claude Code, Codex, Cursor, OpenCode,
    /// pi) to use Solilog.
    ///
    /// The skill is a `SKILL.md` built into this binary: it tells an agent
    /// when to use the `solilog` command (issue ids like SOL-12, "my
    /// issues", …) and how. `skill install` puts it where agents look:
    /// `~/.claude/skills/solilog/` for Claude Code,
    /// `~/.agents/skills/solilog/` for Codex, Cursor, OpenCode and pi, and
    /// `~/.gemini/config/skills/solilog/` for Antigravity. After upgrading
    /// Solilog, run `skill install` again to update it. Files Solilog did not
    /// write are never overwritten or removed.
    #[command(subcommand)]
    Skill(SkillCommand),
}

#[derive(Subcommand)]
pub enum SkillCommand {
    /// Print the skill's SKILL.md.
    Show,
    /// Install or update the skill and print what was written where.
    ///
    /// Without `--agent`, installs it for every agent whose configuration
    /// folder exists (`~/.gemini/antigravity`, `~/.claude`, `~/.codex`,
    /// `~/.cursor`, `~/.config/opencode`, `~/.pi/agent` or
    /// `$PI_CODING_AGENT_DIR`). Fails without writing anything if a target
    /// holds a skill Solilog did not write.
    Install {
        #[command(flatten)]
        agents: AgentArgs,
    },
    /// Remove the skill (all agents unless `--agent` is given) and print
    /// what was removed. Files Solilog did not write are left alone.
    Uninstall {
        #[command(flatten)]
        agents: AgentArgs,
    },
    /// Show where the skill is installed and whether it matches this binary
    /// (`missing`, `current`, `outdated` or `foreign`).
    Status {
        #[command(flatten)]
        agents: AgentArgs,
    },
}

#[derive(Args)]
pub struct AgentArgs {
    /// Only for this agent (repeatable): antigravity, claude, codex, cursor,
    /// opencode or pi. Codex, Cursor, OpenCode and pi share one folder.
    #[arg(long, value_name = "AGENT")]
    pub agent: Vec<Agent>,
}

#[derive(Subcommand)]
pub enum BackupCommand {
    /// Back up now and print the backup.
    Create,
    /// List the backups in the backup directory, newest first, including
    /// those of other databases (`"own": false` in the JSON).
    #[command(visible_alias = "ls")]
    List,
    /// Replace all data (issues, labels, projects, views, attachments) with
    /// a backup's. The current data is backed up first (`before_restore` in
    /// the JSON): restoring that backup undoes the restore. The backup
    /// directory stays as it is.
    Restore {
        /// File name in the backup directory (see `backup list`), or the
        /// path of a backup file.
        backup: String,
    },
    /// Print the backup directory.
    Dir,
    /// Change the backup directory (creating it if needed), move this
    /// database's backups there, and print it.
    #[command(group = clap::ArgGroup::new("target").required(true))]
    SetDir {
        #[arg(group = "target")]
        dir: Option<PathBuf>,
        /// Go back to the default directory, next to the database.
        #[arg(long, group = "target")]
        default: bool,
    },
}

#[derive(Subcommand)]
pub enum IssueCommand {
    /// List issues in the user-defined order (see `issue move`; new issues
    /// first). Archived issues are left out unless `--archived` is given;
    /// those are listed newest first.
    #[command(visible_alias = "ls")]
    List {
        /// Use a saved view's filter (see `solilog view`).
        #[arg(long, short, value_name = "VIEW", conflicts_with = "FilterArgs")]
        view: Option<String>,
        /// List archived issues instead (the filter still applies).
        #[arg(long)]
        archived: bool,
        /// Return lightweight JSON summaries (`id`, `title`, `status`,
        /// `labels`, `projects`, `checklist_progress`, `waiting_on`,
        /// `unreviewed`, `updated_at`, `archived`), omitting descriptions, full
        /// checklists, attachments and authorship details.
        #[arg(long)]
        compact: bool,
        /// Only the issues of the label or project mapped to the current
        /// directory (see `solilog where`), on top of the other filters.
        /// Fails when no label or project is mapped to it.
        #[arg(long)]
        here: bool,
        /// Only issues changed (by anyone) at or after this time: a Unix
        /// timestamp, an age (`30m`, `2h`, `3d`, `1w`) or a local date and
        /// time (`2026-11-25`, `2026-11-25 14:30`). See `issue activity
        /// --since` for what changed.
        #[arg(long, value_name = "TIME")]
        since: Option<String>,
        #[command(flatten)]
        filter: FilterArgs,
    },
    /// Show one issue, with its description, checklist and attachments.
    Show { id: IssueId },
    /// Show an issue together with its projects, dependencies, the `SOL-n`
    /// issues referenced in its description (`referenced_issues`), the
    /// issues whose description, checklist or notes mention it
    /// (`mentioned_by`), its notes and decisions, in one call.
    Context { id: IssueId },
    /// Create an issue and print it.
    #[command(visible_alias = "new")]
    Create {
        /// Title; `-` reads it from stdin.
        title: String,
        /// Plain-text description, any number of lines; `-` reads it from stdin.
        #[arg(long, short, default_value = "")]
        description: String,
        #[arg(long, short, default_value_t = Status::Todo)]
        status: Status,
        /// Add this label, creating it if needed (repeatable).
        #[arg(long, short, value_name = "LABEL")]
        label: Vec<String>,
        /// Add to this existing project, with its default labels (repeatable).
        #[arg(long, short, value_name = "PROJECT")]
        project: Vec<String>,
        /// Add this checklist item, unchecked (repeatable; items keep the
        /// order given).
        #[arg(long, short, value_name = "TEXT")]
        item: Vec<String>,
        /// Make it depend on this issue (repeatable; see `issue add-dependency`).
        #[arg(long, value_name = "ID")]
        depends_on: Vec<IssueId>,
        /// Give it the label, or put it in the project, mapped to the current
        /// directory (see `solilog where`). Fails when none is mapped to it.
        #[arg(long)]
        here: bool,
    },
    /// Change an issue (give at least one change) and print it. All the
    /// changes are made together, or none if one fails.
    ///
    /// Examples: `solilog issue edit SOL-12 --title "Fix the reverb tail"`,
    /// `git log -1 --format=%B | solilog issue edit SOL-12 --description -`,
    /// `solilog issue edit SOL-12 -d - --status todo -l bug --add-dependency SOL-7`.
    #[command(group = clap::ArgGroup::new("fields").required(true).multiple(true))]
    Edit {
        id: IssueId,
        /// New title; `-` reads it from stdin.
        #[arg(long, short, group = "fields")]
        title: Option<String>,
        /// New plain-text description (replaces the old one; `""` removes
        /// it); `-` reads it from stdin.
        #[arg(long, short, group = "fields")]
        description: Option<String>,
        /// New status (see `issue set-status`).
        #[arg(long, short, group = "fields")]
        status: Option<Status>,
        /// Add this label, creating it if needed (repeatable).
        #[arg(long, short = 'l', value_name = "LABEL", group = "fields")]
        add_label: Vec<String>,
        /// Remove this label; sublabels are not removed (repeatable).
        #[arg(long, value_name = "LABEL", group = "fields")]
        remove_label: Vec<String>,
        /// Add to this existing project, with its default labels (repeatable).
        #[arg(long, short = 'p', value_name = "PROJECT", group = "fields")]
        add_to_project: Vec<String>,
        /// Remove from this project; the issue keeps its labels (repeatable).
        #[arg(long, value_name = "PROJECT", group = "fields")]
        remove_from_project: Vec<String>,
        /// Make it depend on this issue (repeatable; see `issue add-dependency`).
        #[arg(long, value_name = "ID", group = "fields")]
        add_dependency: Vec<IssueId>,
        /// Make it no longer depend on this issue (repeatable).
        #[arg(long, value_name = "ID", group = "fields")]
        remove_dependency: Vec<IssueId>,
        /// Add a link, its kind inferred from the target (repeatable; see
        /// `link add`).
        #[arg(long, value_name = "TARGET", group = "fields")]
        add_link: Vec<String>,
        /// Remove this link, by id (repeatable).
        #[arg(long, value_name = "LINK_ID", group = "fields")]
        remove_link: Vec<LinkId>,
    },
    /// List the revisions of an issue's text, newest first: earlier titles,
    /// descriptions and checklist item texts, including deleted items.
    ///
    /// Overwriting a title, description or checklist item (here or in the
    /// app), or deleting an item, saves the old text as a revision; edits
    /// less than 10 minutes apart share one, except edits that remove much
    /// of the text. Older revisions are thinned out over time. Get one back
    /// with `issue restore`.
    History { id: IssueId },
    /// Put a revision's text back (see `issue history`) and print the issue.
    /// The current text is saved as a revision first, so a restore can be
    /// undone. A deleted checklist item comes back, unchecked, with its id.
    Restore {
        id: IssueId,
        /// Revision id, from `issue history`.
        revision: RevisionId,
    },
    /// List recorded changes, newest first (creation, edits, status,
    /// checklist, labels, projects, dependencies, attachments, notes): on
    /// one issue, or on every issue (archived ones included) without an id.
    ///
    /// To catch up after a while, e.g. what the user changed since your
    /// last run: `solilog issue activity --since 2026-11-25T14:30 --by user`.
    /// JSON rows: `id`, `issue_id`, `actor`, `kind`, `detail`, `created_at`.
    /// Deleted issues leave nothing behind.
    Activity {
        id: Option<IssueId>,
        /// Only non-user changes since the issue was last marked reviewed.
        #[arg(long, requires = "id", conflicts_with_all = ["since", "by", "not_by", "here", "limit"])]
        unreviewed: bool,
        /// Only changes at or after this time: a Unix timestamp, an age
        /// (`30m`, `2h`, `3d`, `1w`) or a local date and time (`2026-11-25`,
        /// `2026-11-25 14:30`; with `Z` or `+02:00`, in that zone).
        #[arg(long, value_name = "TIME")]
        since: Option<String>,
        /// Only changes by this actor (repeatable), e.g. `user`.
        #[arg(long, value_name = "ACTOR")]
        by: Vec<Actor>,
        /// Leave out changes by this actor (repeatable), e.g. yourself.
        #[arg(long, value_name = "ACTOR")]
        not_by: Vec<Actor>,
        /// Only on the issues of the label or project mapped to the current
        /// directory (see `solilog where`).
        #[arg(long, conflicts_with = "id")]
        here: bool,
        /// At most this many changes.
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
    /// Mark an issue's non-user changes as reviewed and print it, or use
    /// `--all` to mark every unreviewed issue reviewed. Any later non-user
    /// write marks an issue unreviewed again.
    #[command(group = clap::ArgGroup::new("target").required(true))]
    Review {
        #[arg(group = "target")]
        id: Option<IssueId>,
        /// Mark every non-archived unreviewed issue as reviewed.
        #[arg(long, group = "target")]
        all: bool,
    },
    /// Hand a finished issue to the user for review, with a receipt, and
    /// print the issue and the receipt.
    ///
    /// Sets the status to `in_review` and adds a receipt to the issue's
    /// notes: a short summary of what was done, the checks run (commands or
    /// tests) with pass or fail, and the commits and files changed. The user
    /// approves it in the app (it becomes `done`) or sends it back to
    /// `todo` with a note saying why. Report failing checks with `--fail`
    /// rather than leaving them out. Handing off again adds a new receipt;
    /// earlier ones stay.
    ///
    /// Example: `solilog issue request-review SOL-12 -m "Parser fixed; the
    /// old syntax still works." --pass "cargo test" --fail "cargo clippy: 1
    /// lint in tests" --commit 3f2a9c1 --file src/parser.rs`
    RequestReview {
        id: IssueId,
        /// What was done, in a few lines; `-` reads it from stdin.
        #[arg(short = 'm', long = "message", value_name = "SUMMARY")]
        summary: String,
        /// A check that passed: the command or test run (repeatable).
        #[arg(long = "pass", value_name = "COMMAND")]
        passed: Vec<String>,
        /// A check that failed, with a short reason if useful (repeatable).
        #[arg(long = "fail", value_name = "COMMAND")]
        failed: Vec<String>,
        /// A commit made for this issue: hash or other reference (repeatable).
        #[arg(long = "commit", value_name = "REF")]
        commits: Vec<String>,
        /// A file changed (repeatable).
        #[arg(long = "file", value_name = "PATH")]
        files: Vec<String>,
    },
    /// Approve issues in review (`in_review`): set them done and print them
    /// with the review note added. All or nothing. Meant for the user: an
    /// agent should only record an approval the user gave it.
    Approve {
        #[arg(required = true)]
        ids: Vec<IssueId>,
        /// An optional comment; `-` reads it from stdin.
        #[arg(short, long)]
        message: Option<String>,
    },
    /// Send issues in review (`in_review`) back to `todo` with a note saying
    /// what to change, and print them with that note. All or nothing.
    SendBack {
        #[arg(required = true)]
        ids: Vec<IssueId>,
        /// What to change; `-` reads it from stdin.
        #[arg(short, long)]
        message: String,
    },
    /// Change an issue's status and print it. The issue goes first in the
    /// order of its new status (unless it already had that status). To hand
    /// finished work to the user, use `issue request-review`.
    SetStatus { id: IssueId, status: Status },
    /// Reorder an issue before or after another issue with the same status,
    /// and print it. `issue list` and the GUI (which groups issues by
    /// status) use this order.
    #[command(group = clap::ArgGroup::new("placement").required(true))]
    Move {
        id: IssueId,
        /// Put it right before this issue.
        #[arg(long, value_name = "ID", group = "placement")]
        before: Option<IssueId>,
        /// Put it right after this issue.
        #[arg(long, value_name = "ID", group = "placement")]
        after: Option<IssueId>,
    },
    /// Archive an issue and print it. Archived issues keep their status,
    /// labels and projects, but are left out of listings, views and counts.
    Archive { id: IssueId },
    /// Bring an archived issue back and print it.
    Unarchive { id: IssueId },
    /// Delete an issue for good and print it as it was.
    #[command(visible_alias = "rm")]
    Delete { id: IssueId },
    /// Add labels to an issue (creating missing ones) and print it.
    AddLabel {
        id: IssueId,
        #[arg(required = true)]
        labels: Vec<String>,
    },
    /// Remove labels from an issue and print it. Sublabels are not removed.
    RemoveLabel {
        id: IssueId,
        #[arg(required = true)]
        labels: Vec<String>,
    },
    /// Add an issue to projects (giving it their default labels) and print it.
    AddToProject {
        id: IssueId,
        #[arg(required = true)]
        projects: Vec<String>,
    },
    /// Remove an issue from projects and print it. It keeps its labels.
    RemoveFromProject {
        id: IssueId,
        #[arg(required = true)]
        projects: Vec<String>,
    },
    /// Make an issue depend on other issues (work to finish first) and
    /// print it.
    ///
    /// Example: `solilog issue add-dependency SOL-12 SOL-7 SOL-9` (SOL-12
    /// depends on SOL-7 and SOL-9). The issue is "waiting" while one of
    /// them is open (not done or cancelled), "ready" once none is: see
    /// `issue list --ready`. Nothing changes a status by itself. An issue
    /// can't depend on itself, and dependencies can't form a cycle.
    AddDependency {
        id: IssueId,
        /// The issues it depends on.
        #[arg(required = true)]
        on: Vec<IssueId>,
    },
    /// Make an issue no longer depend on other issues and print it.
    RemoveDependency {
        id: IssueId,
        #[arg(required = true)]
        on: Vec<IssueId>,
    },
}

#[derive(Subcommand)]
pub enum ChecklistCommand {
    /// Add items at the end of an issue's checklist, unchecked.
    ///
    /// Example: `solilog checklist add SOL-12 "Write tests" "Update docs"`.
    /// With `-` as the only text, reads the items from stdin, one per line
    /// (blank lines are skipped).
    Add {
        id: IssueId,
        /// Item texts, one line each.
        #[arg(required = true)]
        texts: Vec<String>,
    },
    /// Check an item (mark it done).
    Check { item: ChecklistItemId },
    /// Uncheck an item.
    Uncheck { item: ChecklistItemId },
    /// Change the text of an item; `-` reads it from stdin.
    Edit { item: ChecklistItemId, text: String },
    /// Move an item before or after another item of the same checklist.
    #[command(group = clap::ArgGroup::new("placement").required(true))]
    Move {
        item: ChecklistItemId,
        /// Put it right before this item.
        #[arg(long, value_name = "ITEM", group = "placement")]
        before: Option<ChecklistItemId>,
        /// Put it right after this item.
        #[arg(long, value_name = "ITEM", group = "placement")]
        after: Option<ChecklistItemId>,
    },
    /// Delete an item.
    #[command(visible_alias = "rm")]
    Delete { item: ChecklistItemId },
}

#[derive(Subcommand)]
pub enum AttachmentCommand {
    /// Attach files to an issue (all or nothing) and print the new
    /// attachments.
    ///
    /// Example: `solilog attachment add SOL-12 crash.log screenshot.png`.
    Add {
        id: IssueId,
        /// Files to copy in (not folders).
        #[arg(required = true)]
        files: Vec<std::path::PathBuf>,
    },
    /// List an issue's attachments, in the order they were added.
    #[command(visible_alias = "ls")]
    List { id: IssueId },
    /// Delete an attachment and its file, and print it as it was.
    #[command(visible_alias = "rm")]
    Delete { attachment: AttachmentId },
}

#[derive(Subcommand)]
pub enum LinkCommand {
    /// Add a link to an issue and print the issue. Adding a link the issue
    /// already has (same kind and target) changes nothing.
    ///
    /// Examples: `solilog link add SOL-12 fix/reverb-tail` (a branch),
    /// `solilog link add SOL-12 https://github.com/acme/reverb/pull/31` (a
    /// PR), `solilog link add SOL-12 https://example.com/spec -t "Spec"`.
    /// Without `--kind`, a URL to a pull or merge request (GitHub, GitLab,
    /// Gitea, Bitbucket) is a `pr`, any other URL a `url`, anything else a
    /// `branch` (it must be a valid branch name).
    Add {
        id: IssueId,
        /// The branch name, or the URL.
        target: String,
        /// `branch`, `pr` or `url`; inferred from the target when left out.
        #[arg(long, short)]
        kind: Option<LinkKind>,
        /// What it is, in a few words (shown instead of the target).
        #[arg(long, short)]
        title: Option<String>,
    },
    /// List an issue's links, in the order they were added.
    #[command(visible_alias = "ls")]
    List { id: IssueId },
    /// Remove a link, by id, and print the issue it was on.
    #[command(visible_alias = "rm")]
    Delete { link: LinkId },
}

#[derive(Subcommand)]
pub enum NoteCommand {
    /// Add a note at the end of an issue's worklog and print it.
    ///
    /// Example: `solilog note add SOL-12 "Tried X: fails on Y. Next: Z."`,
    /// or `-` to read a longer, multi-line note from stdin. Markdown is
    /// fine; it is stored as written.
    Add {
        id: IssueId,
        /// The note; `-` reads it from stdin.
        text: String,
    },
    /// List an issue's notes, oldest first.
    #[command(visible_alias = "ls")]
    List { id: IssueId },
    /// Delete a note for good and print it as it was. Only the user and the
    /// note's author can.
    #[command(visible_alias = "rm")]
    Delete { note: NoteId },
}

/// Which issue, label or project a decision command is about.
#[derive(Args)]
#[group(required = true, multiple = false)]
pub struct DecisionOwnerArgs {
    /// On this issue.
    #[arg(long, value_name = "ID")]
    issue: Option<IssueId>,
    /// On this label (a sublabel's decisions are its own).
    #[arg(long, value_name = "LABEL")]
    label: Option<String>,
    /// On this project.
    #[arg(long, value_name = "PROJECT")]
    project: Option<String>,
    /// On the label or project mapped to the current directory (see
    /// `solilog where`).
    #[arg(long)]
    here: bool,
}

impl DecisionOwnerArgs {
    pub fn owner(self, db: &Db) -> Result<DecisionOwner> {
        Ok(match self {
            Self {
                issue: Some(id), ..
            } => DecisionOwner::issue(id),
            Self {
                label: Some(name), ..
            } => DecisionOwner::label(name),
            Self {
                project: Some(name),
                ..
            } => DecisionOwner::project(name),
            _ => db.require_path_owner(&current_dir()?)?.decision_owner(),
        })
    }
}

#[derive(Subcommand)]
pub enum DecisionCommand {
    /// Record a decision on an issue, label or project and print it.
    ///
    /// Example: `solilog decision add --label acme/reverb "Keep the v1 API;
    /// breaking changes wait for 2.0"`, or `-` to read it from stdin.
    /// Record what the user decided (in your session or elsewhere), not
    /// your own choices.
    #[command(visible_alias = "new")]
    Add {
        #[command(flatten)]
        owner: DecisionOwnerArgs,
        /// The decision; `-` reads it from stdin.
        text: String,
    },
    /// List the decisions recorded on an issue, label or project, oldest
    /// first: those in force, unless `--all`.
    #[command(visible_alias = "ls")]
    List {
        #[command(flatten)]
        owner: DecisionOwnerArgs,
        /// Also superseded and withdrawn decisions.
        #[arg(long, short, conflicts_with = "inherited")]
        all: bool,
        /// Every decision in force that applies, not only those recorded
        /// there: for an issue, its projects' and labels' too; for a label,
        /// its ancestors' (as in `issue context` and `where`).
        #[arg(long)]
        inherited: bool,
    },
    /// Show one decision.
    Show { decision: DecisionId },
    /// Replace a decision in force with a new one on the same issue, label
    /// or project, and print the new one. The old one is kept, superseded.
    Replace {
        decision: DecisionId,
        /// The new decision; `-` reads it from stdin.
        text: String,
    },
    /// Withdraw a decision in force (it no longer applies) and print it.
    Withdraw { decision: DecisionId },
    /// List the decisions awaiting the user's review (recorded or withdrawn
    /// by an agent), oldest first.
    Unreviewed,
    /// Confirm decisions as reviewed and print them. For the user: an
    /// agent shouldn't confirm its own.
    Review {
        #[arg(required = true)]
        decisions: Vec<DecisionId>,
    },
}

#[derive(Subcommand)]
pub enum ProjectCommand {
    /// List projects: open ones, then (with `--all`) closed ones, each in the
    /// user-defined order (see `project move`).
    #[command(visible_alias = "ls")]
    List {
        /// Include done and archived projects.
        #[arg(long, short)]
        all: bool,
    },
    /// Show one project.
    Show { name: String },
    /// Create a project and print it.
    #[command(visible_alias = "new")]
    Create {
        name: String,
        /// planned, active, done or archived.
        #[arg(long, short, default_value_t = ProjectStatus::Active)]
        status: ProjectStatus,
        /// What the project should achieve; `-` reads it from stdin.
        #[arg(long, short, default_value = "")]
        description: String,
        /// Default label, given to issues when they join (repeatable).
        #[arg(long, short, value_name = "LABEL")]
        label: Vec<String>,
    },
    /// Change a project's name, status or description, and print it.
    Update {
        name: String,
        /// New name.
        #[arg(long)]
        rename: Option<String>,
        /// planned, active, done or archived.
        #[arg(long, short)]
        status: Option<ProjectStatus>,
        /// New description; `-` reads it from stdin.
        #[arg(long, short)]
        description: Option<String>,
    },
    /// Move a project before or after another one in the project order (used
    /// by `project list` and the GUI sidebar), and print it. New projects go last.
    #[command(group = clap::ArgGroup::new("placement").required(true))]
    Move {
        name: String,
        /// Put it right before this project.
        #[arg(long, value_name = "PROJECT", group = "placement")]
        before: Option<String>,
        /// Put it right after this project.
        #[arg(long, value_name = "PROJECT", group = "placement")]
        after: Option<String>,
    },
    /// Delete a project and print it. Its issues stay, with their labels.
    #[command(visible_alias = "rm")]
    Delete { name: String },
    /// Add default labels to a project (creating missing ones) and print it.
    /// Issues already in the project are not changed.
    AddLabel {
        name: String,
        #[arg(required = true)]
        labels: Vec<String>,
    },
    /// Remove default labels from a project and print it. Issues keep them.
    RemoveLabel {
        name: String,
        #[arg(required = true)]
        labels: Vec<String>,
    },
    /// Map a directory where the project's codebase lives (default: the
    /// current one) to the project, and print it. See `solilog where`.
    ///
    /// A project can have several directories (a checkout and its
    /// worktrees); a directory belongs to one label or project only.
    AddPath {
        name: String,
        /// Directory; relative paths are from the current directory.
        #[arg(default_value = ".")]
        path: String,
    },
    /// Remove a directory from the project's paths (as listed, or any path
    /// to it), and print the project.
    RemovePath {
        name: String,
        #[arg(default_value = ".")]
        path: String,
    },
}

/// Issue filter flags, shared by `issue list` and `view save`.
#[derive(Args)]
pub struct FilterArgs {
    /// Only issues with this status (repeatable).
    #[arg(long, short)]
    status: Vec<Status>,
    /// Hide done and cancelled issues.
    #[arg(long)]
    open: bool,
    /// Only issues with this label or one of its sublabels (repeatable: all must match).
    #[arg(long, short, value_name = "LABEL")]
    label: Vec<String>,
    /// Exclude issues with this label or one of its sublabels (repeatable).
    #[arg(long, value_name = "LABEL")]
    without_label: Vec<String>,
    /// Only issues in this project (repeatable: any may match).
    #[arg(long, short, value_name = "PROJECT", conflicts_with = "no_project")]
    project: Vec<String>,
    /// Only issues that are in no project.
    #[arg(long)]
    no_project: bool,
    /// Only issues that can be started: open, not in review (`in_review`
    /// waits for the user), and every issue they depend on is done or
    /// cancelled (see `issue add-dependency`).
    #[arg(long, conflicts_with = "waiting")]
    ready: bool,
    /// Only open issues that depend on an issue still open.
    #[arg(long)]
    waiting: bool,
    /// Only issues with unreviewed non-user changes.
    #[arg(long)]
    unreviewed: bool,
}

impl From<FilterArgs> for IssueFilter {
    fn from(args: FilterArgs) -> Self {
        IssueFilter {
            statuses: args.status,
            open_only: args.open,
            labels: args.label,
            without_labels: args.without_label,
            projects: args.project,
            no_project: args.no_project,
            ready: args.ready,
            waiting: args.waiting,
            unreviewed: args.unreviewed,
            updated_since: None,
        }
    }
}

#[derive(Subcommand)]
pub enum ViewCommand {
    /// List saved views.
    #[command(visible_alias = "ls")]
    List,
    /// Show one view.
    Show { name: String },
    /// Save a view: create it, or replace the filter of an existing one, and print it.
    ///
    /// Takes the same filter flags as `issue list`, e.g.
    /// `solilog view save "Acme bugs" --open -l acme -l bug`.
    Save {
        name: String,
        #[command(flatten)]
        filter: FilterArgs,
    },
    /// Rename a view and print it.
    #[command(visible_alias = "mv")]
    Rename { name: String, new_name: String },
    /// Delete a view and print it.
    #[command(visible_alias = "rm")]
    Delete { name: String },
}

#[derive(Subcommand)]
pub enum LabelCommand {
    /// List labels as a tree (siblings in the order set with `label move`),
    /// with the number of issues under each.
    #[command(visible_alias = "ls")]
    List,
    /// Show one label.
    Show { name: String },
    /// Create a label (and missing ancestors) and print it.
    #[command(visible_alias = "new")]
    Create { name: String },
    /// Rename or move a label, with its sublabels, and print it.
    ///
    /// `solilog label rename acme globex/acme` moves `acme/reverb` to
    /// `globex/acme/reverb`. Issues keep their labels.
    #[command(visible_alias = "mv")]
    Rename { name: String, new_name: String },
    /// Reorder a label, with its sublabels, before or after a sibling (a
    /// label with the same parent), and print it. `label list` and the GUI
    /// sidebar use this order; new labels go last among their siblings. To
    /// change a label's parent, use `label rename`.
    #[command(group = clap::ArgGroup::new("placement").required(true))]
    Move {
        name: String,
        /// Put it right before this sibling.
        #[arg(long, value_name = "LABEL", group = "placement")]
        before: Option<String>,
        /// Put it right after this sibling.
        #[arg(long, value_name = "LABEL", group = "placement")]
        after: Option<String>,
    },
    /// Set the color of a label's chips and print the label. Sublabels
    /// without a color of their own are shown in it (`display_color` in
    /// JSON). Colors: red, orange, yellow, green, teal, blue, purple, pink,
    /// gray. Use `--reset` for the default color.
    #[command(group = clap::ArgGroup::new("color_or_reset").required(true))]
    Color {
        name: String,
        #[arg(group = "color_or_reset")]
        color: Option<LabelColor>,
        /// Go back to the default color.
        #[arg(long, group = "color_or_reset")]
        reset: bool,
    },
    /// Hide a label's sublabels in the GUI sidebar (display only: issues and
    /// filters are unaffected). Prints the label.
    Collapse { name: String },
    /// Show a collapsed label's sublabels again in the GUI sidebar. Prints
    /// the label.
    Expand { name: String },
    /// Delete a label and its sublabels, removing them from all issues.
    /// Prints the deleted labels.
    #[command(visible_alias = "rm")]
    Delete { name: String },
    /// Map a directory where the label's codebase lives (default: the
    /// current one) to the label, and print it. See `solilog where`.
    ///
    /// A label can have several directories (a checkout and its
    /// worktrees); a directory belongs to one label or project only.
    /// Directories inside it belong to it too, unless mapped themselves:
    /// map `acme` to `~/code/acme` and `acme/reverb` to
    /// `~/code/acme/reverb`.
    AddPath {
        name: String,
        /// Directory; relative paths are from the current directory.
        #[arg(default_value = ".")]
        path: String,
    },
    /// Remove a directory from the label's paths (as listed, or any path
    /// to it), and print the label.
    RemovePath {
        name: String,
        #[arg(default_value = ".")]
        path: String,
    },
}

#[derive(Args)]
pub struct StatusesArgs {
    #[command(subcommand)]
    pub command: Option<StatusesCommand>,
}

#[derive(Subcommand)]
pub enum StatusesCommand {
    /// Hide a status's issues in the GUI issue list, leaving its section
    /// header (display only: listings and filters are unaffected). Prints
    /// the statuses.
    Collapse { status: Status },
    /// Show a collapsed status's issues again in the GUI issue list. Prints
    /// the statuses.
    Expand { status: Status },
}

#[derive(Args)]
pub struct KeymapArgs {
    #[command(subcommand)]
    pub command: KeymapCommand,
}

#[derive(Subcommand)]
pub enum KeymapCommand {
    /// Print the user keymap path.
    Path,
    /// Print the built-in default keymap.
    Defaults,
    /// List every bindable GUI action.
    Actions,
}
