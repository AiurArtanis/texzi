//! Detached copy writes from the recovered-tab close confirmation.
use super::{
    tests::{record, recovery_tab},
    *,
};
use nsh_core::{
    ApproxTextEngine, Command, Core, EventKind, EventPayload, MountItem, Op, PayloadValue, ROOT_ID,
    Size,
    element::{Environment, Root, RustCall, RustOp, ToJs},
    platform::Os,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

thread_local! {
    static CLOSE_FIXTURE: RefCell<Option<CloseFixture>> = const { RefCell::new(None) };
}

const MIXED_DRAFT: &str = "旧稿\r\nsecond\nlast\r";

struct CloseFixture {
    disk: Arc<Disk>,
    shell: Rc<RefCell<Option<Rc<Shell>>>>,
}

#[component]
fn DraftCloseFixture() -> impl IntoElement {
    let CloseFixture { disk, shell } = CLOSE_FIXTURE.with(|slot| slot.take().unwrap());
    let restored = Shell::new();
    restore(&restored, disk.read().unwrap());
    let session = Rc::new(Session::new(disk));
    session.sequence.set(1);
    *restored.session.borrow_mut() = Some(session);
    *shell.borrow_mut() = Some(restored);
    text("")
}

struct DraftCloseHarness {
    core: Core<ApproxTextEngine>,
    _runtime: tokio::runtime::Runtime,
    shell: Rc<Shell>,
    disk: Arc<Disk>,
    directory: PathBuf,
}

impl DraftCloseHarness {
    fn new(named: bool) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("texzi-draft-close-{}-{unique}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let source = directory.join("source.txt");
        fs::write(&source, b"original").unwrap();
        let mut target = recovery_tab(&source, b"original");
        if !named {
            target.path = None;
            target.draft.as_mut().unwrap().disk_sha256 = None;
        }
        let draft = target.draft.as_mut().unwrap();
        draft.text = MIXED_DRAFT.into();
        draft.format = FilePreference::capture(
            Format {
                encoding: Encoding::Utf16Le,
                bom: true,
            },
            Eol::CrLf,
        );
        let mut neighbor = recovery_tab(&source, b"original");
        neighbor.id = 5;
        neighbor.path = None;
        neighbor.draft.as_mut().unwrap().disk_sha256 = None;
        let mut seed = record(target);
        seed.active = neighbor.id;
        seed.tabs.push(neighbor);
        let disk = Disk::claim(&directory, "413").unwrap().unwrap();
        disk.update(1, || Ok(Some(seed))).unwrap();
        let shell = Rc::new(RefCell::new(None));
        CLOSE_FIXTURE.with(|slot| {
            slot.replace(Some(CloseFixture {
                disk: disk.clone(),
                shell: shell.clone(),
            }))
        });
        let mut core = Core::new(
            ApproxTextEngine,
            Size {
                width: px(960.0),
                height: px(640.0),
            },
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        core.set_task_runtime(runtime.handle().clone());
        let mut env = Environment::default();
        env.os = Os::Windows;
        env.file_requests = true;
        core.set_environment(env);
        core.mount_root(&Root::new::<DraftCloseFixture>(|| ()));
        core.run_component_tasks();
        let _ = core.commit();
        let shell = shell.take().unwrap();
        Self {
            core,
            _runtime: runtime,
            shell,
            disk,
            directory,
        }
    }

    fn in_shell<R: 'static>(&mut self, work: impl FnOnce(&Rc<Shell>) -> R + 'static) -> R {
        let shell = self.shell.clone();
        let work = RefCell::new(Some(work));
        let result = Rc::new(RefCell::new(None));
        let output = result.clone();
        self.core
            .apply(Op::Rust(RustOp {
                call: 1,
                run: RustCall::Op(Box::new(move |_| {
                    *output.borrow_mut() = Some(work.borrow_mut().take().unwrap()(&shell));
                    Ok(Box::new(()) as Box<dyn ToJs>)
                })),
            }))
            .unwrap();
        result.take().unwrap()
    }

    fn wait_for<R>(&mut self, expected: &str, mut find: impl FnMut(Command) -> Option<R>) -> R {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            self.core.run_component_tasks();
            if let Some(batch) = self.core.commit() {
                for item in batch.items {
                    if let MountItem::Command { command, .. } = item
                        && let Some(found) = find(command)
                    {
                        return found;
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "missing draft close command: {expected}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn dialog(&mut self, prefix: &str) -> (u32, Vec<String>) {
        self.wait_for(prefix, |command| {
            let Command::Dialog(dialog) = command else {
                return None;
            };
            assert!(
                dialog
                    .title
                    .as_deref()
                    .unwrap_or_default()
                    .starts_with(prefix)
            );
            Some((
                dialog.id,
                dialog
                    .buttons
                    .iter()
                    .map(|button| button.text.clone())
                    .collect(),
            ))
        })
    }

    fn choose(&mut self, (id, buttons): (u32, Vec<String>), label: &str) {
        let index = buttons.iter().position(|name| name == label).expect(label);
        assert!(self.core.component_event(
            ROOT_ID,
            EventKind::Dialog,
            &EventPayload::Map(vec![
                ("id", PayloadValue::Number(id as f64)),
                ("index", PayloadValue::Number(index as f64)),
            ])
        ));
    }

    fn pick(&mut self, path: Option<PathBuf>) {
        let id = self.save_picker();
        self.answer_picker(id, path);
    }

    fn save_picker(&mut self) -> u32 {
        self.wait_for("SaveDialog", |command| match command {
            Command::FileRequest { id, request }
                if matches!(*request, nsh_core::file_dialog::FileRequest::SaveDialog(_)) =>
            {
                Some(id)
            }
            _ => None,
        })
    }

    fn answer_picker(&mut self, id: u32, path: Option<PathBuf>) {
        assert!(self.core.component_event(
            ROOT_ID,
            EventKind::FileRequest,
            &EventPayload::file_answer(id, path.into_iter().collect())
        ));
    }

    fn wait_for_notice(&mut self, prefix: &str) {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            self.core.run_component_tasks();
            let _ = self.core.commit();
            let notice = self.in_shell(|shell| shell.notice.get());
            if notice.starts_with(prefix) {
                return;
            }
            assert!(Instant::now() < deadline, "expected {prefix}; got {notice}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn finish(self) {
        let directory = self.directory.clone();
        drop(self);
        for file in fs::read_dir(&directory).unwrap() {
            fs::remove_file(file.unwrap().path()).unwrap();
        }
        fs::remove_dir(directory).unwrap();
    }
}

#[test]
fn draft_close_uses_the_recovered_tab_and_returns_to_confirmation_on_cancel() {
    let mut test = DraftCloseHarness::new(true);
    test.in_shell(|shell| {
        let active = shell.current().unwrap();
        active.source.set_composing(true);
        shell.close_tab(4);
        assert!(!shell.modal.get());
        assert_eq!(shell.docs.with(Vec::len), 2);
        assert_eq!(shell.notice.get(), "请先完成或取消中文组合");
        active.source.set_composing(false);
    });
    test.in_shell(|shell| shell.close_tab(4));
    let confirm = test.dialog("关闭未保存标签");
    test.choose(confirm, "另存为…");
    test.pick(None);
    let confirm = test.dialog("关闭未保存标签");
    let original = test.directory.join("source.txt");
    let occupied = test.directory.join("occupied.txt");
    fs::write(&occupied, b"outside").unwrap();
    test.choose(confirm, "另存为…");
    test.pick(Some(occupied.clone()));
    let failed = test.dialog("另存失败");
    test.choose(failed, "返回确认");
    let confirm = test.dialog("关闭未保存标签");
    assert_eq!(test.disk.read().unwrap().tabs.len(), 2);
    let chosen = test.directory.join("chosen.txt");
    test.choose(confirm, "另存为…");
    test.pick(Some(chosen.clone()));
    test.wait_for_notice("已另存为");
    let reopen = chosen.clone();
    test.in_shell(move |shell| {
        assert!(!shell.modal.get());
        assert_eq!(shell.docs.with(Vec::len), 1);
        assert_eq!(shell.current().unwrap().id, 5);
        assert!(matches!(shell.closed.borrow().last(), Some(super::super::ClosedTab::File(path)) if path == &reopen));
    });
    assert_eq!(test.disk.read().unwrap().tabs[0].id, 5);
    assert_eq!(fs::read(original).unwrap(), b"original");
    assert_eq!(fs::read(occupied).unwrap(), b"outside");
    assert_eq!(fs::read(chosen).unwrap(), encode_draft());
    test.finish();
}

#[test]
fn draft_close_retains_the_unnamed_draft_if_its_copy_wrote_but_recovery_record_cannot() {
    let mut test = DraftCloseHarness::new(false);
    let obstruction = test.disk.path.with_extension("tmp");
    fs::create_dir(&obstruction).unwrap();
    test.in_shell(|shell| shell.close_tab(4));
    let confirm = test.dialog("关闭未保存标签");
    let chosen = test.directory.join("chosen.txt");
    test.choose(confirm, "另存为…");
    test.pick(Some(chosen.clone()));
    let failed = test.dialog("无法关闭恢复草稿");
    assert_eq!(fs::read(chosen).unwrap(), encode_draft());
    assert_eq!(test.disk.read().unwrap().tabs.len(), 2);
    test.in_shell(|shell| {
        let origin = shell.docs.with(|docs| docs[0].clone());
        assert!(origin.path.get().is_none());
        assert!(origin.source.is_dirty().unwrap());
        assert!(origin.source.is_recovered_draft());
        assert_eq!(shell.current().unwrap().id, 5);
        assert!(shell.notice.get().contains("副本已保存"));
        assert!(shell.closed.borrow().is_empty());
    });
    test.choose(failed, "知道了");
    fs::remove_dir(obstruction).unwrap();
    test.core.run_component_tasks();
    test.in_shell(|shell| shell.close_tab(4));
    let confirmation = test.dialog("关闭未保存标签");
    let retry = test.directory.join("retry.txt");
    test.choose(confirmation, "另存为…");
    test.pick(Some(retry.clone()));
    test.wait_for_notice("已另存为");
    assert_eq!(fs::read(retry).unwrap(), encode_draft());
    assert_eq!(test.disk.read().unwrap().tabs[0].id, 5);
    test.finish();
}

#[test]
fn draft_close_keeps_tab_and_recovery_after_an_edit_and_undo_during_copy() {
    let mut test = DraftCloseHarness::new(true);
    let (started, ready) = std::sync::mpsc::channel();
    let (resume, suspended) = std::sync::mpsc::channel();
    test.in_shell(move |shell| {
        let draft = shell.docs.with(|docs| docs[0].clone());
        draft.source.before_next_publication(Box::new(move |_| {
            started.send(()).unwrap();
            suspended.recv_timeout(Duration::from_secs(12)).unwrap();
        }));
        shell.close_tab(draft.id);
    });
    let chosen = test.directory.join("chosen.txt");
    let confirm = test.dialog("关闭未保存标签");
    test.choose(confirm, "另存为…");
    test.pick(Some(chosen.clone()));
    let deadline = Instant::now() + Duration::from_secs(12);
    while ready.try_recv().is_err() {
        test.core.run_component_tasks();
        let _ = test.core.commit();
        assert!(Instant::now() < deadline, "detached write did not start");
        std::thread::sleep(Duration::from_millis(5));
    }
    test.in_shell(|shell| {
        let draft = shell.docs.with(|docs| docs[0].clone());
        let snapshot = draft.source.snapshot().unwrap();
        draft
            .source
            .edit(
                snapshot.revision,
                snapshot.view_epoch,
                vec![(0..0, "X".into())],
                ExternalSelections {
                    ranges: vec![(1, 1)],
                    primary: 0,
                },
                nsh_base::editor::ExternalEditKind::Typing,
            )
            .unwrap();
        draft.source.undo_redo(true).unwrap();
        assert_ne!(draft.source.snapshot().unwrap().revision, snapshot.revision);
    });
    resume.send(()).unwrap();
    test.wait_for_notice("已保存所选版本");
    test.in_shell(|shell| {
        let draft = shell.docs.with(|docs| docs[0].clone());
        assert!(!shell.modal.get());
        assert_eq!(shell.current().unwrap().id, 5);
        assert!(draft.source.is_dirty().unwrap());
        assert!(draft.source.is_recovered_draft());
        assert_eq!(
            draft.source.current_body().unwrap().to_utf8(),
            MIXED_DRAFT.as_bytes()
        );
    });
    assert_eq!(fs::read(chosen).unwrap(), encode_draft());
    assert_eq!(test.disk.read().unwrap().tabs.len(), 2);
    assert_eq!(
        fs::read(test.directory.join("source.txt")).unwrap(),
        b"original"
    );
    test.finish();
}

#[test]
fn draft_close_does_not_write_a_picker_result_after_the_active_document_changes() {
    let mut test = DraftCloseHarness::new(true);
    test.in_shell(|shell| shell.close_tab(4));
    let confirm = test.dialog("关闭未保存标签");
    test.choose(confirm, "另存为…");
    let picker = test.save_picker();
    let chosen = test.directory.join("chosen.txt");
    let epoch = test.in_shell(|shell| shell.draft_close_focus_epoch.get());
    test.in_shell(|shell| shell.activate_primary(4));
    test.core.run_component_tasks();
    let _ = test.core.commit();
    test.in_shell(|shell| shell.activate_primary(5));
    test.core.run_component_tasks();
    let _ = test.core.commit();
    assert_ne!(
        test.in_shell(|shell| shell.draft_close_focus_epoch.get()),
        epoch
    );
    test.answer_picker(picker, Some(chosen.clone()));
    let changed = test.dialog("无法关闭恢复草稿");
    test.in_shell(|shell| {
        assert_eq!(shell.docs.with(Vec::len), 2);
        assert_eq!(shell.current().unwrap().id, 5);
        assert!(shell.docs.with(|docs| docs[0].source.is_dirty().unwrap()));
    });
    assert!(!chosen.exists());
    assert_eq!(test.disk.read().unwrap().tabs.len(), 2);
    test.choose(changed, "知道了");
    test.finish();
}

fn encode_draft() -> Vec<u8> {
    [0xff, 0xfe]
        .into_iter()
        .chain(MIXED_DRAFT.encode_utf16().flat_map(u16::to_le_bytes))
        .collect()
}
