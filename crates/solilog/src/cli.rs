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
use solilog_core::{Db, Issue, IssueFilter, IssueId, Label, NewIssue, Status};

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
                IssueCommand::List {
                    status,
                    open,
                    label,
                    without_label,
                } => {
                    let filter = IssueFilter {
                        statuses: status,
                        open_only: open,
                        labels: label,
                        without_labels: without_label,
                        ..Default::default()
                    };
                    out.issues(&db.list_issues(&filter)?)
                }
                IssueCommand::Show { id } => out.issue(&db.get_issue(id)?),
                IssueCommand::Create {
                    title,
                    status,
                    label,
                } => {
                    let title = if title == "-" {
                        let mut buf = String::new();
                        std::io::stdin()
                            .read_to_string(&mut buf)
                            .context("reading title from stdin")?;
                        buf
                    } else {
                        title
                    };
                    let new = NewIssue::new(title).status(status).labels(label);
                    out.issue(&db.create_issue(&new)?)
                }
                IssueCommand::SetStatus { id, status } => out.issue(&db.set_status(id, status)?),
                IssueCommand::AddLabel { id, labels } => out.issue(&db.add_labels(id, &labels)?),
                IssueCommand::RemoveLabel { id, labels } => {
                    out.issue(&db.remove_labels(id, &labels)?)
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
