//! Command-line interface. Designed for scripts and AI agents as much as
//! for humans:
//!
//! - every command accepts `--json` for stable, machine-readable output;
//! - write commands print the resulting object;
//! - errors go to stderr with a non-zero exit code (`--json`: as `{"error": …}`);
//! - issue ids are accepted as `SOL-12` or `12`.

use std::io::{Read as _, Write as _};

use anyhow::{Context as _, Result};
use clap::{Args, Parser, Subcommand};
use serde::Serialize;
use serde_json::json;
use solilog_core::{Db, Issue, IssueFilter, IssueId, Status};

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
    },
    /// Change an issue's status and print it.
    SetStatus { id: IssueId, status: Status },
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
                IssueCommand::List { status, open } => {
                    let filter = IssueFilter {
                        statuses: status,
                        open_only: open,
                    };
                    out.issues(&db.list_issues(&filter)?)
                }
                IssueCommand::Show { id } => out.issue(&db.get_issue(id)?),
                IssueCommand::Create { title, status } => {
                    let title = if title == "-" {
                        let mut buf = String::new();
                        std::io::stdin()
                            .read_to_string(&mut buf)
                            .context("reading title from stdin")?;
                        buf
                    } else {
                        title
                    };
                    out.issue(&db.create_issue(&title, status)?)
                }
                IssueCommand::SetStatus { id, status } => out.issue(&db.set_status(id, status)?),
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

    fn issues(&self, issues: &[Issue]) -> Result<()> {
        self.print(issues, || {
            issues.iter().map(issue_line).collect::<Vec<_>>().join("\n")
        })
    }
}

fn issue_line(issue: &Issue) -> String {
    format!(
        "{:<8} {:<12} {}",
        issue.id.to_string(),
        issue.status.key(),
        issue.title
    )
}

/// Prints an error the way the current output mode expects.
pub fn report_error(json: bool, err: &anyhow::Error) {
    if json {
        eprintln!("{}", json!({ "error": format!("{err:#}") }));
    } else {
        eprintln!("error: {err:#}");
    }
}
