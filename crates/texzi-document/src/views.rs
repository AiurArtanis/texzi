use crate::{Affinity, Error, Snapshot, SourceByte, Transaction, coordinates::boundary};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ViewId(pub(crate) u64);
/// Logical labels only. No renderer, projection, system IME or HWND capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeTag {
    Plain,
    Source,
    Rich,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub position: SourceByte,
    pub affinity: Affinity,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Anchor,
    pub head: Anchor,
}
impl Selection {
    pub fn caret(s: &Snapshot, offset: usize) -> Self {
        let p = Anchor {
            position: SourceByte::at(s, offset),
            affinity: Affinity::After,
        };
        Self { anchor: p, head: p }
    }
    pub fn range(&self) -> std::ops::Range<usize> {
        self.anchor.position.offset.min(self.head.position.offset)
            ..self.anchor.position.offset.max(self.head.position.offset)
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionSet {
    pub selections: Vec<Selection>,
    pub primary: usize,
}
impl SelectionSet {
    pub fn caret(s: &Snapshot, offset: usize) -> Self {
        Self {
            selections: vec![Selection::caret(s, offset)],
            primary: 0,
        }
    }
    pub(crate) fn reversion(&self, s: &Snapshot) -> Self {
        let mut copy = self.clone();
        for item in &mut copy.selections {
            item.anchor.position = SourceByte::at(s, item.anchor.position.offset);
            item.head.position = SourceByte::at(s, item.head.position.offset);
        }
        copy
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnchorSnap {
    pub requested: usize,
    pub actual: usize,
    pub affinity: Affinity,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewState {
    pub id: ViewId,
    pub doc: crate::DocId,
    pub epoch: u64,
    pub mode: ModeTag,
    pub selection: SelectionSet,
    pub scroll: Anchor,
    pub pixel_bias: i32,
    /// Explicit mapping adjustments, never reported as an Exact conversion.
    pub mapping_snaps: Vec<AnchorSnap>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    Typing,
    Paste,
    ImeCommit,
    Structural,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FenceReason {
    Navigation,
    Mode,
    Save,
    CrossView,
    Service,
    Explicit,
    UndoRedo,
}
#[derive(Clone, Debug)]
pub struct ViewTransaction {
    pub view: ViewId,
    pub expected_epoch: u64,
    pub transaction: Transaction,
    pub selection_after: SelectionSet,
    pub kind: InputKind,
    pub tick_ms: u64,
}

pub(crate) fn validate_anchor(s: &Snapshot, a: Anchor) -> Result<(), Error> {
    s.check_position(a.position.doc, a.position.revision)?;
    s.body.scalar(a.position.offset)?;
    if !boundary(&s.body, a.position.offset)? {
        return Err(Error::InsideGrapheme);
    }
    Ok(())
}
fn validate_members(s: &Snapshot, set: &SelectionSet) -> Result<(), Error> {
    if set.selections.is_empty() || set.primary >= set.selections.len() {
        return Err(Error::InvalidSelection);
    }
    for selection in &set.selections {
        validate_anchor(s, selection.anchor)?;
        validate_anchor(s, selection.head)?;
    }
    Ok(())
}
/// Intent planning only: sort, merge overlaps/duplicate carets and carets touching
/// a selected range; keep adjacent non-empty selections separate. Preserve the
/// primary member's direction (or the first sorted member's direction for other groups).
pub fn normalize_selections(s: &Snapshot, input: SelectionSet) -> Result<SelectionSet, Error> {
    validate_members(s, &input)?;
    let primary = input.primary;
    let mut sorted: Vec<_> = input.selections.into_iter().enumerate().collect();
    sorted.sort_by_key(|(_, sel)| {
        let r = sel.range();
        (r.start, r.end)
    });
    let mut groups: Vec<(Selection, Vec<usize>)> = Vec::new();
    for (index, selection) in sorted {
        if let Some((last, members)) = groups.last_mut() {
            let a = last.range();
            let b = selection.range();
            let merge = b.start < a.end || (b.start == a.end && (a.is_empty() || b.is_empty()));
            if merge {
                let reverse = if index == primary {
                    selection.anchor.position.offset > selection.head.position.offset
                } else {
                    last.anchor.position.offset > last.head.position.offset
                };
                let start = a.start.min(b.start);
                let end = a.end.max(b.end);
                let left = Anchor {
                    position: SourceByte::at(s, start),
                    affinity: Affinity::Before,
                };
                let right = Anchor {
                    position: SourceByte::at(s, end),
                    affinity: Affinity::After,
                };
                *last = if reverse {
                    Selection {
                        anchor: right,
                        head: left,
                    }
                } else if start == end {
                    Selection::caret(s, start)
                } else {
                    Selection {
                        anchor: left,
                        head: right,
                    }
                };
                members.push(index);
                continue;
            }
        }
        groups.push((selection, vec![index]));
    }
    let primary = groups
        .iter()
        .position(|(_, members)| members.contains(&primary))
        .unwrap();
    Ok(SelectionSet {
        selections: groups.into_iter().map(|(s, _)| s).collect(),
        primary,
    })
}
pub(crate) fn validate_selection(s: &Snapshot, set: &SelectionSet) -> Result<(), Error> {
    validate_members(s, set)?;
    // Authority rejects an unnormalized plan; it never guesses the user's intent.
    if normalize_selections(s, set.clone())? != *set {
        return Err(Error::InvalidSelection);
    }
    Ok(())
}

pub(crate) fn snap_anchor(s: &Snapshot, a: Anchor) -> Result<(Anchor, Option<AnchorSnap>), Error> {
    s.check_position(a.position.doc, a.position.revision)?;
    let mut c = s.body.scalar(a.position.offset)?;
    let requested = a.position.offset;
    let r = s.body.source();
    let mut at = requested;
    while !boundary(&s.body, at)? {
        match a.affinity {
            Affinity::Before => c -= 1,
            Affinity::After => c += 1,
        }
        at = r.char_to_byte(c);
    }
    Ok((
        Anchor {
            position: SourceByte::at(s, at),
            affinity: a.affinity,
        },
        (at != requested).then_some(AnchorSnap {
            requested,
            actual: at,
            affinity: a.affinity,
        }),
    ))
}
