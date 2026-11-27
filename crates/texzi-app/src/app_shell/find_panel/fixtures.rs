//! Exercise the same frozen snapshot, matches, projection, and publication used by the panel.
use super::*;

fn scan(
    view: &NativeView,
    gate: Option<&ModeGate>,
    query: &str,
    scopes: Option<Vec<Range<usize>>>,
) -> Result<Index, String> {
    let (snapshot, _) = view.search_snapshot()?;
    let source_selection = view.snapshot()?.selections;
    let (root, anchor) = if let Some(gate) = gate {
        let (node, range) = gate.find_rich_root().ok_or("missing rendered document")?;
        (Some(node), range.end)
    } else {
        let (a, b) = source_selection.ranges[source_selection.primary];
        (None, a.max(b))
    };
    Scan {
        snapshot,
        rich: root,
        cached: None,
        pattern: Pattern {
            query: query.into(),
            options: Options {
                case_sensitive: true,
                whole_word: false,
            },
            regex: false,
            scopes,
        },
        anchor,
        previous: false,
        viewport: 0..usize::MAX,
        select_all: false,
    }
    .run(&AtomicBool::new(false), |_, _| {})
}
pub(crate) fn find_source(view: &NativeView, query: &str) -> Result<(u64, u64), String> {
    let index = scan(view, None, query, None)?;
    let Some(hit) = index.results.current else {
        return Ok((0, 0));
    };
    view.find_select(view.search_snapshot()?.0.revision.0, &[hit.range])?;
    Ok((hit.ordinal, index.results.total))
}
pub(crate) fn find_rich(
    gate: &ModeGate,
    view: &NativeView,
    query: &str,
) -> Result<(u64, u64), String> {
    let index = scan(view, Some(gate), query, None)?;
    let Some(hit) = index.results.current else {
        return Ok((0, 0));
    };
    let r = index
        .subject
        .text
        .view(&hit.range)
        .ok_or("missing rich position")?;
    gate.find_rich_select(view.search_snapshot()?.0.revision.0, r)?;
    Ok((hit.ordinal, index.results.total))
}
pub(crate) fn replace_source(view: &NativeView, query: &str, to: &str) -> Result<u64, String> {
    let index = scan(view, None, query, None)?;
    let (snapshot, epoch) = view.search_snapshot()?;
    let (n, plan) = index.replace(snapshot, to, None, Arc::new(AtomicBool::new(false)))?;
    view.submit_find_replace(epoch, plan)?;
    Ok(n)
}

#[test]
fn frozen_selections_move_across_bulk_and_sibling_undo() {
    let runtime = nsh_core::element::Runtime::default();
    let _enter = runtime.enter();
    let view = NativeView::fixture("a\r\na\na");
    let sibling = view.sibling_source().unwrap();
    let scope = view.freeze_search_scope(&[(0, 1), (4, 5)]).unwrap();
    let (snapshot, epoch) = view.search_snapshot().unwrap();
    let index = scan(
        &view,
        None,
        "a",
        Some(scope.borrow().ranges(snapshot.revision).unwrap()),
    )
    .unwrap();
    assert_eq!(index.results.total, 2);
    let (count, plan) = index
        .replace(snapshot, "木木", None, Arc::new(AtomicBool::new(false)))
        .unwrap();
    assert_eq!(count, 2);
    assert!(view.submit_find_replace(epoch, plan).unwrap());
    let (snapshot, _) = sibling.search_snapshot().unwrap();
    assert_eq!(snapshot.body.to_utf8(), "木木\r\na\n木木".as_bytes());
    assert_eq!(
        scope.borrow().ranges(snapshot.revision).unwrap(),
        vec![0..6, 9..15]
    );
    let current = sibling.snapshot().unwrap();
    sibling
        .history(current.revision, current.view_epoch, true)
        .unwrap();
    let (snapshot, _) = view.search_snapshot().unwrap();
    assert_eq!(snapshot.body.to_utf8(), b"a\r\na\na");
    assert_eq!(
        scope.borrow().ranges(snapshot.revision).unwrap(),
        vec![0..1, 4..5]
    );
}

#[test]
fn sibling_edit_rejects_prepared_search_transaction_without_moving_the_scope() {
    use nsh_base::editor::{ExternalEditKind, ExternalSelections};
    let runtime = nsh_core::element::Runtime::default();
    let _enter = runtime.enter();
    let view = NativeView::fixture("old old");
    let other = view.sibling_source().unwrap();
    let (snapshot, epoch) = view.search_snapshot().unwrap();
    let scope = view.freeze_search_scope(&[(4, 7)]).unwrap();
    let index = scan(&view, None, "old", None).unwrap();
    let (_, plan) = index
        .replace(snapshot, "new", None, Arc::new(AtomicBool::new(false)))
        .unwrap();
    let s = other.snapshot().unwrap();
    other
        .edit(
            s.revision,
            s.view_epoch,
            vec![(0..0, "!".into())],
            ExternalSelections {
                primary: 0,
                ranges: vec![(1, 1)],
            },
            ExternalEditKind::Typing,
        )
        .unwrap();
    assert!(view.submit_find_replace(epoch, plan).is_err());
    let (actual, _) = view.search_snapshot().unwrap();
    assert_eq!(actual.body.to_utf8(), b"!old old");
    assert_eq!(scope.borrow().ranges(actual.revision).unwrap(), vec![5..8]);
}
