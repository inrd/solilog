//! Command-line interface. Designed for scripts and AI agents as much as
//! for humans:
//!
//! - every command accepts `--json` for stable, machine-readable output;
//! - write commands print the resulting object;
//! - errors go to stderr with a non-zero exit code (`--json`: as `{"error": …}`);
//! - issue ids are accepted as `SOL-12` or `12`;
//! - labels are `/`-separated paths (`acme/reverb`), matched case-insensitively.

use std::io::{Read as _, Write as _};

use anyhow::{Context as _, Result};
use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use serde_json::json;
use solilog_core::{
    Db, Issue, IssueFilter, IssueId, Label, NewIssue, NewProject, Project, ProjectStatus,
    ProjectUpdate, Status, View,
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
    /// List the valid issue statuses.
    Statuses,
    /// Show keymap information.
    Keymap(KeymapArgs),
    /// Print the database path.
    DbPath,
}

#[derive(Subcommand)]
pub enum IssueCommand {
    /// List issues, newest first.
    #[command(visible_alias = "ls")]
    List {
        /// Use a saved view's filter (see `solilog view`).
        #[arg(long, short, value_name = "VIEW", conflicts_with = "FilterArgs")]
        view: Option<String>,
        #[command(flatten)]
        filter: FilterArgs,
    },
    /// Show one issue.
    Show { id: IssueId },
    /// Create an issue and print it.
    #[command(visible_alias = "new")]
    Create {
        /// Title; `-` reads it from stdin.
        title: String,
        #[arg(long, short, default_value_t = Status::Todo)]
        status: Status,
        /// Add this label, creating it if needed (repeatable).
        #[arg(long, short, value_name = "LABEL")]
        label: Vec<String>,
        /// Add to this existing project, with its default labels (repeatable).
        #[arg(long, short, value_name = "PROJECT")]
        project: Vec<String>,
    },
    /// Change an issue's status and print it.
    SetStatus { id: IssueId, status: Status },
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
pub enum ProjectCommand {
    /// List projects: active, then planned, by target date.
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
        /// Target date, `YYYY-MM-DD`.
        #[arg(long, short, value_name = "DATE")]
        target_date: Option<String>,
        /// Default label, given to issues when they join (repeatable).
        #[arg(long, short, value_name = "LABEL")]
        label: Vec<String>,
    },
    /// Change a project's name, status, description or target date, and print it.
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
        /// New target date, `YYYY-MM-DD`.
        #[arg(long, short, value_name = "DATE", conflicts_with = "clear_target_date")]
        target_date: Option<String>,
        /// Remove the target date.
        #[arg(long)]
        clear_target_date: bool,
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
    /// List labels as a tree, with the number of issues under each.
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
    match command {
        Command::Issue(cmd) => {
            let db = Db::open_default()?;
            match cmd {
                IssueCommand::List { view, filter } => {
                    let filter = match view {
                        Some(name) => db.get_view(&name)?.filter,
                        None => filter.into(),
                    };
                    out.issues(&db.list_issues(&filter)?)
                }
                IssueCommand::Show { id } => out.issue(&db.get_issue(id)?),
                IssueCommand::Create {
                    title,
                    status,
                    label,
                    project,
                } => {
                    let new = NewIssue::new(read_arg(title, "title")?)
                        .status(status)
                        .labels(label)
                        .projects(project);
                    out.issue(&db.create_issue(&new)?)
                }
                IssueCommand::SetStatus { id, status } => out.issue(&db.set_status(id, status)?),
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
        Command::View(cmd) => {
            let db = Db::open_default()?;
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
            let db = Db::open_default()?;
            match cmd {
                ProjectCommand::List { all } => out.projects(&db.list_projects(all)?),
                ProjectCommand::Show { name } => out.project(&db.get_project(&name)?),
                ProjectCommand::Create {
                    name,
                    status,
                    description,
                    target_date,
                    label,
                } => {
                    let new = NewProject {
                        status,
                        description: read_arg(description, "description")?,
                        target_date,
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
                    target_date,
                    clear_target_date,
                } => {
                    let update = ProjectUpdate {
                        name: rename,
                        status,
                        description: description
                            .map(|d| read_arg(d, "description"))
                            .transpose()?,
                        target_date: if clear_target_date {
                            Some(None)
                        } else {
                            target_date.map(Some)
                        },
                    };
                    out.project(&db.update_project(&name, &update)?)
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
            let db = Db::open_default()?;
            match cmd {
                LabelCommand::List => out.labels(&db.list_labels()?),
                LabelCommand::Show { name } => out.label(&db.get_label(&name)?),
                LabelCommand::Create { name } => out.label(&db.create_label(&name)?),
                LabelCommand::Rename { name, new_name } => {
                    out.label(&db.rename_label(&name, &new_name)?)
                }
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
        self.print(issue, || issue_line(issue))
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
    line
}

fn view_line(view: &View) -> String {
    format!("{:<24} {}", view.name, view.filter)
}

/// Name, status, progress, target date, default labels.
fn project_line(project: &Project) -> String {
    let mut line = format!(
        "{:<24} {:<9} {}/{}",
        project.name,
        project.status.key(),
        project.closed_issue_count,
        project.issue_count
    );
    if let Some(date) = &project.target_date {
        line.push_str(&format!("  due {date}"));
    }
    if !project.labels.is_empty() {
        line.push_str(&format!("  [{}]", project.labels.join(", ")));
    }
    line
}

/// Full path and issue count.
fn label_line(label: &Label) -> String {
    format!("{} ({})", label.name, label.issue_count)
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
            format!("{}{leaf} ({})", "  ".repeat(l.depth()), l.issue_count)
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
