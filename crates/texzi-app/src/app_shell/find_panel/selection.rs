use super::*;

/// Keep the user's last selection in this pane when search navigation temporarily
/// selects its own hit. Once enabled, the active scope holds its own anchor set.
#[derive(Default)]
pub(super) struct ScopeCapture {
    latest: RefCell<Option<Rc<RefCell<SearchScope>>>>,
    seen: Cell<Option<(bool, u64, u64)>>,
    applying: Cell<bool>,
}

impl ScopeCapture {
    pub(super) fn reset(&self) {
        self.latest.take();
        self.seen.set(None);
    }

    pub(super) fn latest(&self) -> Option<Rc<RefCell<SearchScope>>> {
        self.latest.borrow().clone()
    }

    fn stamp(gate: &ModeGate) -> Result<(bool, u64, u64), String> {
        let (snapshot, epoch) = gate.selection_view().search_snapshot()?;
        Ok((gate.rich_mode.get(), snapshot.revision.0, epoch))
    }

    pub(super) fn observe(&self, gate: &ModeGate) -> Result<bool, String> {
        let stamp = Self::stamp(gate)?;
        if !self.applying.get() && self.seen.get() != Some(stamp) {
            let view = gate.selection_view();
            let ranges = view.snapshot()?.selections.ranges;
            let scope = if ranges.iter().any(|(a, b)| a != b) {
                Some(view.freeze_search_scope(&ranges)?)
            } else {
                None
            };
            self.latest.replace(scope);
            self.seen.set(Some(stamp));
        }
        Ok(self.latest.borrow().is_some())
    }

    pub(super) fn apply<T>(&self, gate: &ModeGate, action: impl FnOnce() -> T) -> T {
        let was_applying = self.applying.replace(true);
        let result = action();
        self.seen.set(Self::stamp(gate).ok());
        self.applying.set(was_applying);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_base::editor::ExternalSelections;

    #[test]
    fn enabling_scope_uses_latest_manual_selection_even_after_find_navigation() {
        let runtime = nsh_core::element::Runtime::default();
        let _guard = runtime.enter();
        let source = NativeView::fixture("aaaa bbbb cccc");
        let gate = ModeGate::new(source.clone(), source.sibling().unwrap());
        let capture = ScopeCapture::default();
        let revision = source.search_snapshot().unwrap().0.revision;
        let manual = |ranges| {
            let epoch = source.search_snapshot().unwrap().1;
            source
                .select(revision.0, epoch, ExternalSelections { primary: 0, ranges })
                .unwrap();
        };

        assert!(!capture.observe(&gate).unwrap());
        manual(vec![(3, 0), (5, 9)]);
        assert!(capture.observe(&gate).unwrap());
        let enabled = capture.latest().unwrap();
        assert_eq!(enabled.borrow().ranges(revision).unwrap(), [0..3, 5..9]);
        capture.apply(&gate, || source.find_select(revision.0, &[1..2]).unwrap());
        assert!(capture.observe(&gate).unwrap());
        assert!(Rc::ptr_eq(&enabled, &capture.latest().unwrap()));

        manual(vec![(13, 10)]);
        assert!(capture.observe(&gate).unwrap());
        let reenabled = capture.latest().unwrap();
        assert_eq!(reenabled.borrow().ranges(revision).unwrap(), [10..13]);
        assert_eq!(enabled.borrow().ranges(revision).unwrap(), [0..3, 5..9]);
        manual(vec![(13, 13)]);
        assert!(!capture.observe(&gate).unwrap());
        assert!(capture.latest().is_none());
    }
}
