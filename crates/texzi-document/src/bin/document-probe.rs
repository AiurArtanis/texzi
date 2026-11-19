//! JSON-lines verification harness; stdin/stdout, in-memory documents only.
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{self, BufRead, Write},
};
use texzi_document::*;
#[path = "document_probe/observe.rs"]
mod observe_probe;
#[path = "document_probe/save.rs"]
mod save_probe;
type Result<T> = std::result::Result<T, String>;
fn text<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v.get(k)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("expected string {k}"))
}
fn num(v: &Value, k: &str) -> Result<u64> {
    v.get(k)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("expected unsigned {k}"))
}
fn offset(n: u64) -> Result<usize> {
    usize::try_from(n).map_err(|_| "offset overflow".into())
}
fn error(e: Error) -> String {
    format!("{e:?}")
}
fn kind(v: &Value) -> Result<InputKind> {
    match v
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("structural")
    {
        "typing" => Ok(InputKind::Typing),
        "paste" => Ok(InputKind::Paste),
        "ime" => Ok(InputKind::ImeCommit),
        "structural" => Ok(InputKind::Structural),
        _ => Err("unknown kind".into()),
    }
}
fn mode(v: &Value) -> Result<ModeTag> {
    match v.get("mode").and_then(Value::as_str).unwrap_or("source") {
        "plain" => Ok(ModeTag::Plain),
        "source" => Ok(ModeTag::Source),
        "rich" => Ok(ModeTag::Rich),
        _ => Err("unknown logical mode".into()),
    }
}
fn affinity(v: Option<&Value>, default: Affinity) -> Result<Affinity> {
    match v.and_then(Value::as_str) {
        None => Ok(default),
        Some("before") => Ok(Affinity::Before),
        Some("after") => Ok(Affinity::After),
        _ => Err("unknown affinity".into()),
    }
}
fn selections(v: &Value, s: &Snapshot) -> Result<SelectionSet> {
    let input = v
        .get("selection")
        .and_then(Value::as_array)
        .ok_or("expected selection array")?;
    let mut selections = Vec::new();
    for item in input {
        let a = offset(num(item, "anchor")?)?;
        let h = offset(num(item, "head")?)?;
        let aa = if a < h {
            Affinity::Before
        } else {
            Affinity::After
        };
        let ha = if h < a {
            Affinity::Before
        } else {
            Affinity::After
        };
        selections.push(Selection {
            anchor: Anchor {
                position: SourceByte::at(s, a),
                affinity: affinity(item.get("anchor_affinity"), aa)?,
            },
            head: Anchor {
                position: SourceByte::at(s, h),
                affinity: affinity(item.get("head_affinity"), ha)?,
            },
        });
    }
    Ok(SelectionSet {
        selections,
        primary: offset(v.get("primary").and_then(Value::as_u64).unwrap_or(0))?,
    })
}
fn selection_json(set: &SelectionSet) -> Value {
    json!({"primary":set.primary,"selection":set.selections.iter().map(|s|json!({"anchor":s.anchor.position.offset,"head":s.head.position.offset,"revision":s.head.position.revision.0,"anchor_affinity":format!("{:?}",s.anchor.affinity),"head_affinity":format!("{:?}",s.head.affinity)})).collect::<Vec<_>>()})
}
fn delta_json(d: &Delta) -> Value {
    json!({"start":d.range.start,"end":d.range.end,"text":d.text,"eols":d.eols.iter().map(|e|format!("{e:?}")).collect::<Vec<_>>()})
}
fn receipt_json(r: &Receipt) -> Value {
    json!({"previous":r.previous.0,"revision":r.snapshot.revision.0,"changed":r.changed,"forward":r.forward.iter().map(delta_json).collect::<Vec<_>>(),"inverse":r.inverse.iter().map(delta_json).collect::<Vec<_>>()})
}
struct Probe {
    store: Store,
    docs: HashMap<String, DocId>,
    views: HashMap<String, ViewId>,
    saves: HashMap<String, SaveCapture>,
    observer_wire: observe_probe::Wire,
}
impl Probe {
    fn new() -> Self {
        Self {
            store: Store::new(HistoryBudget::default()),
            docs: HashMap::new(),
            views: HashMap::new(),
            saves: HashMap::new(),
            observer_wire: observe_probe::Wire::default(),
        }
    }
    fn doc(&self, v: &Value) -> Result<DocId> {
        self.docs
            .get(text(v, "doc")?)
            .copied()
            .ok_or("unknown doc alias".into())
    }
    fn view(&self, v: &Value) -> Result<ViewState> {
        let id = self
            .views
            .get(text(v, "view")?)
            .ok_or("unknown view alias")?;
        self.store.view(*id).map_err(error)
    }
    fn state(&self, doc: DocId) -> Result<Value> {
        let read = self.store.read_document(doc).map_err(error)?;
        let s = read.document;
        let depth = self.store.history_depth(doc).map_err(error)?;
        let views=read.views.iter().map(|v|json!({"name":self.views.iter().find(|(_,id)|**id==v.id).map(|(name,_)|name),"epoch":v.epoch,"mode":format!("{:?}",v.mode),"selection":selection_json(&v.selection),"scroll":v.scroll.position.offset,"scroll_revision":v.scroll.position.revision.0,"pixel_bias":v.pixel_bias,"snaps":v.mapping_snaps.iter().map(|s|json!({"requested":s.requested,"actual":s.actual,"affinity":format!("{:?}",s.affinity)})).collect::<Vec<_>>() })).collect::<Vec<_>>();
        Ok(
            json!({"source":s.body.source().to_string(),"serialized_utf8":String::from_utf8(s.body.to_utf8()).unwrap(),"revision":s.revision.0,"token":format!("{:?}",s.token),"dirty":self.store.dirty(doc).map_err(error)?,"undo":depth.0,"redo":depth.1,"history_charge":self.store.history_charge(),"views":views}),
        )
    }
    fn request(&mut self, v: &Value) -> Result<Value> {
        let op = text(v, "op")?;
        if op.starts_with("obs_") {
            return self
                .observer_wire
                .request(&mut self.store, &self.docs, &self.views, v);
        }
        if op == "hello" {
            return Ok(
                json!({"protocol":1,"scope":"in-memory component; no GUI or disk IO","unicode":"17.0.0","merge_window_ms":INPUT_MERGE_WINDOW_MS,"merge_max_bytes":INPUT_MERGE_MAX_BYTES}),
            );
        }
        if op == "open" {
            let name = text(v, "doc")?.to_string();
            if self.docs.contains_key(&name) {
                return Err("duplicate doc alias".into());
            }
            let s = self
                .store
                .open_utf8(text(v, "text")?, Profile::default(), false)
                .map_err(error)?;
            self.docs.insert(name, s.doc);
            return self.state(s.doc);
        }
        if matches!(
            op,
            "save_advance_simulated" | "save_fail_simulated" | "save_ack_pre_replace"
        ) {
            return save_probe::request(self, v);
        }
        let doc = self.doc(v)?;
        let s = self.store.snapshot(doc).map_err(error)?;
        let mut receipt = None;
        match op {
            "state" => {}
            "open_view" => {
                let name = text(v, "view")?.to_string();
                if self.views.contains_key(&name) {
                    return Err("duplicate view alias".into());
                }
                let set = selections(v, &s)?;
                let scroll = Anchor {
                    position: SourceByte::at(
                        &s,
                        offset(v.get("scroll").and_then(Value::as_u64).unwrap_or(0))?,
                    ),
                    affinity: affinity(v.get("scroll_affinity"), Affinity::Before)?,
                };
                let opened = self
                    .store
                    .open_view(
                        doc,
                        mode(v)?,
                        set,
                        scroll,
                        v.get("pixel_bias")
                            .and_then(Value::as_i64)
                            .unwrap_or(0)
                            .try_into()
                            .map_err(|_| "bias overflow")?,
                    )
                    .map_err(error)?;
                self.views.insert(name, opened.id);
            }
            "normalize" => {
                return Ok(selection_json(
                    &normalize_selections(&s, selections(v, &s)?).map_err(error)?,
                ));
            }
            "selection" => {
                let view = self.view(v)?;
                if view.doc != doc {
                    return Err("InvalidView".into());
                }
                let set = selections(v, &s)?;
                let set = if v.get("normalize").and_then(Value::as_bool).unwrap_or(false) {
                    normalize_selections(&s, set).map_err(error)?
                } else {
                    set
                };
                self.store
                    .set_selection(view.id, num(v, "epoch")?, set)
                    .map_err(error)?;
            }
            "mode" => {
                let view = self.view(v)?;
                if view.doc != doc {
                    return Err("InvalidView".into());
                }
                self.store
                    .set_mode_tag(view.id, num(v, "epoch")?, mode(v)?)
                    .map_err(error)?;
            }
            "close_view" => {
                let view = self.view(v)?;
                if view.doc != doc {
                    return Err("InvalidView".into());
                }
                self.store
                    .close_view(view.id, num(v, "epoch")?)
                    .map_err(error)?;
            }
            "replace" => {
                let view = self.view(v)?;
                if view.doc != doc {
                    return Err("InvalidView".into());
                }
                let mut plan = self
                    .store
                    .plan_replace(view.id, text(v, "text")?, kind(v)?, num(v, "tick")?)
                    .map_err(error)?;
                // epoch can be intentionally stale for an independent test.
                plan.expected_epoch = num(v, "epoch")?;
                receipt = Some(self.store.submit_view(plan).map_err(error)?);
            }
            "submit" | "service_submit" => {
                let base = Revision(num(v, "base")?);
                let mut coordinates = s.clone();
                coordinates.revision = base;
                let mut edits = Vec::new();
                for e in v
                    .get("edits")
                    .and_then(Value::as_array)
                    .ok_or("expected edits")?
                {
                    let mut edit = Edit::input(
                        &coordinates,
                        offset(num(e, "start")?)?..offset(num(e, "end")?)?,
                        text(e, "text")?,
                    );
                    if let Some(values) = e.get("eols").and_then(Value::as_array) {
                        edit.eols = values
                            .iter()
                            .map(|x| match x.as_str() {
                                Some("Lf") => Ok(Eol::Lf),
                                Some("CrLf") => Ok(Eol::CrLf),
                                Some("Cr") => Ok(Eol::Cr),
                                _ => Err("unknown EOL".to_string()),
                            })
                            .collect::<Result<_>>()?;
                    }
                    edits.push(edit);
                }
                let txn = Transaction {
                    doc,
                    base,
                    edits,
                    boundary: if v.get("scalar").and_then(Value::as_bool).unwrap_or(false) {
                        BoundaryPolicy::Scalar
                    } else {
                        BoundaryPolicy::Grapheme
                    },
                    cancelled: v.get("cancelled").and_then(Value::as_bool).unwrap_or(false),
                };
                receipt = Some(if op == "service_submit" {
                    self.store.submit(txn).map_err(error)?
                } else {
                    let view = self.view(v)?;
                    let selection_doc =
                        if let Some(name) = v.get("selection_doc").and_then(Value::as_str) {
                            *self.docs.get(name).ok_or("unknown selection doc")?
                        } else {
                            doc
                        };
                    let mut future = self.store.snapshot(selection_doc).map_err(error)?;
                    future.revision = Revision(num(v, "selection_revision")?);
                    let after = selections(v, &future)?;
                    self.store
                        .submit_view(ViewTransaction {
                            view: view.id,
                            expected_epoch: num(v, "epoch")?,
                            transaction: txn,
                            selection_after: after,
                            kind: kind(v)?,
                            tick_ms: num(v, "tick")?,
                        })
                        .map_err(error)?
                });
            }
            "undo" | "redo" => {
                let view = self.view(v)?;
                if view.doc != doc {
                    return Err("InvalidView".into());
                }
                receipt = Some(
                    if op == "undo" {
                        self.store.undo_view(view.id, num(v, "epoch")?)
                    } else {
                        self.store.redo_view(view.id, num(v, "epoch")?)
                    }
                    .map_err(error)?,
                );
            }
            "fence" => self
                .store
                .fence(doc, FenceReason::Explicit)
                .map_err(error)?,
            "save_begin" => {
                let name = text(v, "save")?.to_string();
                if self.saves.contains_key(&name) {
                    return Err("duplicate save alias".into());
                }
                let capture = self
                    .store
                    .begin_save(
                        doc,
                        TargetKey(text(v, "target")?.into()),
                        "probe-guard".into(),
                    )
                    .map_err(error)?;
                self.saves.insert(name, capture);
            }
            "save_complete_simulated" | "save_cancel" => {
                let c = self
                    .saves
                    .get(text(v, "save")?)
                    .ok_or("unknown save alias")?
                    .clone();
                if c.snapshot.doc != doc {
                    return Err("InvalidSave".into());
                }
                if op == "save_cancel" {
                    self.store.cancel_save(&c).map_err(error)?;
                } else {
                    self.store
                        .advance_save(&c, SaveStage::WritingTemp)
                        .map_err(error)?;
                    self.store
                        .advance_save(&c, SaveStage::ReadyToReplace)
                        .map_err(error)?;
                    self.store.commit_permit(&c).map_err(error)?;
                    self.store
                        .advance_save(&c, SaveStage::Verifying)
                        .map_err(error)?;
                    self.store.accept_verified_save(&c).map_err(error)?;
                }
            }
            _ => return Err("unknown operation".into()),
        }
        let mut state = self.state(doc)?;
        if let Some(r) = receipt {
            state["receipt"] = receipt_json(&r);
        }
        Ok(state)
    }
}
fn main() {
    let mut probe = Probe::new();
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) => line,
            Err(e) => {
                eprintln!("probe stdin: {e}");
                std::process::exit(2)
            }
        };
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(v) => match probe.request(&v) {
                Ok(value) => json!({"ok":true,"result":value}),
                Err(e) => json!({"ok":false,"error":e}),
            },
            Err(e) => json!({"ok":false,"error":format!("invalid JSON: {e}")}),
        };
        if writeln!(stdout, "{response}")
            .and_then(|_| stdout.flush())
            .is_err()
        {
            eprintln!("probe stdout closed");
            std::process::exit(2);
        }
    }
}
