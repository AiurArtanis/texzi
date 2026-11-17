//! Memory protocol only: complete simulates a caller's verified disk receipt.
use texzi_document::*;
fn attention(store: &mut Store, doc: DocId) -> FileAttention {
    store
        .observe_and_subscribe(doc, ObserverScope::new(vec![]), 8)
        .unwrap()
        .0
        .state
        .global
        .metadata
        .attention
        .clone()
}
#[test]
fn verified_binding_epoch_changes_without_clearing_newer_edit_and_save_as_is_atomic() {
    let mut store = Store::new(HistoryBudget::default());
    let s = store.open_utf8("A", Profile::default(), false).unwrap();
    let other = store.open_utf8("other", Profile::default(), false).unwrap();
    let first = TargetKey("memory-first".into());
    let second = TargetKey("memory-save-as".into());
    let c = store
        .begin_save(s.doc, first.clone(), String::new())
        .unwrap();
    assert_eq!(c.binding_epoch, 0);
    let edited = store
        .submit(Transaction {
            doc: s.doc,
            base: s.revision,
            edits: vec![Edit::input(&s, 1..1, "B")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    super::complete(&mut store, &c); // Not actual IO.
    let a = attention(&mut store, s.doc);
    assert_eq!(a.binding_epoch, 1);
    assert_eq!(a.target, Some(first.clone()));
    assert!(store.dirty(s.doc).unwrap());
    assert_eq!(store.snapshot(s.doc).unwrap().token, edited.token);
    assert!(matches!(
        store.begin_save(other.doc, first.clone(), String::new()),
        Err(Error::Busy)
    ));
    let next = store
        .begin_save(s.doc, second.clone(), String::new())
        .unwrap();
    assert_eq!(next.binding_epoch, 1);
    let mut stale = next.clone();
    stale.binding_epoch = 0;
    assert_eq!(store.cancel_save(&stale), Err(Error::InvalidSave));
    assert_eq!(attention(&mut store, s.doc).target, Some(first.clone()));
    super::complete(&mut store, &next);
    assert_eq!(attention(&mut store, s.doc).binding_epoch, 2);
    assert_eq!(attention(&mut store, s.doc).target, Some(second.clone()));
    assert!(!store.dirty(s.doc).unwrap());
    let available = store.begin_save(other.doc, first, String::new()).unwrap();
    store.cancel_save(&available).unwrap();
    assert!(matches!(
        store.begin_save(other.doc, second.clone(), String::new()),
        Err(Error::Busy)
    ));
    assert_eq!(store.accept_verified_save(&c), Err(Error::InvalidSave));
    store.close_document(s.doc).unwrap();
    let released = store.begin_save(other.doc, second, String::new()).unwrap();
    store.cancel_save(&released).unwrap();
}
#[test]
fn cancelled_and_failed_save_as_keep_old_binding_epoch_baseline_and_ownership() {
    let mut store = Store::new(HistoryBudget::default());
    let s = store.open_utf8("A", Profile::default(), false).unwrap();
    let other = store.open_utf8("other", Profile::default(), false).unwrap();
    let original = TargetKey("memory-original".into());
    let c = store
        .begin_save(s.doc, original.clone(), String::new())
        .unwrap();
    super::complete(&mut store, &c);
    let target = TargetKey("memory-save-as-fails".into());
    let cancelled = store
        .begin_save(s.doc, target.clone(), String::new())
        .unwrap();
    store.cancel_save(&cancelled).unwrap();
    assert_eq!(attention(&mut store, s.doc).binding_epoch, 1);
    let failed = store
        .begin_save(s.doc, target.clone(), String::new())
        .unwrap();
    let edited = store
        .submit(Transaction {
            doc: s.doc,
            base: s.revision,
            edits: vec![Edit::input(&s, 1..1, "B")],
            boundary: BoundaryPolicy::Grapheme,
            cancelled: false,
        })
        .unwrap()
        .snapshot;
    store
        .finish_save_failure(
            &failed,
            SaveFailure {
                outcome: SaveFailureOutcome::BeforeReplace,
                conflict: false,
                os_error: Some(112),
                message: "memory-only disk-full receipt".into(),
                recovery_paths: vec![],
            },
        )
        .unwrap();
    let a = attention(&mut store, s.doc);
    assert_eq!(a.binding_epoch, 1);
    assert_eq!(a.target, Some(original.clone()));
    assert_eq!(store.snapshot(s.doc).unwrap().token, edited.token);
    assert!(store.dirty(s.doc).unwrap());
    assert!(matches!(
        store.begin_save(other.doc, original, String::new()),
        Err(Error::Busy)
    ));
    let released = store.begin_save(other.doc, target, String::new()).unwrap();
    store.cancel_save(&released).unwrap();
    let undo = store.undo(s.doc).unwrap().snapshot;
    assert_eq!(undo.token, s.token);
    assert!(!store.dirty(s.doc).unwrap());
    assert_eq!(attention(&mut store, s.doc).binding_epoch, 1);
}
