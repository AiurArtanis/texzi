//! Intent planning for the native editor. Authority remains submit_view.
use crate::*;
use std::{ops::Range, sync::Arc};

impl Store {
    /// Prepare explicit source-byte replacements and directional post-selection.
    /// No mutation; invalid/stale plans have no partial effect.
    pub fn plan_view_edits(
        &self,
        view: ViewId,
        expected_revision: Revision,
        expected_epoch: u64,
        replacements: Vec<(Range<usize>, String)>,
        after: Vec<(usize, usize)>,
        primary: usize,
        kind: InputKind,
        tick_ms: u64,
    ) -> Result<ViewTransaction, Error> {
        let v = self.view(view)?;
        if v.epoch != expected_epoch {
            return Err(Error::StaleView);
        }
        let s = self.snapshot(v.doc)?;
        if s.revision != expected_revision {
            return Err(Error::StaleRevision);
        }
        let edits: Vec<_> = replacements
            .into_iter()
            .map(|(r, t)| Edit::input(&s, r, &t))
            .collect();
        let mut end = 0;
        let mut changed = false;
        for (i, e) in edits.iter().enumerate() {
            let r = e.start.offset..e.end.offset;
            if r.start > r.end || r.end > s.body.len_bytes() || (i > 0 && r.start < end) {
                return Err(Error::InvalidOverlap);
            }
            s.body.scalar(r.start)?;
            s.body.scalar(r.end)?;
            if !s.is_grapheme_boundary(e.start)? || !s.is_grapheme_boundary(e.end)? {
                return Err(Error::InsideGrapheme);
            }
            changed |= s.body.source().byte_slice(r.clone()).to_string() != e.text
                || s.body.eols_in(r.clone()) != e.eols;
            end = r.end;
        }
        let mut body = (*s.body).clone();
        for e in edits.iter().rev() {
            body.replace(e.start.offset..e.end.offset, &e.text, &e.eols);
        }
        let preview = Snapshot {
            doc: s.doc,
            revision: Revision(if changed {
                s.revision.0.checked_add(1).ok_or(Error::CounterExhausted)?
            } else {
                s.revision.0
            }),
            token: s.token,
            body: Arc::new(body),
        };
        let selection_after = normalize_selections(
            &preview,
            SelectionSet {
                primary,
                selections: after
                    .into_iter()
                    .map(|(a, h)| Selection {
                        anchor: Anchor {
                            position: SourceByte::at(&preview, a),
                            affinity: Affinity::After,
                        },
                        head: Anchor {
                            position: SourceByte::at(&preview, h),
                            affinity: Affinity::After,
                        },
                    })
                    .collect(),
            },
        )?;
        Ok(ViewTransaction {
            view,
            expected_epoch,
            transaction: Transaction {
                doc: s.doc,
                base: s.revision,
                edits,
                boundary: BoundaryPolicy::Grapheme,
                cancelled: false,
            },
            selection_after,
            kind,
            tick_ms,
        })
    }
}
