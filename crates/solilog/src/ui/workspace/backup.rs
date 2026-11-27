//! Backup commands, and automatic backups while the app runs (see
//! `solilog_core`'s `backup.rs` for what a backup is).

use std::path::PathBuf;
use std::time::Duration;

use gpui::{Action, Context, PathPromptOptions, Task, Window};
use solilog_core::{Backup, BackupReason, Db, format_size, format_utc};

use super::Workspace;
use crate::ui::actions::backup::{
    ChooseDirectory, Create, OpenDirectory, Restore, UseDefaultDirectory,
};
use crate::ui::picker::PickerItem;

/// How often to check whether an automatic backup is due (the first check
/// is shortly after start). See `Db::auto_backup`.
const AUTO_BACKUP_CHECK: Duration = Duration::from_secs(10 * 60);
const FIRST_AUTO_BACKUP_CHECK: Duration = Duration::from_secs(10);

impl Workspace {
    /// Makes automatic backups, in the background, for as long as the
    /// workspace lives.
    pub(super) fn auto_backups(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut delay = FIRST_AUTO_BACKUP_CHECK;
            loop {
                cx.background_executor().timer(delay).await;
                delay = AUTO_BACKUP_CHECK;
                let Ok(Some(path)) = this.update(cx, |this, _| this.db.path()) else {
                    break;
                };
                let made = cx
                    .background_executor()
                    .spawn(async move { Db::open(&path)?.auto_backup() })
                    .await;
                let alive = this.update(cx, |this, cx| {
                    if let Err(err) = made {
                        this.report::<()>(Err(err.context("automatic backup failed")), cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
    }

    pub(super) fn on_create_backup(&mut self, _: &Create, _: &mut Window, cx: &mut Context<Self>) {
        self.save_detail(false, cx);
        let path = anyhow::Context::context(self.db.path(), "the database is in memory");
        let Some(path) = self.report(path, cx) else {
            return;
        };
        self.footer.notice = Some("Backing up…".into());
        cx.notify();
        // In the background: attachments can take a while.
        let made = cx
            .background_executor()
            .spawn(async move { Db::open(&path)?.create_backup(BackupReason::Manual) });
        cx.spawn(async move |this, cx| {
            let made = made.await;
            this.update(cx, |this, cx| {
                if let Some(backup) = this.report(made, cx) {
                    this.footer.notice = Some(format!(
                        "Backed up: {} ({})",
                        backup.name,
                        format_size(backup.size)
                    ));
                } else {
                    this.footer.notice = None;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn on_open_backup_directory(
        &mut self,
        _: &OpenDirectory,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dir) = self.report(self.db.backup_dir(), cx) else {
            return;
        };
        if dir.path.is_dir() {
            cx.open_with_system(&dir.path);
        } else {
            self.footer.error = Some(format!(
                "No backups yet ({} doesn't exist)",
                dir.path.display()
            ));
            cx.notify();
        }
    }

    pub(super) fn on_choose_backup_directory(
        &mut self,
        action: &ChooseDirectory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = &action.path {
            return self.set_backup_dir(Some(path.clone()), cx);
        }
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Use for Backups".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let dir = match chosen.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Err(err)) => {
                    this.update(cx, |this, cx| this.report::<()>(Err(err), cx))
                        .ok();
                    return;
                }
                _ => None,
            };
            if let Some(dir) = dir {
                this.update(cx, |this, cx| this.set_backup_dir(Some(dir), cx))
                    .ok();
            }
        })
        .detach();
    }

    pub(super) fn on_use_default_backup_directory(
        &mut self,
        _: &UseDefaultDirectory,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_backup_dir(None, cx);
    }

    fn set_backup_dir(&mut self, dir: Option<PathBuf>, cx: &mut Context<Self>) {
        let Some(change) = self.report(self.db.set_backup_dir(dir.as_deref()), cx) else {
            return;
        };
        let mut notice = format!("Backups go to {}", change.dir.path.display());
        if change.moved > 0 {
            notice.push_str(&format!(" (moved {})", change.moved));
        }
        if change.not_moved > 0 {
            notice.push_str(&format!(
                "; {} left in the old folder: names already taken",
                change.not_moved
            ));
        }
        self.footer.notice = Some(notice);
        cx.notify();
    }

    pub(super) fn on_restore_backup(
        &mut self,
        action: &Restore,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = &action.name else {
            return self.pick_backup(window, cx);
        };
        let Some(backup) = self.report(self.db.find_backup(name), cx) else {
            return;
        };
        let date = format_utc(backup.created_at);
        if !action.confirmed {
            let item = PickerItem::new(format!("Restore the backup from {date}"))
                .hint(Some("the current data is backed up first"));
            let restore = Restore {
                name: Some(backup.name),
                confirmed: true,
            };
            return self.open_action_picker(
                "Replace all data with this backup?",
                vec![(item, Box::new(restore))],
                0,
                window,
                cx,
            );
        }
        // Edits in progress are saved, so the backup made first has them.
        self.close_detail(cx);
        let restored = self.db.restore_backup(&backup.path.to_string_lossy());
        if let Some(restored) = self.report(restored, cx) {
            self.reload(cx);
            self.footer.notice = Some(format!(
                "Restored the backup from {date}. The data as it was is in {}",
                restored.before_restore.name
            ));
            cx.notify();
        }
    }

    fn pick_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(backups) = self.report(self.db.list_backups(), cx) else {
            return;
        };
        if backups.is_empty() {
            self.footer.error = Some("No backups yet".into());
            cx.notify();
            return;
        }
        let entries = backups
            .iter()
            .map(|backup| {
                let item = PickerItem::new(format_utc(backup.created_at))
                    .hint(Some(backup_summary(backup)));
                let restore = Restore {
                    name: Some(backup.name.clone()),
                    confirmed: false,
                };
                (item, Box::new(restore) as Box<dyn Action>)
            })
            .collect();
        self.open_action_picker("Restore backup…", entries, 0, window, cx);
    }
}

/// `before restore · 12 issues · 1.2 MB`.
fn backup_summary(backup: &Backup) -> String {
    let mut parts = Vec::new();
    if !backup.own {
        parts.push("another database".to_owned());
    }
    if backup.reason != BackupReason::Auto {
        parts.push(backup.reason_label());
    }
    let plural = if backup.issue_count == 1 { "" } else { "s" };
    parts.push(format!("{} issue{plural}", backup.issue_count));
    parts.push(format_size(backup.size));
    parts.join(" · ")
}
