//! Output: JSON (`--json`, the core types serialized) or human-readable text.

use std::io::Write as _;
use std::path::Path;

use anyhow::Result;
use serde::Serialize;
use serde_json::json;
use solilog_core::{
    Activity, Actor, Agent, Attachment, Backup, BackupDir, BackupDirChange, CompactIssue, Decision,
    DecisionState, Handoff, Issue, IssueContext, IssueId, IssueSummary, Label, MappedPath, Note,
    NoteKind, PathMapping, Project, ProjectSummary, Restored, Revision, SKILL_NAME, SKILL_VERSION,
    SearchField, SearchHit, SkillChange, SkillState, SkillStatus, Status, Verdict, View,
    format_age, format_size, format_utc,
};

pub struct Output {
    pub json: bool,
}

impl Output {
    pub fn print<T: Serialize + ?Sized>(
        &self,
        value: &T,
        text: impl FnOnce() -> String,
    ) -> Result<()> {
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

    pub fn issue(&self, issue: &Issue) -> Result<()> {
        self.print(issue, || issue_text(issue))
    }

    pub fn issue_context(&self, ctx: &IssueContext) -> Result<()> {
        self.print(ctx, || {
            let mut text = issue_text(&ctx.issue);
            if !ctx.projects.is_empty() {
                text.push_str("\n\nProjects:");
                for p in &ctx.projects {
                    text.push('\n');
                    text.push_str(&project_summary_line(p));
                }
            }
            if !ctx.depends_on.is_empty() {
                text.push_str("\n\nDepends on:");
                for i in &ctx.depends_on {
                    text.push('\n');
                    text.push_str(&issue_summary_line(i));
                }
            }
            if !ctx.referenced_issues.is_empty() {
                text.push_str("\n\nReferenced:");
                for i in &ctx.referenced_issues {
                    text.push('\n');
                    text.push_str(&issue_summary_line(i));
                }
            }
            if !ctx.mentioned_by.is_empty() {
                text.push_str("\n\nMentioned by:");
                for i in &ctx.mentioned_by {
                    text.push('\n');
                    text.push_str(&issue_summary_line(i));
                }
            }
            if !ctx.decisions.is_empty() {
                text.push_str("\n\nDecisions:\n");
                text.push_str(&decisions_text(&ctx.decisions));
            }
            if !ctx.notes.is_empty() {
                text.push_str("\n\nNotes:\n");
                text.push_str(&notes_text(&ctx.notes));
            }
            text
        })
    }

    pub fn note(&self, note: &Note) -> Result<()> {
        self.print(note, || note_text(note))
    }

    pub fn decision(&self, decision: &Decision) -> Result<()> {
        self.print(decision, || decision_text(decision))
    }

    pub fn decisions(&self, decisions: &[Decision]) -> Result<()> {
        self.print(decisions, || decisions_text(decisions))
    }

    pub fn handoff(&self, handoff: &Handoff) -> Result<()> {
        self.print(handoff, || handoff_text(handoff))
    }

    pub fn handoffs(&self, handoffs: &[Handoff]) -> Result<()> {
        self.print(handoffs, || {
            handoffs
                .iter()
                .map(handoff_text)
                .collect::<Vec<_>>()
                .join("\n\n")
        })
    }

    pub fn notes(&self, notes: &[Note]) -> Result<()> {
        self.print(notes, || notes_text(notes))
    }

    pub fn revisions(&self, revisions: &[Revision]) -> Result<()> {
        self.print(revisions, || {
            revisions
                .iter()
                .map(revision_line)
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    /// Activities across issues: each line names its issue.
    pub fn feed(&self, activities: &[Activity]) -> Result<()> {
        self.print(activities, || {
            activities
                .iter()
                .map(|a| format!("{:<8} {}", a.issue_id.to_string(), activity_line(a)))
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    pub fn search_hits(&self, hits: &[SearchHit]) -> Result<()> {
        self.print(hits, || {
            hits.iter()
                .map(|hit| {
                    let mut text = compact_issue_line(&hit.issue);
                    for m in &hit.matches {
                        if m.field == SearchField::Title {
                            continue;
                        }
                        let field = match (m.field, m.id) {
                            (SearchField::ChecklistItem, Some(id)) => format!("item {id}"),
                            (SearchField::Note, Some(id)) => format!("note {id}"),
                            (field, _) => field.key().to_owned(),
                        };
                        text.push_str(&format!("\n    {field:<12} {}", m.snippet));
                    }
                    text
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    pub fn activities(&self, activities: &[Activity]) -> Result<()> {
        self.print(activities, || {
            activities
                .iter()
                .map(activity_line)
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    pub fn attachment(&self, attachment: &Attachment) -> Result<()> {
        self.print(attachment, || attachment_line(attachment))
    }

    pub fn attachments(&self, attachments: &[Attachment]) -> Result<()> {
        self.print(attachments, || {
            attachments
                .iter()
                .map(attachment_line)
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    pub fn backup(&self, backup: &Backup) -> Result<()> {
        self.print(backup, || backup_line(backup))
    }

    pub fn backups(&self, backups: &[Backup]) -> Result<()> {
        self.print(backups, || {
            backups
                .iter()
                .map(backup_line)
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    pub fn view(&self, view: &View) -> Result<()> {
        self.print(view, || view_line(view))
    }

    pub fn views(&self, views: &[View]) -> Result<()> {
        self.print(views, || {
            views.iter().map(view_line).collect::<Vec<_>>().join("\n")
        })
    }

    pub fn project(&self, project: &Project) -> Result<()> {
        self.print(project, || {
            let mut text = project_line(project);
            if !project.description.is_empty() {
                text.push_str(&format!("\n\n{}", project.description));
            }
            text
        })
    }

    pub fn projects(&self, projects: &[Project]) -> Result<()> {
        self.print(projects, || {
            projects
                .iter()
                .map(project_line)
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    pub fn label(&self, label: &Label) -> Result<()> {
        self.print(label, || label_line(label))
    }

    pub fn labels(&self, labels: &[Label]) -> Result<()> {
        self.print(labels, || label_tree(labels))
    }

    pub fn skill_changes(&self, changes: &[SkillChange]) -> Result<()> {
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

    pub fn issues(&self, issues: &[Issue]) -> Result<()> {
        self.print(issues, || {
            issues.iter().map(issue_line).collect::<Vec<_>>().join("\n")
        })
    }

    pub fn compact_issues(&self, issues: &[Issue]) -> Result<()> {
        if self.json {
            let compact: Vec<CompactIssue> = issues.iter().map(CompactIssue::from).collect();
            self.print(&compact, String::new)
        } else {
            self.issues(issues)
        }
    }

    /// Every status, with the ones collapsed in the GUI marked.
    pub fn statuses(&self, collapsed: &[Status]) -> Result<()> {
        let rows: Vec<_> = Status::ALL
            .iter()
            .map(|s| {
                json!({
                    "key": s.key(),
                    "label": s.label(),
                    "closed": s.is_closed(),
                    "collapsed": collapsed.contains(s),
                })
            })
            .collect();
        self.print(&rows, || {
            Status::ALL
                .iter()
                .map(|s| {
                    let line = format!("{:<12} {}", s.key(), s.label());
                    if collapsed.contains(s) {
                        format!("{line:<25} (collapsed)")
                    } else {
                        line
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    /// GUI actions, as `{name, description, in_palette}` objects.
    pub fn actions(&self, actions: &[serde_json::Value]) -> Result<()> {
        self.print(actions, || {
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

    /// A file path: `{"path": …}` in JSON.
    pub fn path(&self, path: &Path) -> Result<()> {
        let path = path.display().to_string();
        self.print(&json!({ "path": path }), || path.clone())
    }

    pub fn mappings(&self, mappings: &[PathMapping]) -> Result<()> {
        self.print(mappings, || {
            mappings
                .iter()
                .map(mapping_line)
                .collect::<Vec<_>>()
                .join("\n")
        })
    }

    /// The label or project a directory belongs to, with the decisions
    /// that apply there.
    pub fn path_owner(&self, mapping: &PathMapping, decisions: &[Decision]) -> Result<()> {
        let result = WhereResult { mapping, decisions };
        self.print(&result, || {
            let mut text = mapping_line(mapping);
            if !decisions.is_empty() {
                text.push_str("\n\nDecisions:\n");
                text.push_str(&decisions_text(decisions));
            }
            text
        })
    }

    pub fn restored(&self, restored: &Restored) -> Result<()> {
        self.print(restored, || {
            format!(
                "Restored {} ({}).\nThe data as it was before is in {}.",
                restored.restored.name,
                format_utc(restored.restored.created_at),
                restored.before_restore.name
            )
        })
    }

    pub fn backup_dir(&self, dir: &BackupDir) -> Result<()> {
        self.print(dir, || backup_dir_line(dir))
    }

    pub fn backup_dir_change(&self, change: &BackupDirChange) -> Result<()> {
        self.print(change, || {
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

    /// The agent skill itself: as written, or with its name and version
    /// in JSON.
    pub fn skill(&self, content: &str) -> Result<()> {
        if self.json {
            self.print(
                &json!({ "name": SKILL_NAME, "version": SKILL_VERSION, "content": content }),
                String::new,
            )
        } else {
            print!("{content}");
            Ok(())
        }
    }

    pub fn skill_statuses(&self, statuses: &[SkillStatus]) -> Result<()> {
        self.print(statuses, || {
            statuses
                .iter()
                .map(skill_status_line)
                .collect::<Vec<_>>()
                .join("\n")
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
    if issue.is_waiting() {
        line.push_str(&format!("  waiting on: {}", id_list(&issue.waiting_on)));
    }
    if issue.unreviewed {
        let actors: Vec<&str> = issue.unreviewed_actors.iter().map(Actor::as_str).collect();
        line.push_str(&format!("  unreviewed ({})", actors.join(", ")));
    }
    if issue.archived {
        line.push_str("  (archived)");
    }
    line
}

/// Like [`issue_line`], for a compact issue.
fn compact_issue_line(issue: &CompactIssue) -> String {
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
    if issue.archived {
        line.push_str("  (archived)");
    }
    line
}

fn issue_text(issue: &Issue) -> String {
    let mut text = issue_line(issue);
    if !issue.description.is_empty() {
        text.push_str(&format!("\n\n{}", issue.description));
    }
    if !issue.depends_on.is_empty() || !issue.dependents.is_empty() {
        text.push('\n');
    }
    if !issue.depends_on.is_empty() {
        let deps: Vec<String> = issue
            .depends_on
            .iter()
            .map(|id| {
                let state = if issue.waiting_on.contains(id) {
                    "open"
                } else {
                    "closed"
                };
                format!("{id} ({state})")
            })
            .collect();
        text.push_str(&format!("\nDepends on: {}", deps.join(", ")));
    }
    if !issue.dependents.is_empty() {
        text.push_str(&format!("\nNeeded by: {}", id_list(&issue.dependents)));
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
}

fn project_summary_line(project: &ProjectSummary) -> String {
    let mut line = format!("  {} ({})", project.name, project.status.key());
    if !project.description.is_empty() {
        line.push_str(&format!(": {}", project.description));
    }
    line
}

fn issue_summary_line(issue: &IssueSummary) -> String {
    let mut line = format!(
        "  {:<8} {:<12} {}",
        issue.id,
        issue.status.key(),
        issue.title
    );
    if !issue.summary.is_empty() {
        line.push_str(&format!(" — {}", issue.summary));
    }
    line
}

/// `SOL-1, SOL-2`.
fn id_list(ids: &[IssueId]) -> String {
    ids.iter()
        .map(IssueId::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Id, age, actor, field, reason and the start of the text.
fn revision_line(revision: &Revision) -> String {
    format!(
        "{:<4} {:<12} {:<16} {:<28} {:<14} {}",
        revision.id.0,
        format_age(revision.created_at),
        revision.author,
        revision.field_label(),
        revision.reason.label(),
        revision.preview(60)
    )
}

/// Id, age, actor and summary.
fn activity_line(activity: &Activity) -> String {
    format!(
        "{:<4} {:<12} {:<16} {}",
        activity.id.0,
        format_age(activity.created_at),
        activity.actor,
        activity.summary()
    )
}

/// Id, age, author and kind, then the text and the kind's fields, indented.
fn note_text(note: &Note) -> String {
    let kind = match &note.kind {
        NoteKind::Note => "",
        NoteKind::Receipt(_) => "  receipt",
        NoteKind::Review {
            verdict: Verdict::Approved,
        } => "  approved",
        NoteKind::Review {
            verdict: Verdict::ChangesRequested,
        } => "  sent back",
    };
    let mut text = format!(
        "{:<4} {:<12} {}{kind}",
        note.id.0,
        format_age(note.created_at),
        note.author
    );
    let mut lines: Vec<String> = note.text.lines().map(str::to_owned).collect();
    if let NoteKind::Receipt(receipt) = &note.kind {
        for check in &receipt.checks {
            let mark = if check.passed { "pass" } else { "FAIL" };
            lines.push(format!("{mark}  {}", check.command));
        }
        lines.extend(receipt.commits.iter().map(|c| format!("commit  {c}")));
        lines.extend(receipt.files.iter().map(|f| format!("file  {f}")));
    }
    for line in &lines {
        text.push('\n');
        if !line.is_empty() {
            text.push_str("    ");
            text.push_str(line);
        }
    }
    text
}

/// The issue's line, then the note the review step added.
fn handoff_text(handoff: &Handoff) -> String {
    format!(
        "{}\n{}",
        issue_line(&handoff.issue),
        note_text(&handoff.note)
    )
}

/// Notes, a blank line apart.
fn notes_text(notes: &[Note]) -> String {
    notes.iter().map(note_text).collect::<Vec<_>>().join("\n\n")
}

/// `where`'s JSON: the mapping's fields and the decisions that apply.
#[derive(Serialize)]
struct WhereResult<'a> {
    #[serde(flatten)]
    mapping: &'a PathMapping,
    decisions: &'a [Decision],
}

/// Id, owner, age, author, state and review, then the text, indented.
fn decision_text(decision: &Decision) -> String {
    let mut text = format!(
        "{:<4} {:<20} {:<12} {}",
        decision.id.0,
        decision.owner.to_string(),
        format_age(decision.created_at),
        decision.author
    );
    match (decision.state, decision.replaced_by) {
        (DecisionState::Superseded, Some(by)) => text.push_str(&format!("  superseded by {by}")),
        (DecisionState::InForce, _) => {}
        (state, _) => text.push_str(&format!("  {}", state.key())),
    }
    if let Some(old) = decision.replaces {
        text.push_str(&format!("  replaces {old}"));
    }
    if !decision.reviewed {
        text.push_str("  unreviewed");
    }
    for line in decision.text.lines() {
        text.push('\n');
        if !line.is_empty() {
            text.push_str("    ");
            text.push_str(line);
        }
    }
    text
}

/// Decisions, a blank line apart.
fn decisions_text(decisions: &[Decision]) -> String {
    decisions
        .iter()
        .map(decision_text)
        .collect::<Vec<_>>()
        .join("\n\n")
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
        backup.reason_label(),
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
    line.push_str(&paths_suffix(&project.paths));
    line
}

/// `  ~/code/reverb, /srv/x (missing)`, or nothing without paths.
fn paths_suffix(paths: &[MappedPath]) -> String {
    if paths.is_empty() {
        return String::new();
    }
    let paths: Vec<String> = paths
        .iter()
        .map(|p| {
            if p.exists {
                p.path.clone()
            } else {
                format!("{} (missing)", p.path)
            }
        })
        .collect();
    format!("  {}", paths.join(", "))
}

/// `label acme  ~/code/acme`.
fn mapping_line(mapping: &PathMapping) -> String {
    let missing = if mapping.exists { "" } else { " (missing)" };
    format!(
        "{} {}  {}{missing}",
        mapping.kind.key(),
        mapping.name,
        mapping.path
    )
}

/// Full path and issue count.
fn label_line(label: &Label) -> String {
    let line = match label.color {
        Some(color) => format!("{} ({}) {color}", label.name, label.issue_count),
        None => format!("{} ({})", label.name, label.issue_count),
    };
    line + &paths_suffix(&label.paths)
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
                "{}{leaf} ({}){color}{}",
                "  ".repeat(l.depth()),
                l.issue_count,
                paths_suffix(&l.paths)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
