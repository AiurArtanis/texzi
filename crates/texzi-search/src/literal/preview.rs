use super::*;

impl Literal {
    /// Return a proven visible hit while the document-wide count is pending.
    /// A needle without a shared prefix/suffix cannot overlap an earlier hit
    /// straddling the left edge. Other needles use the normal global scan.
    pub fn first_visible(
        &self,
        rope: &Rope,
        band: Range<usize>,
        scope: &Scope,
        cancelled: impl FnMut() -> bool,
    ) -> Result<Option<Range<usize>>, Cancelled> {
        let prefix = match &self.0 {
            Needle::Bytes(pattern) => &pattern.prefix,
            Needle::Scalars {
                pattern,
                whole_word: false,
                ..
            } => &pattern.prefix,
            _ => return Ok(None),
        };
        if band.start == 0 || prefix.len() > 4096 || prefix.last() != Some(&0) {
            return Ok(None);
        }
        let mut start = band.start.min(rope.len_bytes());
        let mut end = band
            .end
            .min(start.saturating_add(32 * 1024))
            .min(rope.len_bytes());
        while start < end && rope.byte(start) & 0xc0 == 0x80 {
            start += 1;
        }
        while end > start && end < rope.len_bytes() && rope.byte(end) & 0xc0 == 0x80 {
            end -= 1;
        }
        if end <= start {
            return Ok(None);
        }
        let excerpt = Rope::from(rope.byte_slice(start..end));
        let hit = self.scan(&excerpt, 0, &Scope::Document, cancelled, |local| {
            let range = start + local.start..start + local.end;
            if scope.contains(&range) {
                ControlFlow::Break(range)
            } else {
                ControlFlow::Continue(())
            }
        })?;
        Ok(hit.break_value())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_preview_respects_scope_and_never_guesses_overlapping_match_phase() {
        let rope = Rope::from_str("xxxxab aB aaaa");
        let ab = Literal::new("ab", Options::default()).unwrap();
        let scope = Scope::selections([7..9].into_iter());
        assert_eq!(
            ab.first_visible(&rope, 4..9, &scope, || false).unwrap(),
            Some(7..9)
        );
        assert_eq!(
            ab.first_visible(&rope, 9..13, &scope, || false).unwrap(),
            None
        );
        let aa = Literal::new("aa", Options::default()).unwrap();
        assert_eq!(
            aa.first_visible(&rope, 11..14, &Scope::Document, || false)
                .unwrap(),
            None
        );
    }
}
