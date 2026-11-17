//! Atomic initial import. Only the private captured-open path can create anchors.
use super::*;
use crate::files::{
    LoadedBytes,
    windows_identity::{AttachIssue, OpenAnchor},
};

pub(crate) enum OpenDocument {
    Existing(Snapshot),
    Imported(Snapshot),
}
impl Store {
    pub(crate) fn import_verified_open(
        &mut self,
        loaded: &LoadedBytes,
        anchor: OpenAnchor,
        scope: SaveScope,
    ) -> Result<OpenDocument, AttachIssue> {
        let key = anchor.key();
        if scope.object.as_ref() != Some(&key) {
            return Err(AttachIssue::Owner(Error::InvalidSave));
        }
        // Locate only, before reservation checks. Existing does not change a
        // conflict, apply the requested decoder, or generate a new binding/token.
        if let Some(doc) = self.bound_targets.get(&key) {
            let old = self
                .opened_identities
                .get(doc)
                .ok_or(AttachIssue::ContinuityUnproven)?;
            if !old.same_object(&anchor).map_err(AttachIssue::Identity)? {
                return Err(AttachIssue::ContinuityUnproven);
            }
            return self
                .snapshot(*doc)
                .map(OpenDocument::Existing)
                .map_err(AttachIssue::Owner);
        }
        // Fixed priority for new imports: held/unresolved, then active, then
        // profile/counter checks. No iteration-order-dependent lease selection.
        let parent = self.scope_parents.get(&scope.parent);
        let object = self.scope_objects.get(&key);
        if parent.is_some_and(|l| l.held)
            || object.is_some_and(|l| l.held)
            || self.unresolved_targets.contains_key(&key)
        {
            return Err(AttachIssue::Owner(Error::UnresolvedConflict));
        }
        if parent.is_some() || object.is_some() || self.targets.contains_key(&key) {
            return Err(AttachIssue::Owner(Error::Busy));
        }
        let profile = Profile {
            version: 2,
            file_format: Some(loaded.format()),
            ..Profile::default()
        };
        if !profile.valid() {
            return Err(AttachIssue::Owner(Error::InvalidProfile));
        }
        let next = self
            .next_token
            .checked_add(1)
            .ok_or(AttachIssue::Owner(Error::CounterExhausted))?;
        let body = Arc::new(Body::import_decoded(loaded.text(), profile));
        // Global allocation may consume unpublished IDs; never roll it back.
        // No returned-error branch remains after this allocation.
        #[cfg(test)]
        let doc_counter = self
            .verified_open_doc_counter
            .as_deref()
            .unwrap_or(&NEXT_DOC);
        #[cfg(not(test))]
        let doc_counter = &NEXT_DOC;
        let id = doc_counter
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| AttachIssue::Owner(Error::CounterExhausted))?;
        let snapshot = Snapshot {
            doc: DocId(id),
            revision: Revision(0),
            token: StateToken(self.next_token),
            body,
        };
        let document = Document {
            failures: Vec::new(),
            recovery_snapshots: Vec::new(),
            save_blocked: false,
            closed_views: std::collections::BTreeMap::new(),
            baseline_kind: BaselineKind::VerifiedOpen,
            bound_target: Some(key.clone()),
            binding_epoch: 1,
            save_terminal: None,
            views: HashMap::new(),
            merge: None,
            last_fence: FenceReason::Explicit,
            current: snapshot.clone(),
            baseline: snapshot.token,
            backed: true,
            undo: VecDeque::new(),
            redo: Vec::new(),
            charge: 0,
        };
        // Single serial owner commit: journal observes the fully bound initial
        // state. Allocation/OOM/panic recovery is not implemented by this API.
        self.next_token = next;
        self.docs.insert(snapshot.doc, document);
        self.bound_targets.insert(key, snapshot.doc);
        self.opened_identities.insert(snapshot.doc, anchor);
        self.register_observer_document(snapshot.doc);
        Ok(OpenDocument::Imported(snapshot))
    }
}

#[cfg(test)]
mod tests;
