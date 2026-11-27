//! Running a parsed command: one function per command group. Each gathers
//! its arguments, calls the core and hands the result to [`Output`].

use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde_json::json;
use solilog_core::{
    ActivityQuery, Actor, BackupReason, Check, Db, IssueChange, IssueEdit, IssueFilter, NewIssue,
    NewProject, Placement, ProjectUpdate, Receipt, SearchQuery, SkillEnv,
};

use super::args::{
    AttachmentCommand, BackupCommand, ChecklistCommand, Command, DecisionCommand, IssueCommand,
    KeymapCommand, LabelCommand, NoteCommand, ProjectCommand, SkillCommand, StatusesCommand,
    ViewCommand,
};
use super::text::Output;

/// Runs `command`. The database is opened only by the commands that need
/// it, into `opened`, so the caller can back up what they wrote.
pub fn command(
    out: &Output,
    command: Command,
    actor: Actor,
    opened: &mut Option<Db>,
) -> Result<()> {
    match command {
        Command::Issue(cmd) => issue(out, open_db(opened, actor)?, cmd),
        Command::Checklist(cmd) => checklist(out, open_db(opened, actor)?, cmd),
        Command::Attachment(cmd) => attachment(out, open_db(opened, actor)?, cmd),
        Command::Search {
            query,
            include_closed,
            here,
            since,
            limit,
            filter,
        } => {
            let db = &*open_db(opened, actor)?;
            let mut filter: IssueFilter = filter.into();
            if let Some(since) = since {
                filter.updated_since = Some(db.parse_time(&since)?);
            }
            if here {
                db.require_path_owner(&current_dir()?)?
                    .scope_filter(&mut filter);
            }
            out.search_hits(&db.search(&SearchQuery {
                text: query.join(" "),
                filter,
                include_closed,
                limit: Some(limit),
            })?)
        }
        Command::Note(cmd) => note(out, open_db(opened, actor)?, cmd),
        Command::Decision(cmd) => decision(out, open_db(opened, actor)?, cmd),
        Command::View(cmd) => view(out, opened.insert(Db::open_default()?), cmd),
        Command::Project(cmd) => project(out, opened.insert(Db::open_default()?), cmd),
        Command::Label(cmd) => label(out, opened.insert(Db::open_default()?), cmd),
        Command::Statuses(args) => statuses(out, opened.insert(Db::open_default()?), args.command),
        Command::Keymap(args) => keymap(out, args.command),
        Command::Where { dir, all } => where_(out, open_db(opened, actor)?, dir, all),
        Command::DbPath => out.path(&solilog_core::default_path()?),
        Command::Backup(cmd) => backup(out, opened.insert(Db::open_default()?), cmd),
        Command::Skill(cmd) => skill(out, cmd),
    }
}

fn open_db(opened: &mut Option<Db>, actor: Actor) -> Result<&mut Db> {
    Ok(opened.insert(Db::open_default()?.with_actor(actor)))
}

fn issue(out: &Output, db: &Db, cmd: IssueCommand) -> Result<()> {
    match cmd {
        IssueCommand::List {
            view,
            archived,
            compact,
            here,
            since,
            filter,
        } => {
            let mut filter = match view {
                Some(name) => db.get_view(&name)?.filter,
                None => filter.into(),
            };
            if let Some(since) = since {
                filter.updated_since = Some(db.parse_time(&since)?);
            }
            if here {
                db.require_path_owner(&current_dir()?)?
                    .scope_filter(&mut filter);
            }
            let issues = if archived {
                db.list_archived_issues(&filter)?
            } else {
                db.list_issues(&filter)?
            };
            if compact {
                out.compact_issues(&issues)
            } else {
                out.issues(&issues)
            }
        }
        IssueCommand::Show { id } => out.issue(&db.get_issue(id)?),
        IssueCommand::Context { id } => out.issue_context(&db.issue_context(id)?),
        IssueCommand::Create {
            title,
            description,
            status,
            label,
            project,
            item,
            depends_on,
            here,
        } => {
            check_one_stdin([&title, &description])?;
            let mut new = NewIssue::new(read_arg(title, "title")?)
                .description(read_arg(description, "description")?)
                .status(status)
                .labels(label)
                .projects(project)
                .checklist(item)
                .depends_on(depends_on);
            if here {
                db.require_path_owner(&current_dir()?)?
                    .scope_new_issue(&mut new);
            }
            out.issue(&db.create_issue(&new)?)
        }
        IssueCommand::Edit {
            id,
            title,
            description,
            status,
            add_label,
            remove_label,
            add_to_project,
            remove_from_project,
            add_dependency,
            remove_dependency,
        } => {
            check_one_stdin([&title, &description].into_iter().flatten())?;
            // Projects first, so that `--remove-label` can drop a
            // default label they add; the status last.
            let changes = [
                (!add_to_project.is_empty()).then_some(IssueChange::AddToProjects(add_to_project)),
                (!remove_from_project.is_empty())
                    .then_some(IssueChange::RemoveFromProjects(remove_from_project)),
                (!add_label.is_empty()).then_some(IssueChange::AddLabels(add_label)),
                (!remove_label.is_empty()).then_some(IssueChange::RemoveLabels(remove_label)),
                (!add_dependency.is_empty())
                    .then_some(IssueChange::AddDependencies(add_dependency)),
                (!remove_dependency.is_empty())
                    .then_some(IssueChange::RemoveDependencies(remove_dependency)),
                status.map(IssueChange::SetStatus),
            ];
            let edit = IssueEdit {
                title: title.map(|t| read_arg(t, "title")).transpose()?,
                description: description
                    .map(|d| read_arg(d, "description"))
                    .transpose()?,
                changes: changes.into_iter().flatten().collect(),
            };
            out.issue(&db.edit_issue(id, &edit)?)
        }
        IssueCommand::History { id } => out.revisions(&db.issue_revisions(id)?),
        IssueCommand::Restore { id, revision } => {
            let found = db.get_revision(revision)?;
            anyhow::ensure!(
                found.issue_id == id,
                "revision {revision} is of {}, not {id}",
                found.issue_id
            );
            out.issue(&db.restore_revision(revision)?)
        }
        IssueCommand::Activity {
            id,
            unreviewed,
            since,
            by,
            not_by,
            here,
            limit,
        } => {
            let activities = match id {
                Some(id) if unreviewed => db.unreviewed_activities(id)?,
                _ => {
                    let issues = if here {
                        let mut filter = IssueFilter::default();
                        db.require_path_owner(&current_dir()?)?
                            .scope_filter(&mut filter);
                        Some(filter)
                    } else {
                        None
                    };
                    db.activities(&ActivityQuery {
                        issue: id,
                        since: since.map(|s| db.parse_time(&s)).transpose()?,
                        by,
                        not_by,
                        issues,
                        limit,
                    })?
                }
            };
            if id.is_some() {
                out.activities(&activities)
            } else {
                out.feed(&activities)
            }
        }
        IssueCommand::Review { id, all } => {
            if all {
                out.issues(&db.mark_all_issues_reviewed()?)
            } else if let Some(id) = id {
                out.issue(&db.mark_issue_reviewed(id)?)
            } else {
                unreachable!("clap group requires id or --all")
            }
        }
        IssueCommand::RequestReview {
            id,
            summary,
            passed,
            failed,
            commits,
            files,
        } => {
            let check = |passed| move |command| Check { command, passed };
            let receipt = Receipt {
                checks: passed
                    .into_iter()
                    .map(check(true))
                    .chain(failed.into_iter().map(check(false)))
                    .collect(),
                commits,
                files,
            };
            let summary = read_arg(summary, "summary")?;
            out.handoff(&db.request_review(id, &summary, &receipt)?)
        }
        IssueCommand::Approve { ids, message } => {
            let message = message
                .map(|m| read_arg(m, "message"))
                .transpose()?
                .unwrap_or_default();
            out.handoffs(&db.approve_issues(&ids, &message)?)
        }
        IssueCommand::SendBack { ids, message } => {
            let message = read_arg(message, "message")?;
            out.handoffs(&db.send_back_issues(&ids, &message)?)
        }
        IssueCommand::SetStatus { id, status } => out.issue(&db.set_status(id, status)?),
        IssueCommand::Move { id, before, after } => {
            out.issue(&db.move_issue(id, &Placement::from_options(before, after)?)?)
        }
        IssueCommand::Archive { id } => out.issue(&db.set_archived(id, true)?),
        IssueCommand::Unarchive { id } => out.issue(&db.set_archived(id, false)?),
        IssueCommand::Delete { id } => out.issue(&db.delete_issue(id)?),
        IssueCommand::AddLabel { id, labels } => out.issue(&db.add_labels(id, &labels)?),
        IssueCommand::RemoveLabel { id, labels } => out.issue(&db.remove_labels(id, &labels)?),
        IssueCommand::AddToProject { id, projects } => {
            out.issue(&db.add_to_projects(id, &projects)?)
        }
        IssueCommand::RemoveFromProject { id, projects } => {
            out.issue(&db.remove_from_projects(id, &projects)?)
        }
        IssueCommand::AddDependency { id, on } => out.issue(&db.add_dependencies(id, &on)?),
        IssueCommand::RemoveDependency { id, on } => out.issue(&db.remove_dependencies(id, &on)?),
    }
}

fn checklist(out: &Output, db: &Db, cmd: ChecklistCommand) -> Result<()> {
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

fn attachment(out: &Output, db: &Db, cmd: AttachmentCommand) -> Result<()> {
    match cmd {
        AttachmentCommand::Add { id, files } => out.attachments(&db.add_attachments(id, &files)?),
        AttachmentCommand::List { id } => out.attachments(&db.list_attachments(id)?),
        AttachmentCommand::Delete { attachment } => {
            out.attachment(&db.delete_attachment(attachment)?)
        }
    }
}

fn note(out: &Output, db: &Db, cmd: NoteCommand) -> Result<()> {
    match cmd {
        NoteCommand::Add { id, text } => out.note(&db.add_note(id, &read_arg(text, "note")?)?),
        NoteCommand::List { id } => out.notes(&db.issue_notes(id)?),
        NoteCommand::Delete { note } => out.note(&db.delete_note(note)?),
    }
}

fn decision(out: &Output, db: &Db, cmd: DecisionCommand) -> Result<()> {
    match cmd {
        DecisionCommand::Add { owner, text } => {
            out.decision(&db.record_decision(&owner.owner(db)?, &read_arg(text, "decision")?)?)
        }
        DecisionCommand::List {
            owner,
            all,
            inherited,
        } => {
            let owner = owner.owner(db)?;
            if inherited {
                out.decisions(&db.inherited_decisions(&owner)?)
            } else {
                out.decisions(&db.decisions(&owner, all)?)
            }
        }
        DecisionCommand::Show { decision } => out.decision(&db.get_decision(decision)?),
        DecisionCommand::Replace { decision, text } => {
            out.decision(&db.replace_decision(decision, &read_arg(text, "decision")?)?)
        }
        DecisionCommand::Withdraw { decision } => out.decision(&db.withdraw_decision(decision)?),
        DecisionCommand::Unreviewed => out.decisions(&db.unreviewed_decisions()?),
        DecisionCommand::Review { decisions } => {
            out.decisions(&db.mark_decisions_reviewed(&decisions)?)
        }
    }
}

fn view(out: &Output, db: &Db, cmd: ViewCommand) -> Result<()> {
    match cmd {
        ViewCommand::List => out.views(&db.list_views()?),
        ViewCommand::Show { name } => out.view(&db.get_view(&name)?),
        ViewCommand::Save { name, filter } => out.view(&db.save_view(&name, &filter.into())?),
        ViewCommand::Rename { name, new_name } => out.view(&db.rename_view(&name, &new_name)?),
        ViewCommand::Delete { name } => out.view(&db.delete_view(&name)?),
    }
}

fn project(out: &Output, db: &Db, cmd: ProjectCommand) -> Result<()> {
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
        ProjectCommand::AddPath { name, path } => {
            out.project(&db.add_project_path(&name, &path_arg(&path)?)?)
        }
        ProjectCommand::RemovePath { name, path } => {
            out.project(&db.remove_project_path(&name, &path_arg(&path)?)?)
        }
        ProjectCommand::RemoveLabel { name, labels } => {
            out.project(&db.remove_project_labels(&name, &labels)?)
        }
    }
}

fn label(out: &Output, db: &Db, cmd: LabelCommand) -> Result<()> {
    match cmd {
        LabelCommand::List => out.labels(&db.list_labels()?),
        LabelCommand::Show { name } => out.label(&db.get_label(&name)?),
        LabelCommand::Create { name } => out.label(&db.create_label(&name)?),
        LabelCommand::Rename { name, new_name } => out.label(&db.rename_label(&name, &new_name)?),
        LabelCommand::Move {
            name,
            before,
            after,
        } => {
            let placement = Placement::from_options(before, after)?;
            out.label(&db.move_label(&name, &placement)?)
        }
        LabelCommand::Color { name, color, .. } => out.label(&db.set_label_color(&name, color)?),
        LabelCommand::Collapse { name } => out.label(&db.set_label_collapsed(&name, true)?),
        LabelCommand::Expand { name } => out.label(&db.set_label_collapsed(&name, false)?),
        LabelCommand::Delete { name } => out.labels(&db.delete_label(&name)?),
        LabelCommand::AddPath { name, path } => {
            out.label(&db.add_label_path(&name, &path_arg(&path)?)?)
        }
        LabelCommand::RemovePath { name, path } => {
            out.label(&db.remove_label_path(&name, &path_arg(&path)?)?)
        }
    }
}

fn statuses(out: &Output, db: &Db, cmd: Option<StatusesCommand>) -> Result<()> {
    let collapsed = match cmd {
        None => db.collapsed_statuses()?,
        Some(StatusesCommand::Collapse { status }) => db.set_status_collapsed(status, true)?,
        Some(StatusesCommand::Expand { status }) => db.set_status_collapsed(status, false)?,
    };
    out.statuses(&collapsed)
}

fn keymap(out: &Output, cmd: KeymapCommand) -> Result<()> {
    match cmd {
        KeymapCommand::Path => {
            out.path(&crate::ui::keymap::user_keymap_path().context("no home directory")?)
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
            out.actions(&actions)
        }
    }
}

fn where_(out: &Output, db: &Db, dir: Option<PathBuf>, all: bool) -> Result<()> {
    if all {
        return out.mappings(&db.list_paths()?);
    }
    let dir = match dir {
        Some(dir) => current_dir()?.join(dir),
        None => current_dir()?,
    };
    let found = db.require_path_owner(&dir)?;
    let decisions = db.inherited_decisions(&found.decision_owner())?;
    out.path_owner(&found, &decisions)
}

fn backup(out: &Output, db: &mut Db, cmd: BackupCommand) -> Result<()> {
    match cmd {
        BackupCommand::Create => out.backup(&db.create_backup(BackupReason::Manual)?),
        BackupCommand::List => out.backups(&db.list_backups()?),
        BackupCommand::Restore { backup } => out.restored(&db.restore_backup(&backup)?),
        BackupCommand::Dir => out.backup_dir(&db.backup_dir()?),
        BackupCommand::SetDir { dir, .. } => {
            out.backup_dir_change(&db.set_backup_dir(dir.as_deref())?)
        }
    }
}

fn skill(out: &Output, cmd: SkillCommand) -> Result<()> {
    match cmd {
        SkillCommand::Show => out.skill(&solilog_core::skill_md()),
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
            out.skill_statuses(&solilog_core::skill_status(&env, &agents.agent)?)
        }
    }
}

pub fn current_dir() -> Result<PathBuf> {
    std::env::current_dir().context("cannot read the current directory")
}

/// A directory argument made absolute (relative to the current directory);
/// `~/…` is left for the core to expand.
fn path_arg(path: &str) -> Result<String> {
    let trimmed = path.trim();
    if trimmed == "~" || trimmed.starts_with("~/") || Path::new(trimmed).is_absolute() {
        return Ok(trimmed.to_owned());
    }
    current_dir()?
        .join(trimmed)
        .to_str()
        .map(str::to_owned)
        .context("the path is not valid UTF-8")
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
