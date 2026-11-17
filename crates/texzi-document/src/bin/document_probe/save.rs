//! Owner-protocol verification only. These commands perform no disk IO.
use super::*;

fn capture(probe: &Probe, v: &Value, doc: DocId) -> Result<SaveCapture> {
    let c = probe
        .saves
        .get(text(v, "save")?)
        .ok_or("unknown save alias")?;
    if c.snapshot.doc != doc {
        return Err("InvalidSave".into());
    }
    Ok(c.clone())
}

pub(super) fn request(probe: &mut Probe, v: &Value) -> Result<Value> {
    let doc = probe.doc(v)?;
    match text(v, "op")? {
        "save_advance_simulated" => {
            let c = capture(probe, v, doc)?;
            match text(v, "stage")? {
                "WritingTemp" => probe.store.advance_save(&c, SaveStage::WritingTemp),
                "ReadyToReplace" => probe.store.advance_save(&c, SaveStage::ReadyToReplace),
                "Replacing" => probe.store.commit_permit(&c),
                "Verifying" => probe.store.advance_save(&c, SaveStage::Verifying),
                _ => return Err("invalid simulated stage".into()),
            }
            .map_err(error)?;
        }
        "save_fail_simulated" => {
            let c = capture(probe, v, doc)?;
            // Parse the complete request before calling the owner. A parse error
            // must not advance a stage or record a terminal result.
            let outcome = match text(v, "outcome")? {
                "BeforeReplace" => SaveFailureOutcome::BeforeReplace,
                "ReplaceFailed" => SaveFailureOutcome::ReplaceFailed,
                "OutcomeUnknown" => SaveFailureOutcome::OutcomeUnknown,
                _ => return Err("invalid simulated outcome".into()),
            };
            let conflict = v
                .get("conflict")
                .and_then(Value::as_bool)
                .ok_or("expected bool conflict")?;
            probe
                .store
                .finish_save_failure(
                    &c,
                    SaveFailure {
                        outcome,
                        conflict,
                        os_error: None,
                        message: "probe-simulated failure; no disk IO".into(),
                        recovery_paths: Vec::new(),
                    },
                )
                .map_err(error)?;
        }
        "save_ack_pre_replace" => {
            let receipt = probe
                .store
                .acknowledge_pre_replace_failure(doc, num(v, "operation")?)
                .map_err(error)?;
            let mut state = probe.state(doc)?;
            state["acknowledgement"] = json!({
                "failure": observe_probe::failure_json(&receipt.failure),
                "snapshot": observe_probe::body_json(&receipt.snapshot),
                "newly_acknowledged": receipt.newly_acknowledged,
                "scope": "in-memory owner acknowledgement; no disk IO or conflict release"
            });
            return Ok(state);
        }
        _ => return Err("unknown simulated save operation".into()),
    }
    let mut state = probe.state(doc)?;
    state["simulated_save"] = json!(true);
    state["scope"] = json!("in-memory protocol only; no disk IO or file-success proof");
    Ok(state)
}
