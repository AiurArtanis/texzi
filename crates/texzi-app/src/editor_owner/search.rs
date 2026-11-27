//! Frozen search selections follow the same receipts as the visible views.
use super::*;
use std::cell::RefCell;
use std::ops::Range;
use std::rc::{Rc, Weak};

pub(crate) struct SearchScope {
    revision: doc::Revision,
    ranges: Vec<(doc::SourceByte, doc::SourceByte)>,
}

impl SearchScope {
    pub(crate) fn ranges(&self, revision: doc::Revision) -> Result<Vec<Range<usize>>, String> {
        if revision != self.revision {
            return Err("搜索范围已过期，请重新选择文字".into());
        }
        Ok(self
            .ranges
            .iter()
            .map(|(a, b)| a.offset..b.offset)
            .collect())
    }
    fn advance(&mut self, receipt: &doc::Receipt) {
        if self.revision != receipt.previous {
            return;
        }
        let mut mapped = Vec::with_capacity(self.ranges.len());
        for (a, b) in &self.ranges {
            let (Ok(a), Ok(b)) = (
                receipt.map_anchor(*a, doc::Affinity::Before),
                receipt.map_anchor(*b, doc::Affinity::After),
            ) else {
                return;
            };
            if a.offset < b.offset {
                mapped.push((a, b));
            }
        }
        self.ranges = mapped;
        self.revision = receipt.snapshot.revision;
    }
}

pub(super) type ScopeRegistry = RefCell<Vec<Weak<RefCell<SearchScope>>>>;
impl NativeView {
    pub(crate) fn track_selection(&self) {
        self.selection_changed.get();
    }
    pub(crate) fn search_snapshot(&self) -> Result<(doc::Snapshot, u64), String> {
        let store = self.owner.store.borrow();
        let view = store.view(self.view).map_err(err)?;
        Ok((store.snapshot(view.doc).map_err(err)?, view.epoch))
    }
    pub(crate) fn freeze_search_scope(
        &self,
        ranges: &[(usize, usize)],
    ) -> Result<Rc<RefCell<SearchScope>>, String> {
        let (snapshot, _) = self.search_snapshot()?;
        let scope = Rc::new(RefCell::new(SearchScope {
            revision: snapshot.revision,
            ranges: ranges
                .iter()
                .filter(|(a, b)| a != b)
                .map(|&(a, b)| {
                    (
                        doc::SourceByte::at(&snapshot, a.min(b)),
                        doc::SourceByte::at(&snapshot, a.max(b)),
                    )
                })
                .collect(),
        }));
        let mut tracked = self.owner.search_scopes.borrow_mut();
        tracked.retain(|weak| weak.strong_count() != 0);
        tracked.push(Rc::downgrade(&scope));
        Ok(scope)
    }
    pub(super) fn advance_search_scopes(&self, receipt: &doc::Receipt) {
        if !receipt.changed {
            return;
        }
        self.owner.search_scopes.borrow_mut().retain(|weak| {
            if let Some(scope) = weak.upgrade() {
                scope.borrow_mut().advance(receipt);
                true
            } else {
                false
            }
        });
    }
    pub(crate) fn find_select(&self, revision: u64, ranges: &[Range<usize>]) -> Result<(), String> {
        if ranges.is_empty() {
            return Ok(());
        }
        let (snapshot, epoch) = self.search_snapshot()?;
        if snapshot.revision.0 != revision {
            return Err("正文已更改，请重新查找".into());
        }
        let mut selections = Vec::with_capacity(ranges.len());
        for range in ranges {
            let start = snapshot
                .snap_source_offset(range.start, doc::Affinity::Before)
                .map_err(err)?;
            let end = if range.is_empty() {
                start
            } else {
                snapshot
                    .snap_source_offset(range.end, doc::Affinity::After)
                    .map_err(err)?
            };
            selections.push((start, end));
        }
        self.select(
            revision,
            epoch,
            ExternalSelections {
                primary: 0,
                ranges: selections,
            },
        )?;
        self.fence();
        self.notify();
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn submit_find_replace(
        &self,
        epoch: u64,
        prepared: doc::PreparedBulk,
    ) -> Result<bool, String> {
        self.submit_find_replace_at(epoch, prepared, None)
            .map(|(changed, _)| changed)
    }
    pub(crate) fn submit_find_replace_at(
        &self,
        epoch: u64,
        prepared: doc::PreparedBulk,
        marker: Option<doc::SourceByte>,
    ) -> Result<(bool, Option<usize>), String> {
        if self.is_composing() {
            return Err("请先完成或取消中文组合".into());
        }
        let mut store = self.owner.store.borrow_mut();
        let receipt = store.submit_bulk(self.view, epoch, prepared).map_err(err)?;
        self.advance_search_scopes(&receipt);
        let changed = receipt.changed;
        let next = marker
            .and_then(|p| receipt.map_anchor(p, doc::Affinity::After).ok())
            .map(|p| p.offset);
        drop(store);
        if changed {
            self.notify();
        }
        Ok((changed, next))
    }
}
