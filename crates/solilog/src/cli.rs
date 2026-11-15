//! Command-line interface. Designed for scripts and AI agents as much as
//! for humans:
//!
//! - every command accepts `--json` for stable, machine-readable output;
//! - write commands print the resulting object;
//! - errors go to stderr with a non-zero exit code (`--json`: as `{"error": …}`);
//! - issue ids are accepted as `SOL-12` or `12`;
//! - labels are `/`-separated paths (`acme/reverb`), matched case-insensitively.

use std::io::{IsTerminal as _, Read as _, Write as _};
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use serde_json::json;
use solilog_core::{
    Agent, Attachment, AttachmentId, Backup, BackupDir, BackupReason, ChecklistItemId, Db, Issue,
    IssueEdit, IssueFilter, IssueId, Label, LabelColor, NewIssue, NewProject, Placement, Project,
    ProjectStatus, ProjectUpdate, SKILL_NAME, SKILL_VERSION, SkillChange, SkillEnv, SkillState,
    SkillStatus, Status, View, format_size, format_utc,
};

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
    /// List the valid issue statuses.
    Statuses,
    /// Show keymap information.
    Keymap(KeymapArgs),
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
    #[command(subcommand)]
    Backup(BackupCommand),
    /// Teach coding agents (Claude Code, Codex, Cursor, OpenCode, pi) to use
    /// Solilog.
    ///
    /// The skill is a `SKILL.md` built into this binary: it tells an agent
    /// when to use the `solilog` command (issue ids like SOL-12, "my
    /// issues", …) and how. `skill install` puts it where agents look:
    /// `~/.claude/skills/solilog/` for Claude Code, and
    /// `~/.agents/skills/solilog/` for Codex, Cursor, OpenCode and pi. After
    /// upgrading Solilog, run `skill install` again to update it. Files
    /// Solilog did not write are never overwritten or removed.
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
    /// folder exists (`~/.claude`, `~/.codex`, `~/.cursor`,
    /// `~/.config/opencode`, `~/.pi/agent` or `$PI_CODING_AGENT_DIR`). Fails
    /// without writing anything if a target holds a skill Solilog did not
    /// write.
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
    /// Only for this agent (repeatable): claude, codex, cursor, opencode or
    /// pi. Codex, Cursor, OpenCode and pi share one folder.
    #[arg(long, value_name = "AGENT")]
    agent: Vec<Agent>,
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
        #[command(flatten)]
        filter: FilterArgs,
    },
    /// Show one issue, with its description, checklist and attachments.
    Show { id: IssueId },
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
    },
    /// Change an issue's fields (give at least one) and print it.
    ///
    /// Examples: `solilog issue edit SOL-12 --title "Fix the reverb tail"`,
    /// `git log -1 --format=%B | solilog issue edit SOL-12 --description -`.
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
    },
    /// Change an issue's status and print it. The issue goes first in the
    /// order of its new status (unless it already had that status).
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
}

#[derive(Args)]
pub struct KeymapArgs {
    #[command(subcommand)]
    command: KeymapCommand,
}

#[derive(Subcommand)]
enum KeymapCommand {
    /// Print the user keymap path.
    Path,
    /// Print the built-in default keymap.
    Defaults,
    /// List every bindable GUI action.
    Actions,
}

pub fn run(cli: Cli, command: Command) -> Result<()> {
    let out = Output { json: cli.json };
    let hint_skill = !cli.json && !matches!(command, Command::Skill(_));
    let mut opened = None;
    let result = run_command(&out, command, &mut opened);
    // Changes made from the CLI (e.g. by agents) are backed up too.
    if let Some(db) = opened
        && result.is_ok()
        && db.has_written()
        && let Err(err) = db.auto_backup()
    {
        eprintln!("warning: automatic backup failed: {err:#}");
    }
    if hint_skill && result.is_ok() && std::io::stderr().is_terminal() {
        hint_outdated_skill();
    }
    result
}

/// For people at a terminal (agents and scripts don't see it): a nudge to
/// update the agent skill after upgrading Solilog.
fn hint_outdated_skill() {
    let Ok(env) = SkillEnv::from_env() else {
        return;
    };
    if let Some(path) = solilog_core::outdated_skills(&env).first() {
        eprintln!(
            "note: the Solilog skill for AI agents in {} is out of date; \
             run `solilog skill install` to update it",
            path.parent().unwrap_or(path).display()
        );
    }
}

fn run_command(out: &Output, command: Command, opened: &mut Option<Db>) -> Result<()> {
    match command {
        Command::Issue(cmd) => {
            let db = &*opened.insert(Db::open_default()?);
            match cmd {
                IssueCommand::List {
                    view,
                    archived,
                    filter,
                } => {
                    let filter = match view {
                        Some(name) => db.get_view(&name)?.filter,
                        None => filter.into(),
                    };
                    out.issues(&if archived {
                        db.list_archived_issues(&filter)?
                    } else {
                        db.list_issues(&filter)?
                    })
                }
                IssueCommand::Show { id } => out.issue(&db.get_issue(id)?),
                IssueCommand::Create {
                    title,
                    description,
                    status,
                    label,
                    project,
                    item,
                } => {
                    check_one_stdin([&title, &description])?;
                    let new = NewIssue::new(read_arg(title, "title")?)
                        .description(read_arg(description, "description")?)
                        .status(status)
                        .labels(label)
                        .projects(project)
                        .checklist(item);
                    out.issue(&db.create_issue(&new)?)
                }
                IssueCommand::Edit {
                    id,
                    title,
                    description,
                } => {
                    check_one_stdin([&title, &description].into_iter().flatten())?;
                    let edit = IssueEdit {
                        title: title.map(|t| read_arg(t, "title")).transpose()?,
                        description: description
                            .map(|d| read_arg(d, "description"))
                            .transpose()?,
                    };
                    out.issue(&db.edit_issue(id, &edit)?)
                }
                IssueCommand::SetStatus { id, status } => out.issue(&db.set_status(id, status)?),
                IssueCommand::Move { id, before, after } => {
                    out.issue(&db.move_issue(id, &Placement::from_options(before, after)?)?)
                }
                IssueCommand::Archive { id } => out.issue(&db.set_archived(id, true)?),
                IssueCommand::Unarchive { id } => out.issue(&db.set_archived(id, false)?),
                IssueCommand::Delete { id } => out.issue(&db.delete_issue(id)?),
                IssueCommand::AddLabel { id, labels } => out.issue(&db.add_labels(id, &labels)?),
                IssueCommand::RemoveLabel { id, labels } => {
                    out.issue(&db.remove_labels(id, &labels)?)
                }
                IssueCommand::AddToProject { id, projects } => {
                    out.issue(&db.add_to_projects(id, &projects)?)
                }
                IssueCommand::RemoveFromProject { id, projects } => {
                    out.issue(&db.remove_from_projects(id, &projects)?)
                }
            }
        }
        Command::Checklist(cmd) => {
            let db = &*opened.insert(Db::open_default()?);
            let issue = match cmd {
                ChecklistCommand::Add { id, texts } => {
                    let texts = if texts == ["-"] {
                        read_arg(texts[0].clone(), "items")?
                            .lines()
                            .filter(|l| !l.trim().is_empty())
                            .map(str::to_owned)
                            .collect()
                    } else {
                        texts
                    };
                    db.add_checklist_items(id, &texts)?
                }
                ChecklistCommand::Check { item } => db.set_checklist_item_done(item, true)?,
                ChecklistCommand::Uncheck { item } => db.set_checklist_item_done(item, false)?,
                ChecklistCommand::Edit { item, text } => {
                    db.edit_checklist_item(item, &read_arg(text, "text")?)?
                }
                ChecklistCommand::Move {
                    item,
                    before,
                    after,
                } => db.move_checklist_item(item, &Placement::from_options(before, after)?)?,
                ChecklistCommand::Delete { item } => db.delete_checklist_item(item)?,
            };
            out.issue(&issue)
        }
        Command::Attachment(cmd) => {
            let db = &*opened.insert(Db::open_default()?);
            match cmd {
                AttachmentCommand::Add { id, files } => {
                    out.attachments(&db.add_attachments(id, &files)?)
                }
                AttachmentCommand::List { id } => out.attachments(&db.list_attachments(id)?),
                AttachmentCommand::Delete { attachment } => {
                    out.attachment(&db.delete_attachment(attachment)?)
                }
            }
        }
        Command::View(cmd) => {
            let db = &*opened.insert(Db::open_default()?);
            match cmd {
                ViewCommand::List => out.views(&db.list_views()?),
                ViewCommand::Show { name } => out.view(&db.get_view(&name)?),
                ViewCommand::Save { name, filter } => {
                    out.view(&db.save_view(&name, &filter.into())?)
                }
                ViewCommand::Rename { name, new_name } => {
                    out.view(&db.rename_view(&name, &new_name)?)
                }
                ViewCommand::Delete { name } => out.view(&db.delete_view(&name)?),
            }
        }
        Command::Project(cmd) => {
            let db = &*opened.insert(Db::open_default()?);
            match cmd {
                ProjectCommand::List { all } => out.projects(&db.list_projects(all)?),
                ProjectCommand::Show { name } => out.project(&db.get_project(&name)?),
                ProjectCommand::Create {
                    name,
                    status,
                    description,
                    label,
                } => {
                    let new = NewProject {
                        status,
                        description: read_arg(description, "description")?,
                        labels: label,
                        ..NewProject::new(name)
                    };
                    out.project(&db.create_project(&new)?)
                }
                ProjectCommand::Update {
                    name,
                    rename,
                    status,
                    description,
                } => {
                    let update = ProjectUpdate {
                        name: rename,
                        status,
                        description: description
                            .map(|d| read_arg(d, "description"))
                            .transpose()?,
                    };
                    out.project(&db.update_project(&name, &update)?)
                }
                ProjectCommand::Move {
                    name,
                    before,
                    after,
                } => {
                    let placement = Placement::from_options(before, after)?;
                    out.project(&db.move_project(&name, &placement)?)
                }
                ProjectCommand::Delete { name } => out.project(&db.delete_project(&name)?),
                ProjectCommand::AddLabel { name, labels } => {
                    out.project(&db.add_project_labels(&name, &labels)?)
                }
                ProjectCommand::RemoveLabel { name, labels } => {
                    out.project(&db.remove_project_labels(&name, &labels)?)
                }
            }
        }
        Command::Label(cmd) => {
            let db = &*opened.insert(Db::open_default()?);
            match cmd {
                LabelCommand::List => out.labels(&db.list_labels()?),
                LabelCommand::Show { name } => out.label(&db.get_label(&name)?),
                LabelCommand::Create { name } => out.label(&db.create_label(&name)?),
                LabelCommand::Rename { name, new_name } => {
                    out.label(&db.rename_label(&name, &new_name)?)
                }
                LabelCommand::Move {
                    name,
                    before,
                    after,
                } => {
                    let placement = Placement::from_options(before, after)?;
                    out.label(&db.move_label(&name, &placement)?)
                }
                LabelCommand::Color { name, color, .. } => {
                    out.label(&db.set_label_color(&name, color)?)
                }
                LabelCommand::Collapse { name } => out.label(&db.set_label_collapsed(&name, true)?),
                LabelCommand::Expand { name } => out.label(&db.set_label_collapsed(&name, false)?),
                LabelCommand::Delete { name } => out.labels(&db.delete_label(&name)?),
            }
        }
        Command::Statuses => {
            let rows: Vec<_> = Status::ALL
                .iter()
                .map(|s| json!({ "key": s.key(), "label": s.label(), "closed": s.is_closed() }))
                .collect();
            out.print(&rows, || {
                Status::ALL
                    .iter()
                    .map(|s| format!("{:<12} {}", s.key(), s.label()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
        }
        Command::Keymap(args) => match args.command {
            KeymapCommand::Path => {
                let path = crate::ui::keymap::user_keymap_path().context("no home directory")?;
                let path = path.display().to_string();
                out.print(&json!({ "path": path }), || path.clone())
            }
            KeymapCommand::Defaults => {
                print!("{}", crate::ui::keymap::DEFAULT_KEYMAP);
                Ok(())
            }
            KeymapCommand::Actions => {
                let mut actions: Vec<_> = gpui::generate_list_of_all_registered_actions()
                    .filter(|a| a.name != "zed::NoAction")
                    .map(|a| {
                        json!({
                            "name": a.name,
                            "description": a.documentation.map(str::trim),
                            "in_palette": crate::ui::actions::is_palette_visible(a.name),
                        })
                    })
                    .collect();
                actions.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                out.print(&actions, || {
                    actions
                        .iter()
                        .map(|a| {
                            let doc = a["description"].as_str().unwrap_or("");
                            let first = doc.lines().next().unwrap_or("");
                            format!("{:<32} {first}", a["name"].as_str().unwrap_or(""))
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                })
            }
        },
        Command::DbPath => {
            let path = solilog_core::default_path()?.display().to_string();
            out.print(&json!({ "path": path }), || path.clone())
        }
        Command::Backup(cmd) => {
            let db = opened.insert(Db::open_default()?);
            match cmd {
                BackupCommand::Create => out.backup(&db.create_backup(BackupReason::Manual)?),
                BackupCommand::List => out.backups(&db.list_backups()?),
                BackupCommand::Restore { backup } => {
                    let restored = db.restore_backup(&backup)?;
                    out.print(&restored, || {
                        format!(
                            "Restored {} ({}).\nThe data as it was before is in {}.",
                            restored.restored.name,
                            format_utc(restored.restored.created_at),
                            restored.before_restore.name
                        )
                    })
                }
                BackupCommand::Dir => {
                    let dir = db.backup_dir()?;
                    out.print(&dir, || backup_dir_line(&dir))
                }
                BackupCommand::SetDir { dir, .. } => {
                    let change = db.set_backup_dir(dir.as_deref())?;
                    out.print(&change, || {
                        let mut text = backup_dir_line(&change.dir);
                        text.push_str(&format!("\nMoved {} backups.", change.moved));
                        if change.not_moved > 0 {
                            text.push_str(&format!(
                                "\n{} not moved: a file by that name is already there.",
                                change.not_moved
                            ));
                        }
                        text
                    })
                }
            }
        }
        Command::Skill(cmd) => match cmd {
            SkillCommand::Show => {
                let content = solilog_core::skill_md();
                if out.json {
                    out.print(
                        &json!({ "name": SKILL_NAME, "version": SKILL_VERSION, "content": content }),
                        String::new,
                    )
                } else {
                    print!("{content}");
                    Ok(())
                }
            }
            SkillCommand::Install { agents } => {
                let env = SkillEnv::from_env()?;
                out.skill_changes(&solilog_core::install_skill(&env, &agents.agent)?)
            }
            SkillCommand::Uninstall { agents } => {
                let env = SkillEnv::from_env()?;
                out.skill_changes(&solilog_core::uninstall_skill(&env, &agents.agent)?)
            }
            SkillCommand::Status { agents } => {
                let env = SkillEnv::from_env()?;
                let statuses = solilog_core::skill_status(&env, &agents.agent)?;
                out.print(&statuses, || {
                    statuses
                        .iter()
                        .map(skill_status_line)
                        .collect::<Vec<_>>()
                        .join("\n")
                })
            }
        },
    }
}

/// `-` means "read from stdin".
fn read_arg(value: String, what: &str) -> Result<String> {
    if value != "-" {
        return Ok(value);
    }
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .with_context(|| format!("reading {what} from stdin"))?;
    Ok(buf)
}

/// Stdin can be read only once: at most one argument may be `-`.
fn check_one_stdin<'a>(values: impl IntoIterator<Item = &'a String>) -> Result<()> {
    let from_stdin = values.into_iter().filter(|v| *v == "-").count();
    anyhow::ensure!(
        from_stdin <= 1,
        "only one argument can be read from stdin (`-`)"
    );
    Ok(())
}

struct Output {
    json: bool,
}

impl Output {
    fn print<T: Serialize + ?Sized>(&self, value: &T, text: impl FnOnce() -> String) -> Result<()> {
        let mut stdout = std::io::stdout().lock();
        if self.json {
            serde_json::to_writer_pretty(&mut stdout, value)?;
            writeln!(stdout)?;
        } else {
            let text = text();
            if !text.is_empty() {
                writeln!(stdout, "{text}")?;
            }
        }
        Ok(())
    }

    fn issue(&self, issue: &Issue) -> Result<()> {
        self.print(issue, || {
            let mut text = issue_line(issue);
            if !issue.description.is_empty() {
                text.push_str(&format!("\n\n{}", issue.description));
            }
            if !issue.checklist.is_empty() {
                text.push('\n');
                for item in &issue.checklist {
                    let check = if item.done { "x" } else { " " };
                    text.push_str(&format!("\n[{check}] {:<4} {}", item.id.0, item.text));
                }
            }
            if !issue.attachments.is_empty() {
                text.push('\n');
                for attachment in &issue.attachments {
                    text.push('\n');
                    text.push_str(&attachment_line(attachment));
                }
            }
            text
        })
    }

    fn attachment(&self, attachment: &Attachment) -> Result<()> {
        self.print(attachment, || attachment_line(attachment))
    }

    fn attachments(&self, attachments: &[Attachment]) -> Result<()> {
        self.print(attachments, || {
            attachments
                .iter()
                .map(attachment_line)
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    fn backup(&self, backup: &Backup) -> Result<()> {
        self.print(backup, || backup_line(backup))
    }

    fn backups(&self, backups: &[Backup]) -> Result<()> {
        self.print(backups, || {
            backups
                .iter()
                .map(backup_line)
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    fn view(&self, view: &View) -> Result<()> {
        self.print(view, || view_line(view))
    }

    fn views(&self, views: &[View]) -> Result<()> {
        self.print(views, || {
            views.iter().map(view_line).collect::<Vec<_>>().join("\n")
        })
    }

    fn project(&self, project: &Project) -> Result<()> {
        self.print(project, || {
            let mut text = project_line(project);
            if !project.description.is_empty() {
                text.push_str(&format!("\n\n{}", project.description));
            }
            text
        })
    }

    fn projects(&self, projects: &[Project]) -> Result<()> {
        self.print(projects, || {
            projects
                .iter()
                .map(project_line)
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    fn label(&self, label: &Label) -> Result<()> {
        self.print(label, || label_line(label))
    }

    fn labels(&self, labels: &[Label]) -> Result<()> {
        self.print(labels, || label_tree(labels))
    }

    fn skill_changes(&self, changes: &[SkillChange]) -> Result<()> {
        self.print(changes, || {
            changes
                .iter()
                .map(|c| {
                    let mut line = format!(
                        "{:<13} {}  (for {})",
                        c.action.key(),
                        c.path.display(),
                        agent_list(&c.agents)
                    );
                    if !c.kept.is_empty() {
                        line.push_str("\n              kept the folder: it has other files");
                    }
                    line
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    fn issues(&self, issues: &[Issue]) -> Result<()> {
        self.print(issues, || {
            issues.iter().map(issue_line).collect::<Vec<_>>().join("\n")
        })
    }
}

fn issue_line(issue: &Issue) -> String {
    let mut line = format!(
        "{:<8} {:<12} {}",
        issue.id.to_string(),
        issue.status.key(),
        issue.title
    );
    if !issue.labels.is_empty() {
        line.push_str(&format!("  [{}]", issue.labels.join(", ")));
    }
    if !issue.projects.is_empty() {
        line.push_str(&format!("  in: {}", issue.projects.join(", ")));
    }
    if let Some((done, total)) = issue.checklist_progress() {
        line.push_str(&format!("  checklist {done}/{total}"));
    }
    if issue.archived {
        line.push_str("  (archived)");
    }
    line
}

/// Id, name, size and path.
fn attachment_line(attachment: &Attachment) -> String {
    let mut line = format!(
        "{:<4} {}  {}",
        attachment.id.0,
        attachment.name,
        solilog_core::format_size(attachment.size)
    );
    if let Some(path) = &attachment.path {
        line.push_str(&format!("  {}", path.display()));
    }
    line
}

fn backup_line(backup: &Backup) -> String {
    let other = if backup.own {
        ""
    } else {
        "  (another database)"
    };
    format!(
        "{}  {}  {:<14}  {} issues, {} attachments  {}{other}",
        backup.name,
        format_utc(backup.created_at),
        backup.reason.label(),
        backup.issue_count,
        backup.attachment_count,
        format_size(backup.size),
    )
}

fn backup_dir_line(dir: &BackupDir) -> String {
    let default = if dir.is_default { " (default)" } else { "" };
    format!("{}{default}", dir.path.display())
}

fn skill_status_line(status: &SkillStatus) -> String {
    let state = match (status.state, &status.installed_version) {
        (SkillState::Outdated, Some(v)) => format!("outdated ({v}; this is {SKILL_VERSION})"),
        (SkillState::Foreign, _) => "foreign (not written by Solilog)".to_owned(),
        (state, _) => state.key().to_owned(),
    };
    format!(
        "{state:<10} {}  (for {})",
        status.path.display(),
        agent_list(&status.agents)
    )
}

fn agent_list(agents: &[Agent]) -> String {
    agents
        .iter()
        .map(|a| a.key())
        .collect::<Vec<_>>()
        .join(", ")
}

fn view_line(view: &View) -> String {
    format!("{:<24} {}", view.name, view.filter)
}

/// Name, status, progress, default labels.
fn project_line(project: &Project) -> String {
    let mut line = format!(
        "{:<24} {:<9} {}/{}",
        project.name,
        project.status.key(),
        project.closed_issue_count,
        project.issue_count
    );
    if !project.labels.is_empty() {
        line.push_str(&format!("  [{}]", project.labels.join(", ")));
    }
    line
}

/// Full path and issue count.
fn label_line(label: &Label) -> String {
    match label.color {
        Some(color) => format!("{} ({}) {color}", label.name, label.issue_count),
        None => format!("{} ({})", label.name, label.issue_count),
    }
}

/// Indented tree: each label shows its last segment under its parent.
fn label_tree(labels: &[Label]) -> String {
    labels
        .iter()
        .map(|l| {
            let leaf = l
                .name
                .rsplit(solilog_core::LABEL_SEPARATOR)
                .next()
                .unwrap_or("");
            let color = l.color.map(|c| format!(" {c}")).unwrap_or_default();
            format!(
                "{}{leaf} ({}){color}",
                "  ".repeat(l.depth()),
                l.issue_count
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Prints an error the way the current output mode expects.
pub fn report_error(json: bool, err: &anyhow::Error) {
    if json {
        eprintln!("{}", json!({ "error": format!("{err:#}") }));
    } else {
        eprintln!("error: {err:#}");
    }
}
