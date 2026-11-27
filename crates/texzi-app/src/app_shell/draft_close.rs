use super::*;
use nsh_platform_windows::RecoveredDraftChoice;

struct RecoveredClose {
    draft: Rc<Document>,
    active: Option<Rc<Document>>,
    second_active: bool,
    focus_epoch: u64,
    original: Option<PathBuf>,
    revision: Option<u64>,
}

impl RecoveredClose {
    fn same_draft(&self, shell: &Shell) -> bool {
        shell
            .docs
            .with(|docs| docs.iter().any(|doc| Rc::ptr_eq(doc, &self.draft)))
            && self.draft.path.get() == self.original
            && !self.draft.gate.busy()
            && self.draft.needs_close_confirmation()
    }

    fn can_close_written_revision(&self, shell: &Shell, revision: u64) -> bool {
        let same_group = shell.second_active.get() == self.second_active
            && match (&self.active, shell.current()) {
                (Some(before), Some(now)) => Rc::ptr_eq(before, &now) && !now.gate.busy(),
                (None, None) => true,
                _ => false,
            };
        same_group
            && shell.draft_close_focus_epoch.get() == self.focus_epoch
            && self.same_draft(shell)
            && self.draft.source.dirty_revision() == Some(revision)
    }

    fn dialog(&self) -> nsh_base::AlertDialogOptions {
        let title = format!("关闭未保存标签「{}」？", self.draft.name());
        if self.revision.is_none() {
            return nsh_base::alert_dialog()
                .title(title)
                .description(
                    "此恢复草稿尚不能编辑。可继续查看预览，或放弃草稿。完整正文可编辑后可另存。",
                )
                .cancel("continue", "继续编辑")
                .destructive("discard", "放弃并关闭")
                .close_on_escape(true);
        }
        nsh_base::alert_dialog()
            .title(title)
            .description(
                "此标签来自上次会话的恢复记录。你可以继续编辑、另存为文件，或放弃这份草稿。",
            )
            .action("save-as", "另存为…")
            .destructive("discard", "放弃并关闭")
            // Windows focuses the last regular action. Enter must continue editing.
            .action("continue", "继续编辑")
            .close_on_escape(true)
    }

    #[cfg(not(test))]
    fn dialog_details(&self) -> nsh_platform_windows::RecoveredDraftDetails {
        let editable = self.revision.is_some();
        nsh_platform_windows::RecoveredDraftDetails {
            badge: Some(if editable {
                "恢复草稿 · 尚未另存为文件".into()
            } else {
                "恢复草稿 · 只读预览".into()
            }),
            summary: editable.then(|| draft_summary(&self.draft)),
        }
    }
}

#[cfg(not(test))]
fn draft_summary(draft: &Document) -> String {
    let Some(body) = draft.source.current_body() else {
        return String::new();
    };
    if body.len_bytes() == 0 {
        return "空草稿".into();
    }
    let mut first_line = body.source().chars().take_while(|&ch| ch != '\n');
    let mut excerpt: String = first_line
        .by_ref()
        .take(80)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    if first_line.next().is_some() {
        excerpt.push('…');
    }
    format!("1 │ {excerpt}    {} 行", body.logical_line_count())
}

impl Shell {
    pub(super) fn watch_draft_close_focus(self: &Rc<Self>) {
        let shell = self.clone();
        effect(move || {
            let _ = (
                shell.active.get(),
                shell.second_active.get(),
                shell.settings_active(),
            );
            let epoch = &shell.draft_close_focus_epoch;
            epoch.set(epoch.get().wrapping_add(1));
        });
    }

    pub(super) fn confirm_recovered_close(self: &Rc<Self>, draft: Rc<Document>) {
        let active = self.current();
        if active.as_ref().is_some_and(|doc| doc.gate.busy()) {
            self.notice.set("请先完成或取消中文组合".into());
            return;
        }
        let pending = RecoveredClose {
            active,
            second_active: self.second_active.get(),
            focus_epoch: self.draft_close_focus_epoch.get(),
            original: draft.path.get(),
            revision: draft
                .editable_body_ready()
                .then(|| draft.source.dirty_revision())
                .flatten(),
            draft,
        };
        let shell = self.clone();
        self.modal.set(true);
        self.owner.run(|| {
            spawn(async move {
                let result = shell.recovered_close(&pending).await;
                shell.modal.set(false);
                if let Err(error) = result {
                    shell.explain("无法关闭恢复草稿", &error);
                } else if !shell.current().is_some_and(|doc| doc.gate.busy()) {
                    shell.restore_editor_focus();
                }
            });
        });
    }

    async fn recovered_close(self: &Rc<Self>, pending: &RecoveredClose) -> Result<(), String> {
        let mut resume_save_as = false;
        loop {
            match self.recovered_close_choice(pending, resume_save_as).await? {
                Some(RecoveredDraftChoice::Discard)
                    if pending.same_draft(self)
                        && pending.revision.is_none_or(|revision| {
                            pending.draft.source.dirty_revision() == Some(revision)
                        }) =>
                {
                    session_recovery::remove_closed_tab(self, pending.draft.id)
                        .map_err(|error| format!("无法更新恢复记录：{error}"))?;
                    self.remove(pending.draft.id);
                    return Ok(());
                }
                Some(RecoveredDraftChoice::SaveAs) => {
                    let Some(revision) = pending.revision else {
                        return Ok(());
                    };
                    let Some((path, written_revision)) =
                        self.write_recovered_copy(pending, revision).await?
                    else {
                        resume_save_as = true;
                        continue;
                    };
                    self.remember(&path);
                    let location = display_path(&path);
                    if !pending.can_close_written_revision(self, written_revision) {
                        self.notice.set(format!(
                            "已保存所选版本到 {location}，当前草稿或活动编辑组已变化，草稿保持打开"
                        ));
                        return Ok(());
                    }
                    session_recovery::remove_closed_tab(self, pending.draft.id).map_err(
                        |error| {
                            format!(
                                "副本已保存到 {location}。无法更新恢复记录，草稿保持打开：{error}"
                            )
                        },
                    )?;
                    self.remove_with_reopen(pending.draft.id, Some(ClosedTab::File(path)));
                    self.notice.set(format!("已另存为 {location}，已关闭草稿"));
                    return Ok(());
                }
                _ => return Ok(()),
            }
        }
    }

    async fn recovered_close_choice(
        &self,
        pending: &RecoveredClose,
        resume_save_as: bool,
    ) -> Result<Option<RecoveredDraftChoice>, String> {
        #[cfg(not(test))]
        {
            self.owner
                .run(|| {
                    nsh_platform_windows::show_recovered_draft_dialog_with_details(
                        &self.draft_dialog_host,
                        pending.dialog(),
                        resume_save_as,
                        pending.dialog_details(),
                    )
                })
                .await
        }
        // Core fixtures verify the same actions and persistence without a native AppWindow.
        #[cfg(test)]
        {
            let _ = resume_save_as;
            let answer = self.owner.run(|| pending.dialog().show()).await?;
            Ok(match answer.action.as_deref() {
                Some("continue") => Some(RecoveredDraftChoice::Continue),
                Some("discard") => Some(RecoveredDraftChoice::Discard),
                Some("save-as") => Some(RecoveredDraftChoice::SaveAs),
                _ => None,
            })
        }
    }

    async fn write_recovered_copy(
        self: &Rc<Self>,
        pending: &RecoveredClose,
        revision: u64,
    ) -> Result<Option<(PathBuf, u64)>, String> {
        const CHANGED: &str = "草稿内容或活动编辑组已变化。请重新发起另存为。";
        while pending.can_close_written_revision(self, revision) {
            let picker = save_path_dialog(pending.original.as_deref(), true);
            #[cfg(not(test))]
            let chosen = self
                .owner
                .run(|| {
                    nsh_platform_windows::pick_recovered_draft_save_path(
                        &self.draft_dialog_host,
                        picker,
                    )
                })
                .await?;
            #[cfg(test)]
            let chosen = self.owner.run(|| pick_save_path(picker)).await?;
            let Some(path) = chosen else {
                self.notice.set("已取消另存，草稿保持打开".into());
                return Ok(None);
            };
            let mut use_utf8 = false;
            let write_error = loop {
                if !pending.can_close_written_revision(self, revision) {
                    return Err(CHANGED.into());
                }
                if pending.original.as_ref().is_some_and(|original| {
                    display_path(original).eq_ignore_ascii_case(&display_path(&path))
                }) {
                    break "请选择原文件以外的新文件名。".into();
                }
                self.notice
                    .set(format!("正在另存 {}…", pending.draft.name()));
                match pending
                    .draft
                    .source
                    .write_draft_copy(path.clone(), revision, use_utf8)
                    .await
                {
                    Ok(Some(written)) => return Ok(Some((path, written))),
                    Ok(None) => return Err(CHANGED.into()),
                    Err(SaveError::Encoding) => {
                        if !self.confirm_save_as_utf8().await? {
                            self.notice.set("已取消另存，草稿保持打开".into());
                            return Ok(None);
                        }
                        use_utf8 = true;
                    }
                    Err(SaveError::Other(error)) => break error,
                }
            };
            self.notice.set("另存失败，草稿保持打开".into());
            let answer = self
                .owner
                .run(|| {
                    nsh_base::alert_dialog()
                        .title("另存失败")
                        .description(format!(
                            "{write_error}\n请选择尚不存在的文件名。草稿仍可继续编辑。"
                        ))
                        .action("choose", "另选位置")
                        .cancel("back", "返回确认")
                        .close_on_escape(true)
                        .show()
                })
                .await?;
            if answer.action.as_deref() != Some("choose") {
                return Ok(None);
            }
        }
        Err(CHANGED.into())
    }
}
