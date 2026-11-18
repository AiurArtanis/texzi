//! Menu commands use the same versioned Store operations as native input.
use super::*;
use unicode_segmentation::UnicodeSegmentation;

impl NativeView {
    pub fn text(&self) -> Result<String, String> {
        Ok(self.snapshot()?.rope.to_string())
    }
    pub fn is_dirty(&self) -> Result<bool, String> {
        let store = self.owner.store.borrow();
        store
            .dirty(store.view(self.view).map_err(err)?.doc)
            .map_err(err)
    }
    pub fn has_undo_redo(&self) -> Result<(bool, bool), String> {
        Ok(self.can_history())
    }
    fn menu_select(
        &self,
        snapshot: &ExternalSnapshot,
        ranges: Vec<(usize, usize)>,
        primary: usize,
    ) -> Result<(), String> {
        self.select(
            snapshot.revision,
            snapshot.view_epoch,
            ExternalSelections { ranges, primary },
        )?;
        self.fence();
        self.notify();
        Ok(())
    }
    pub fn select_all(&self) -> Result<(), String> {
        let s = self.snapshot()?;
        self.menu_select(&s, vec![(0, s.rope.len_bytes())], 0)
    }
    pub fn clear_extra_selections(&self) -> Result<(), String> {
        let s = self.snapshot()?;
        let selection = s
            .selections
            .ranges
            .get(s.selections.primary)
            .copied()
            .ok_or("没有主选区")?;
        self.menu_select(&s, vec![selection], 0)
    }
    pub fn undo_redo(&self, undo: bool) -> Result<(), String> {
        if self.is_composing() {
            return Err("请先完成或取消中文组合".into());
        }
        let s = self.snapshot()?;
        self.history(s.revision, s.view_epoch, undo)?;
        self.notify();
        Ok(())
    }
    pub fn go_to_line(&self, line: usize) -> Result<(), String> {
        if line == 0 {
            return Err("行号从 1 开始".into());
        }
        let s = self.snapshot()?;
        let raw = s.rope.to_string();
        let at = if line == 1 {
            0
        } else {
            raw.match_indices('\n')
                .nth(line - 2)
                .map(|(at, _)| at + 1)
                .ok_or("行号超出文档")?
        };
        self.menu_select(&s, vec![(at, at)], 0)
    }
    pub fn select_next_match(&self, all: bool) -> Result<(), String> {
        let s = self.snapshot()?;
        let raw = s.rope.to_string();
        let (a, h) = *s
            .selections
            .ranges
            .get(s.selections.primary)
            .ok_or("没有主选区")?;
        let (mut start, mut end) = (a.min(h), a.max(h));
        if start == end {
            let (at, word) = raw
                .unicode_word_indices()
                .find(|(at, word)| *at <= h && h <= *at + word.len())
                .ok_or("光标处没有可选择的词")?;
            start = at;
            end = at + word.len();
            if !all {
                return self.menu_select(&s, vec![(start, end)], 0);
            }
        }
        let query = raw
            .get(start..end)
            .filter(|s| !s.is_empty())
            .ok_or("请先选择文字")?;
        let matches: Vec<_> = raw
            .match_indices(query)
            .map(|(at, _)| (at, at + query.len()))
            .collect();
        if all {
            return self.menu_select(&s, matches, 0);
        }
        let mut ranges = s.selections.ranges.clone();
        let next = matches
            .iter()
            .skip_while(|(_, b)| *b <= end)
            .find(|(a, b)| !ranges.iter().any(|(x, y)| x.min(y) < b && a < x.max(y)))
            .copied();
        let next = next
            .or_else(|| {
                matches
                    .iter()
                    .find(|(a, b)| !ranges.iter().any(|(x, y)| x.min(y) < b && a < x.max(y)))
                    .copied()
            })
            .ok_or("没有更多匹配")?;
        ranges.push(next);
        let primary = ranges.len() - 1;
        self.menu_select(&s, ranges, primary)
    }
    pub fn find_literal(&self, query: &str, previous: bool) -> Result<(usize, usize), String> {
        if query.is_empty() {
            return Err("请输入查找文字".into());
        }
        let s = self.snapshot()?;
        let raw = s.rope.to_string();
        let hits: Vec<_> = raw
            .match_indices(query)
            .map(|(at, _)| (at, at + query.len()))
            .collect();
        if hits.is_empty() {
            return Ok((0, 0));
        }
        let (a, b) = s.selections.ranges[s.selections.primary];
        let index = if previous {
            hits.iter()
                .rposition(|(_, end)| *end <= a.min(b))
                .unwrap_or(hits.len() - 1)
        } else {
            hits.iter()
                .position(|(start, _)| *start >= a.max(b))
                .unwrap_or(0)
        };
        self.menu_select(&s, vec![hits[index]], 0)?;
        Ok((index + 1, hits.len()))
    }
    pub fn replace_literal(
        &self,
        query: &str,
        replacement: &str,
        all: bool,
    ) -> Result<usize, String> {
        if self.is_composing() {
            return Err("请先完成或取消中文组合".into());
        }
        if query.is_empty() {
            return Err("请输入查找文字".into());
        }
        let s = self.snapshot()?;
        let raw = s.rope.to_string();
        let at = s.selections.ranges[s.selections.primary]
            .0
            .min(s.selections.ranges[s.selections.primary].1);
        let hits: Vec<_> = raw
            .match_indices(query)
            .map(|(at, _)| at..at + query.len())
            .collect();
        let edits: Vec<_> = if all {
            hits
        } else {
            hits.iter()
                .find(|r| r.start >= at)
                .or(hits.first())
                .cloned()
                .into_iter()
                .collect()
        };
        let count = edits.len();
        if count == 0 {
            return Ok(0);
        }
        let first = edits[0].start;
        self.edit(
            s.revision,
            s.view_epoch,
            edits
                .into_iter()
                .map(|r| (r, replacement.to_owned()))
                .collect(),
            ExternalSelections {
                primary: 0,
                ranges: vec![(first, first + replacement.len())],
            },
            ExternalEditKind::Other,
        )?;
        self.notify();
        Ok(count)
    }
    pub fn duplicate_line(&self) -> Result<(), String> {
        if self.is_composing() {
            return Err("请先完成或取消中文组合".into());
        }
        let s = self.snapshot()?;
        let raw = s.rope.to_string();
        let at = s.selections.ranges[s.selections.primary].1;
        let start = raw[..at].rfind('\n').map_or(0, |i| i + 1);
        let end = raw[at..].find('\n').map_or(raw.len(), |i| at + i + 1);
        let value = if raw[start..end].ends_with('\n') {
            raw[start..end].to_owned()
        } else {
            format!("\n{}", &raw[start..end])
        };
        let caret = end + value.len();
        self.edit(
            s.revision,
            s.view_epoch,
            vec![(end..end, value)],
            ExternalSelections {
                primary: 0,
                ranges: vec![(caret, caret)],
            },
            ExternalEditKind::Other,
        )?;
        self.notify();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_selection_find_and_history_share_the_document_owner() {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        let native = NativeView::fixture("甲 apple\napple 乙\n");
        let original = native.snapshot().unwrap();
        native.go_to_line(2).unwrap();
        assert_eq!(native.find_literal("apple", false).unwrap(), (2, 2));
        native.select_next_match(true).unwrap();
        assert_eq!(native.snapshot().unwrap().selections.ranges.len(), 2);
        assert_eq!(native.snapshot().unwrap().revision, original.revision);
        assert!(!native.is_dirty().unwrap());
        assert_eq!(native.replace_literal("apple", "梨", true).unwrap(), 2);
        assert_eq!(native.text().unwrap(), "甲 梨\n梨 乙\n");
        native.undo_redo(true).unwrap();
        assert_eq!(native.text().unwrap(), original.rope.to_string());
        native.undo_redo(false).unwrap();
        assert_eq!(native.text().unwrap(), "甲 梨\n梨 乙\n");
    }
    #[test]
    fn menu_duplicate_line_and_invalid_commands_are_atomic() {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        let native = NativeView::fixture("中文\n尾行");
        native.go_to_line(2).unwrap();
        native.duplicate_line().unwrap();
        assert_eq!(native.text().unwrap(), "中文\n尾行\n尾行");
        native.undo_redo(true).unwrap();
        let before = native.snapshot().unwrap();
        assert!(native.go_to_line(0).is_err());
        assert!(native.replace_literal("", "坏", true).is_err());
        let after = native.snapshot().unwrap();
        assert_eq!(before.rope, after.rope);
        assert_eq!(before.revision, after.revision);
        assert_eq!(before.view_epoch, after.view_epoch);
    }
}
