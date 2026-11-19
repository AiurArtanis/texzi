use super::{SaveFailureOutcome, SaveFailureRecord, SaveStage, Snapshot, Store};
use crate::{DocId, Error};

/// Owned recovery data for an explicitly acknowledged pre-replace failure.
/// The snapshot is the operation's original capture, not the current document.
/// Its Body Arc, token and serialization format are preserved.
///
/// This receipt records owner responsibility, not disk success, file cleanup,
/// unknown-outcome verification or release of any publication/scope resource.
/// It owns memory recovery data, not native handles or parent/identity witnesses.
#[must_use = "The owner must retain recovery data or explicitly decide to discard it."]
#[derive(Clone, Debug)]
pub struct PreReplaceAcknowledgement {
    pub failure: SaveFailureRecord,
    pub snapshot: Snapshot,
    pub newly_acknowledged: bool,
}

pub(super) fn is_eligible_pre_replace_failure(failure: &SaveFailureRecord) -> bool {
    failure.detail.outcome == SaveFailureOutcome::BeforeReplace
        && !failure.detail.conflict
        && matches!(
            failure.stage,
            SaveStage::Encoding | SaveStage::WritingTemp | SaveStage::ReadyToReplace
        )
}

impl Store {
    pub(crate) fn pre_replace_acknowledgement(
        &self,
        doc: DocId,
        operation: u64,
    ) -> Result<PreReplaceAcknowledgement, Error> {
        let document = self.docs.get(&doc).ok_or(Error::InvalidDoc)?;
        let failure = document
            .failures
            .iter()
            .find(|failure| failure.operation == operation)
            .ok_or(Error::InvalidSave)?;
        let snapshot = document
            .recovery_snapshots
            .iter()
            .find(|(candidate, _)| *candidate == operation)
            .map(|(_, snapshot)| snapshot)
            .ok_or(Error::InvalidSave)?;
        // Validate the retained operation/capture pair before eligibility or
        // any mutation. Never substitute the document's current snapshot.
        if snapshot.doc != doc
            || snapshot.revision != failure.revision
            || snapshot.token != failure.token
        {
            return Err(Error::InvalidSave);
        }
        if !is_eligible_pre_replace_failure(failure) || document.save_blocked {
            return Err(Error::UnresolvedConflict);
        }
        if self.saves.contains_key(&doc) {
            return Err(Error::Busy);
        }
        let newly_acknowledged = !failure.acknowledged_before_replace;
        let mut failure = failure.clone();
        failure.acknowledged_before_replace = true;
        Ok(PreReplaceAcknowledgement {
            failure,
            snapshot: snapshot.clone(),
            newly_acknowledged,
        })
    }

    pub(crate) fn acknowledge_pre_replace_failure_unpublished(
        &mut self,
        doc: DocId,
        operation: u64,
    ) -> Result<PreReplaceAcknowledgement, Error> {
        let acknowledgement = self.pre_replace_acknowledgement(doc, operation)?;
        // observed_write has already preflighted event counters. Every
        // fallible check above precedes this sole document-state mutation.
        if acknowledgement.newly_acknowledged {
            self.docs
                .get_mut(&doc)
                .unwrap()
                .failures
                .iter_mut()
                .find(|failure| failure.operation == operation)
                .unwrap()
                .acknowledged_before_replace = true;
        }
        Ok(acknowledgement)
    }
}
