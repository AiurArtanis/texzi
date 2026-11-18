//! Menu commands use the same versioned Store operations as native input.
use super::*;
use unicode_segmentation::UnicodeSegmentation;

// The document contract counts LF only. NSH also enables Ropey's Unicode-line
// feature in this binary, so Rope::line_* must not redefine logical rows here.
fn logical_lf_starts(snapshot: &ExternalSnapshot) -> Vec<usize> {
    std::iter::once(0)
        .chain(
            snapshot
                .rope
                .bytes()
                .enumerate()
                .filter_map(|(at, byte)| (byte == b'\n').then_some(at + 1)),
        )
        .collect()
}

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
        self.go_to_line_column(line, 1)
    }
    /// User-facing coordinates are one-based logical LF lines and extended
    /// grapheme columns (CONTRACT C03). A Tab is one grapheme, not its visual
    /// expansion; column grapheme_count + 1 is the caret at the line's end.
    pub fn go_to_line_column(&self, line: usize, column: usize) -> Result<(), String> {
        if self.is_composing() {
            return Err("请先完成或取消中文组合，再转到行列".into());
        }
        if line == 0 {
            return Err("行号从 1 开始".into());
        }
        if column == 0 {
            return Err("列号从 1 开始，按字素计数，Tab 算 1 列".into());
        }
        let s = self.snapshot()?;
        let row = line - 1;
        let starts = logical_lf_starts(&s);
        let start = *starts.get(row).ok_or("行号超出文档")?;
        let end = starts.get(row + 1).map_or(s.rope.len_bytes(), |at| at - 1);
        let body = s.rope.byte_slice(start..end).to_string();
        let offset = body
            .grapheme_indices(true)
            .map(|(at, _)| at)
            .chain(std::iter::once(body.len()))
            .nth(column - 1)
            .ok_or("列号超出本行；列按字素计数，Tab 算 1 列")?;
        let at = start + offset;
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
        if s.selections.ranges.get(s.selections.primary).is_none() {
            return Err("没有主选区".into());
        }
        // Match NSH's line-block semantics: an exclusive end at the next
        // line's start leaves that line out; overlapping/touching line runs
        // join. Editing/Plan itself is private to NSH, so publish the resulting
        // source-byte intents through the existing Store planner once.
        struct LineRun {
            first: usize,
            last: usize,
            members: Vec<usize>,
        }
        let starts = logical_lf_starts(&s);
        let mut selected = Vec::with_capacity(s.selections.ranges.len());
        for (index, &(anchor, head)) in s.selections.ranges.iter().enumerate() {
            let (start, end) = (anchor.min(head), anchor.max(head));
            let first = starts.partition_point(|at| *at <= start) - 1;
            let mut last = starts.partition_point(|at| *at <= end) - 1;
            if start != end && last > first && end == starts[last] {
                last -= 1;
            }
            selected.push((first, last, index));
        }
        selected.sort_by_key(|&(first, last, _)| (first, last));
        let mut runs: Vec<LineRun> = Vec::new();
        for (first, last, index) in selected {
            if let Some(run) = runs.last_mut()
                && first <= run.last.saturating_add(1)
            {
                run.last = run.last.max(last);
                run.members.push(index);
            } else {
                runs.push(LineRun {
                    first,
                    last,
                    members: vec![index],
                });
            }
        }
        let mut edits = Vec::with_capacity(runs.len());
        let mut after = vec![(0, 0); s.selections.ranges.len()];
        let mut shift = 0usize;
        for run in runs {
            let start = starts[run.first];
            let terminated = run.last + 1 < starts.len();
            let end = if terminated {
                starts[run.last + 1] - 1
            } else {
                s.rope.len_bytes()
            };
            let lines = s.rope.byte_slice(start..end).to_string();
            // Keep every existing EOL outside the insertion. Only the copy's
            // new LFs use Store's pre-submit dominant-EOL policy.
            let (at, prefix, value) = if terminated {
                (end + 1, 0, format!("{lines}\n"))
            } else {
                (end, 1, format!("\n{lines}"))
            };
            let copy = at
                .checked_add(shift)
                .and_then(|at| at.checked_add(prefix))
                .ok_or("复制行位置超出索引范围")?;
            for member in run.members {
                let (anchor, head) = s.selections.ranges[member];
                after[member] = (
                    copy.checked_add(anchor - start)
                        .ok_or("复制行位置超出索引范围")?,
                    copy.checked_add(head - start)
                        .ok_or("复制行位置超出索引范围")?,
                );
            }
            shift = shift
                .checked_add(value.len())
                .ok_or("复制行长度超出索引范围")?;
            edits.push((at..at, value));
        }
        self.edit(
            s.revision,
            s.view_epoch,
            edits,
            ExternalSelections {
                primary: s.selections.primary,
                ranges: after,
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
    fn bytes(native: &NativeView) -> Vec<u8> {
        native.inspect(|store, view| {
            store
                .snapshot(store.view(view).unwrap().doc)
                .unwrap()
                .body
                .to_utf8()
        })
    }
    fn select_ranges(native: &NativeView, ranges: &[(usize, usize)], primary: usize) {
        let snapshot = native.snapshot().unwrap();
        native
            .select(
                snapshot.revision,
                snapshot.view_epoch,
                ExternalSelections {
                    ranges: ranges.to_vec(),
                    primary,
                },
            )
            .unwrap();
    }
    fn body_audit(native: &NativeView) -> String {
        native.inspect(|store, view| {
            let doc = store.view(view).unwrap().doc;
            let snapshot = store.snapshot(doc).unwrap();
            format!(
                "{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}",
                snapshot.revision,
                snapshot.token,
                snapshot.body.to_utf8(),
                snapshot.body.eols(),
                snapshot.body.origins(),
                snapshot.body.profile,
                store.dirty(doc),
                store.history_depth(doc)
            )
        })
    }
    fn full_audit(native: &NativeView) -> String {
        format!(
            "{}|{}",
            body_audit(native),
            native.inspect(|store, view| format!(
                "{:?}|{:?}",
                store.view(view),
                store.last_fence(store.view(view).unwrap().doc)
            ))
        )
    }
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

    #[test]
    fn menu_duplicate_all_selection_line_runs_deduplicates_and_preserves_direction_in_one_history()
    {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        let raw = "aA\r\nbB\ncC\rdD\r\neE";
        let native = NativeView::fixture(raw);
        // Two separate selections touch line a, another crosses b/c, and a
        // final caret touches e. The unselected d line separates two copies.
        select_ranges(&native, &[(2, 1), (0, 0), (4, 7), (13, 13)], 0);
        let before = native.snapshot().unwrap();
        assert_eq!(
            before.selections.ranges,
            vec![(0, 0), (2, 1), (4, 7), (13, 13)]
        );
        assert_eq!(before.selections.primary, 1);
        native.duplicate_line().unwrap();
        let expected = "aA\r\nbB\ncC\raA\r\nbB\r\ncC\r\ndD\r\neE\r\neE";
        assert_eq!(bytes(&native), expected.as_bytes());
        let after = native.snapshot().unwrap();
        assert_eq!(after.revision, before.revision + 1);
        assert_eq!(
            after.selections.ranges,
            vec![(9, 9), (11, 10), (13, 16), (25, 25)]
        );
        assert_eq!(
            after.selections.primary, 1,
            "the reversed primary selection stays primary and reversed"
        );
        assert_eq!(
            native
                .inspect(|store, view| store.history_depth(store.view(view).unwrap().doc).unwrap()),
            (1, 0)
        );
        native.undo_redo(true).unwrap();
        assert_eq!(bytes(&native), raw.as_bytes());
        assert_eq!(native.snapshot().unwrap().selections, before.selections);
        native.undo_redo(false).unwrap();
        assert_eq!(bytes(&native), expected.as_bytes());
        assert_eq!(native.snapshot().unwrap().selections, after.selections);
    }

    #[test]
    fn menu_duplicate_handles_exclusive_line_ends_empty_eof_and_unicode_without_extra_blank_lines()
    {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        for (raw, ranges, expected) in [
            ("", vec![(0, 0)], "\r\n"),
            ("last", vec![(4, 4)], "last\r\nlast"),
            ("a\n", vec![(2, 2)], "a\n\n"),
            ("a\r\nb", vec![(3, 0)], "a\r\nb\r\na\r\nb"),
            ("a\r\nb\nc", vec![(0, 2)], "a\r\na\r\nb\nc"),
            (
                "A😀e\u{301}👩🏽‍💻🇨🇳B\r\n尾\t行",
                vec![
                    ("A😀e\u{301}👩🏽‍💻🇨🇳B".len(), 0),
                    (
                        "A😀e\u{301}👩🏽‍💻🇨🇳B\n尾\t".len(),
                        "A😀e\u{301}👩🏽‍💻🇨🇳B\n尾\t".len(),
                    ),
                ],
                "A😀e\u{301}👩🏽‍💻🇨🇳B\r\n尾\t行\r\nA😀e\u{301}👩🏽‍💻🇨🇳B\r\n尾\t行",
            ),
            (
                "a\u{2028}b\r\nz",
                vec![(1, 1)],
                "a\u{2028}b\r\na\u{2028}b\r\nz",
            ),
        ] {
            eprintln!("duplicate-line fixture: {raw:?}");
            let native = NativeView::fixture(raw);
            select_ranges(&native, &ranges, 0);
            let before = native.snapshot().unwrap();
            native.duplicate_line().unwrap();
            assert_eq!(bytes(&native), expected.as_bytes(), "{raw:?}");
            assert_eq!(native.snapshot().unwrap().revision, before.revision + 1);
            assert_eq!(
                native.inspect(|store, view| store
                    .history_depth(store.view(view).unwrap().doc)
                    .unwrap()),
                (1, 0)
            );
            native.undo_redo(true).unwrap();
            assert_eq!(bytes(&native), raw.as_bytes());
            assert_eq!(native.snapshot().unwrap().selections, before.selections);
            native.undo_redo(false).unwrap();
            assert_eq!(bytes(&native), expected.as_bytes());
        }
    }

    #[test]
    fn menu_goto_line_column_uses_exact_graphemes_and_logical_lf_rows_without_body_changes() {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        let raw = "\u{feff}A😀e\u{301}👩🏽‍💻🇨🇳B\r\n\t中\n尾\r";
        let native = NativeView::fixture(raw);
        let before = body_audit(&native);
        for (line, column, expected) in [
            (1, 1, 0),
            (1, 2, "A".len()),
            (1, 3, "A😀".len()),
            (1, 4, "A😀e\u{301}".len()),
            (1, 5, "A😀e\u{301}👩🏽‍💻".len()),
            (1, 6, "A😀e\u{301}👩🏽‍💻🇨🇳".len()),
            (1, 7, "A😀e\u{301}👩🏽‍💻🇨🇳B".len()),
            (2, 2, "A😀e\u{301}👩🏽‍💻🇨🇳B\n\t".len()),
            (2, 3, "A😀e\u{301}👩🏽‍💻🇨🇳B\n\t中".len()),
            (3, 2, "A😀e\u{301}👩🏽‍💻🇨🇳B\n\t中\n尾".len()),
            (4, 1, "A😀e\u{301}👩🏽‍💻🇨🇳B\n\t中\n尾\n".len()),
        ] {
            native.go_to_line_column(line, column).unwrap();
            assert_eq!(
                native.snapshot().unwrap().selections.ranges,
                vec![(expected, expected)],
                "{line}:{column}"
            );
            assert_eq!(body_audit(&native), before);
            assert_eq!(bytes(&native), raw.as_bytes());
        }
        native.go_to_line(2).unwrap();
        assert_eq!(
            native.snapshot().unwrap().selections.ranges,
            vec![("A😀e\u{301}👩🏽‍💻🇨🇳B\n".len(), "A😀e\u{301}👩🏽‍💻🇨🇳B\n".len())]
        );
        let invalid_before = full_audit(&native);
        for (line, column) in [
            (0, 1),
            (1, 0),
            (1, 8),
            (2, 4),
            (4, 2),
            (5, 1),
            (usize::MAX, 1),
            (1, usize::MAX),
        ] {
            assert!(native.go_to_line_column(line, column).is_err());
            assert_eq!(
                full_audit(&native),
                invalid_before,
                "invalid {line}:{column}"
            );
        }
        let native = NativeView::fixture("a\u{2028}b\n尾");
        native.go_to_line_column(1, 3).unwrap();
        assert_eq!(
            native.snapshot().unwrap().selections.ranges,
            vec![("a\u{2028}".len(), "a\u{2028}".len())]
        );
        native.go_to_line(2).unwrap();
        assert_eq!(
            native.snapshot().unwrap().selections.ranges,
            vec![("a\u{2028}b\n".len(), "a\u{2028}b\n".len())]
        );
    }

    #[test]
    fn menu_invalid_goto_and_ime_commands_keep_selection_body_and_existing_redo() {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        let native = NativeView::fixture("甲\r\n尾");
        native.duplicate_line().unwrap();
        native.undo_redo(true).unwrap();
        assert_eq!(native.has_undo_redo().unwrap(), (false, true));
        let before = full_audit(&native);
        assert!(native.go_to_line_column(2, 3).is_err());
        assert_eq!(full_audit(&native), before);
        native.set_composing(true);
        assert!(native.go_to_line_column(1, 1).is_err());
        assert!(native.duplicate_line().is_err());
        assert_eq!(full_audit(&native), before);
        native.set_composing(false);
        native.undo_redo(false).unwrap();
        assert_eq!(bytes(&native), "甲\r\n甲\r\n尾".as_bytes());
    }

    #[test]
    fn menu_store_eol_rank_keeps_unicode_break_edits_history_and_immutable_source_origins() {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        for separator in ["\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}"] {
            let raw = format!("\u{feff}甲{separator}\r\n乙\n{separator}丙\r尾");
            let native = NativeView::fixture(&raw);
            let original = native
                .inspect(|store, view| store.snapshot(store.view(view).unwrap().doc).unwrap());
            let original_origins = format!("{:?}", original.body.origins());
            let origin_source = original
                .body
                .origins()
                .into_iter()
                .find_map(|(_, origin)| origin.map(|origin| origin.source))
                .unwrap();
            let before = native.snapshot().unwrap();
            let text = before.rope.to_string();
            let first = text.find(separator).unwrap();
            let second = text.rfind(separator).unwrap();
            let logical_after = format!("甲{separator}新\n乙\n前\n{separator}后丙\n尾");
            native
                .edit(
                    before.revision,
                    before.view_epoch,
                    vec![
                        (
                            first..first + separator.len() + 1,
                            format!("{separator}新\n"),
                        ),
                        (
                            second..second + separator.len(),
                            format!("前\n{separator}后"),
                        ),
                    ],
                    ExternalSelections {
                        primary: 0,
                        ranges: vec![(logical_after.len(), logical_after.len())],
                    },
                    ExternalEditKind::Other,
                )
                .unwrap();
            let expected = format!("\u{feff}甲{separator}新\r\n乙\n前\r\n{separator}后丙\r尾");
            assert_eq!(
                bytes(&native),
                expected.as_bytes(),
                "separator {separator:?}"
            );
            assert_eq!(
                original.body.to_utf8(),
                raw.as_bytes(),
                "old persistent snapshot remains immutable"
            );
            let edited = native
                .inspect(|store, view| store.snapshot(store.view(view).unwrap().doc).unwrap());
            assert_eq!(edited.revision.0, original.revision.0 + 1);
            assert_eq!(
                edited.body.eols(),
                vec![doc::Eol::CrLf, doc::Eol::Lf, doc::Eol::CrLf, doc::Eol::Cr]
            );
            let untouched = logical_after.find("乙\n").unwrap();
            assert!(
                edited
                    .body
                    .origins()
                    .iter()
                    .any(|(range, origin)| range.start <= untouched
                        && range.end >= untouched + "乙\n".len()
                        && origin.as_ref().is_some_and(|origin| std::sync::Arc::ptr_eq(
                            &origin.source,
                            &origin_source
                        ))),
                "untouched original bytes retain provenance"
            );
            assert_eq!(
                native.inspect(|store, view| store
                    .history_depth(store.view(view).unwrap().doc)
                    .unwrap()),
                (1, 0)
            );
            native.undo_redo(true).unwrap();
            assert_eq!(bytes(&native), raw.as_bytes());
            native.inspect(|store, view| {
                let restored = store.snapshot(store.view(view).unwrap().doc).unwrap();
                assert_eq!(restored.token, original.token);
                assert_eq!(restored.body.eols(), original.body.eols());
                assert_eq!(restored.body.profile, original.body.profile);
                assert_eq!(format!("{:?}", restored.body.origins()), original_origins);
            });
            native.undo_redo(false).unwrap();
            assert_eq!(bytes(&native), expected.as_bytes());
            native.inspect(|store, view| {
                assert_eq!(
                    store.snapshot(store.view(view).unwrap().doc).unwrap().token,
                    edited.token
                )
            });
            assert_eq!(original.body.to_utf8(), raw.as_bytes());
        }
    }

    #[test]
    fn menu_store_eol_rank_handles_edits_on_both_sides_of_unicode_break_and_lf() {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        for separator in ["\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}"] {
            let raw = format!("a{separator}\r\nb\n");
            let lf = 1 + separator.len();
            for (range, replacement, expected) in [
                (1..1, "\n".to_owned(), format!("a\r\n{separator}\r\nb\n")),
                (lf..lf, "\n".to_owned(), format!("a{separator}\r\n\r\nb\n")),
                (lf..lf + 1, String::new(), format!("a{separator}b\n")),
                (1..lf, String::new(), "a\r\nb\n".to_owned()),
                (
                    1..lf + 1,
                    format!("\n{separator}"),
                    format!("a\r\n{separator}b\n"),
                ),
            ] {
                let native = NativeView::fixture(&raw);
                let before = native.snapshot().unwrap();
                native
                    .edit(
                        before.revision,
                        before.view_epoch,
                        vec![(range.clone(), replacement)],
                        ExternalSelections {
                            primary: 0,
                            ranges: vec![(0, 0)],
                        },
                        ExternalEditKind::Other,
                    )
                    .unwrap();
                assert_eq!(
                    bytes(&native),
                    expected.as_bytes(),
                    "separator {separator:?}, source range {range:?}"
                );
                assert_eq!(
                    native.inspect(|store, view| store
                        .history_depth(store.view(view).unwrap().doc)
                        .unwrap()),
                    (1, 0)
                );
                native.undo_redo(true).unwrap();
                assert_eq!(bytes(&native), raw.as_bytes());
                native.undo_redo(false).unwrap();
                assert_eq!(bytes(&native), expected.as_bytes());
            }
        }
    }
}
