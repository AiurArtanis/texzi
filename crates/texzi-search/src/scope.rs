use serde::{Deserialize, Serialize};
use std::ops::Range;

/// Nonempty selection spans frozen before the find panel took focus. An empty
/// selections scope matches nothing, including a zero-width match.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    #[default]
    Document,
    Selections(SelectedRanges),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Vec<Range<usize>>")]
pub struct SelectedRanges(Vec<Range<usize>>);

impl From<Vec<Range<usize>>> for SelectedRanges {
    fn from(mut spans: Vec<Range<usize>>) -> Self {
        for span in &mut spans {
            if span.start > span.end {
                std::mem::swap(&mut span.start, &mut span.end);
            }
        }
        spans.retain(|span| !span.is_empty());
        spans.sort_unstable_by_key(|span| span.start);
        let mut count = 0;
        for index in 0..spans.len() {
            let span = spans[index].clone();
            if count > 0 && span.start <= spans[count - 1].end {
                spans[count - 1].end = spans[count - 1].end.max(span.end);
            } else {
                spans[count] = span;
                count += 1;
            }
        }
        spans.truncate(count);
        Self(spans)
    }
}

impl Scope {
    pub fn selections(spans: impl IntoIterator<Item = Range<usize>>) -> Self {
        Self::Selections(SelectedRanges::from(spans.into_iter().collect::<Vec<_>>()))
    }

    pub fn ranges(&self) -> Option<&[Range<usize>]> {
        match self {
            Self::Document => None,
            Self::Selections(selected) => Some(&selected.0),
        }
    }

    /// Tests the complete match against original-subject coordinates. An empty
    /// match may sit at either edge of a selected span, including document EOF.
    pub fn contains(&self, hit: &Range<usize>) -> bool {
        let Some(ranges) = self.ranges() else {
            return true;
        };
        let index = ranges.partition_point(|span| span.end < hit.start);
        ranges
            .get(index)
            .is_some_and(|span| span.start <= hit.start && hit.end <= span.end)
    }

    pub(crate) fn cursor(&self, byte: usize) -> ScopeCursor<'_> {
        ScopeCursor(self.ranges().map(|ranges| {
            let index = ranges.partition_point(|span| span.end < byte);
            &ranges[index..]
        }))
    }
}

/// Matches from both search engines advance monotonically in the subject.
pub(crate) struct ScopeCursor<'a>(Option<&'a [Range<usize>]>);

impl ScopeCursor<'_> {
    pub(crate) fn contains(&mut self, hit: &Range<usize>) -> bool {
        let Some(ranges) = &mut self.0 else {
            return true;
        };
        while ranges.first().is_some_and(|span| span.end < hit.start) {
            *ranges = &ranges[1..];
        }
        ranges
            .first()
            .is_some_and(|span| span.start <= hit.start && hit.end <= span.end)
    }
}
