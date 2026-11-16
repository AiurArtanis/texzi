use super::{Result, delta_json, num, offset, text};
use serde_json::{Value, json};
use std::collections::HashMap;
use texzi_document::*;
fn err(e: ObserverError) -> String {
    format!("{e:?}")
}
fn alias<T: PartialEq + std::fmt::Debug>(map: &HashMap<String, T>, id: &T) -> String {
    map.iter()
        .find(|(_, v)| *v == id)
        .map_or_else(|| format!("{id:?}"), |(n, _)| n.clone())
}
fn cursor_json(
    c: &Cursor,
    docs: &HashMap<String, DocId>,
    views: &HashMap<String, ViewId>,
) -> Value {
    json!({"store":{"session":[format!("{:016x}",c.store.session[0]),format!("{:016x}",c.store.session[1])],"instance":c.store.instance},"doc":alias(docs,&c.doc),"scope":{"schema":c.scope.schema,"views":c.scope.views.iter().map(|id|alias(views,id)).collect::<Vec<_>>()},"seq":c.seq.0})
}
fn scope(v: &Value, views: &HashMap<String, ViewId>) -> Result<ObserverScope> {
    let ids = v
        .get("views")
        .and_then(Value::as_array)
        .ok_or("expected views array")?
        .iter()
        .map(|v| {
            views
                .get(v.as_str().ok_or("expected view alias")?)
                .copied()
                .ok_or("unknown view alias".into())
        })
        .collect::<Result<Vec<_>>>()?;
    let mut scope = ObserverScope::new(ids);
    scope.schema = v
        .get("schema")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .try_into()
        .map_err(|_| "schema overflow")?;
    Ok(scope)
}
fn cursor_from_json(
    v: &Value,
    docs: &HashMap<String, DocId>,
    views: &HashMap<String, ViewId>,
) -> Result<Cursor> {
    let store = v.get("store").ok_or("missing cursor store")?;
    let session = store
        .get("session")
        .and_then(Value::as_array)
        .ok_or("missing nonce")?;
    if session.len() != 2 {
        return Err("invalid nonce length".into());
    }
    let parse = |i: usize| {
        u64::from_str_radix(session[i].as_str().ok_or("nonce must be hex")?, 16)
            .map_err(|_| "invalid nonce".to_string())
    };
    Ok(Cursor {
        store: StoreEpoch {
            session: [parse(0)?, parse(1)?],
            instance: num(store, "instance")?,
        },
        doc: *docs.get(text(v, "doc")?).ok_or("unknown cursor doc")?,
        scope: scope(v.get("scope").ok_or("missing scope")?, views)?,
        seq: EventSeq(num(v, "seq")?),
    })
}
fn body_json(s: &Snapshot) -> Value {
    let mut sources: Vec<std::sync::Arc<str>> = Vec::new();
    let spans=s.body.origins().iter().map(|(span,origin)|{
        let origin=origin.as_ref().map(|o|{let id=if let Some(i)=sources.iter().position(|p|std::sync::Arc::ptr_eq(p,&o.source)){i}else{sources.push(o.source.clone());sources.len()-1};json!({"source":id,"raw_start":o.raw.start,"raw_end":o.raw.end,"logical_len":o.logical_len})});
        json!({"start":span.start,"end":span.end,"origin":origin})
    }).collect::<Vec<_>>();
    json!({"revision":s.revision.0,"token":format!("{:?}",s.token),"source":s.body.source().to_string(),"serialized_utf8":String::from_utf8(s.body.to_utf8()).unwrap(),"eols":s.body.eols().iter().map(|e|format!("{e:?}")).collect::<Vec<_>>(),"profile":{"encoding":"UTF-8","bom":s.body.profile.utf8_bom,"version":s.body.profile.version,"default_eol":format!("{:?}",s.body.profile.default_eol)},"provenance":{"sources":sources.iter().map(|s|s.as_ref()).collect::<Vec<_>>(),"spans":spans}})
}
fn global_json(g: &ObservedGlobal, views: &HashMap<String, ViewId>) -> Value {
    let m = &g.metadata;
    let active=m.save.active.as_ref().map(|a|json!({"operation":a.operation,"revision":a.revision.0,"token":format!("{:?}",a.token),"target":a.target.0,"guard":a.guard,"binding_epoch":a.binding_epoch,"stage":format!("{:?}",a.stage),"cancellable":a.cancellable}));
    let merge=m.history.merge.as_ref().map(|m|json!({"view":alias(views,&m.view),"tick_ms":m.tick_ms,"end":m.end,"token":format!("{:?}",m.token)}));
    json!({"document":body_json(&g.document),"baseline":{"token":format!("{:?}",m.baseline.token),"kind":format!("{:?}",m.baseline.kind),"backed_by_disk":m.baseline.backed_by_disk},"dirty":m.dirty,"attention":{"needs_target":m.attention.needs_target,"binding_epoch":m.attention.binding_epoch,"target":m.attention.target.as_ref().map(|t|&t.0),"known_fingerprint":m.attention.known_fingerprint,"conflict":format!("{:?}",m.attention.conflict),"save_outcome_unknown":format!("{:?}",m.attention.save_outcome_unknown),"display_error":m.attention.display_error},"save":{"active":active,"last_terminal":m.save.last_terminal.as_ref().map(|t|json!({"operation":t.operation,"result":format!("{:?}",t.result),"error":t.error}))},"history":{"can_undo":m.history.can_undo,"can_redo":m.history.can_redo,"undo_groups":m.history.undo_groups,"redo_groups":m.history.redo_groups,"last_undo_group":m.history.last_undo_group.map(|id|id.0),"last_redo_group":m.history.last_redo_group.map(|id|id.0),"merge":merge,"last_fence":format!("{:?}",m.history.last_fence)}})
}
fn view_json(
    v: &ObservedView,
    docs: &HashMap<String, DocId>,
    views: &HashMap<String, ViewId>,
) -> Value {
    match v {
        ObservedView::Closed(v) => {
            json!({"status":"Closed","view":alias(views,&v.id),"doc":alias(docs,&v.doc),"epoch":v.epoch,"at_revision":v.at_revision.0})
        }
        ObservedView::Open {
            state: v,
            folds,
            logical_focus,
            composition,
            pending_mode,
        } => {
            json!({"status":"Open","view":alias(views,&v.id),"doc":alias(docs,&v.doc),"epoch":v.epoch,"mode":format!("{:?}",v.mode),"selection":super::selection_json(&v.selection),"scroll":{"byte":v.scroll.position.offset,"revision":v.scroll.position.revision.0,"affinity":format!("{:?}",v.scroll.affinity),"pixel_bias":v.pixel_bias},"mapping_snaps":v.mapping_snaps.iter().map(|s|json!({"requested":s.requested,"actual":s.actual,"affinity":format!("{:?}",s.affinity)})).collect::<Vec<_>>(),"folds":format!("{folds:?}"),"logical_focus":format!("{logical_focus:?}"),"composition":format!("{composition:?}"),"pending_mode":format!("{pending_mode:?}")})
        }
    }
}
fn state_json(
    s: &ObserverState,
    docs: &HashMap<String, DocId>,
    views: &HashMap<String, ViewId>,
) -> Value {
    json!({"global":global_json(&s.global,views),"views":s.views.values().map(|v|view_json(v,docs,views)).collect::<Vec<_>>()})
}
fn event_json(
    e: &ObserverEvent,
    docs: &HashMap<String, DocId>,
    views: &HashMap<String, ViewId>,
) -> Value {
    match e {
        ObserverEvent::Global(g) => json!({"type":"Global","state":global_json(g,views)}),
        ObserverEvent::View(v) => json!({"type":"View","state":view_json(v,docs,views)}),
        ObserverEvent::Content(c) => {
            json!({"type":"Content","transaction":{"store":{"session":[format!("{:016x}",c.transaction.store.session[0]),format!("{:016x}",c.transaction.store.session[1])],"instance":c.transaction.store.instance},"doc":alias(docs,&c.transaction.doc),"seq":c.transaction.seq.0},"group":c.group.0,"previous":c.previous.0,"revision":c.revision.0,"token":format!("{:?}",c.token),"forward":c.forward.iter().map(delta_json).collect::<Vec<_>>(),"inverse":c.inverse.iter().map(delta_json).collect::<Vec<_>>()})
        }
    }
}
fn frame_json(
    f: &ObserverFrame,
    docs: &HashMap<String, DocId>,
    views: &HashMap<String, ViewId>,
) -> Value {
    let (c, id) = f.identity();
    let mut v = json!({"cursor":cursor_json(c,docs,views),"subscription":id.0});
    match f {
        ObserverFrame::Begin { parts, .. } => {
            v["type"] = json!("Begin");
            v["parts"] = json!(parts);
        }
        ObserverFrame::End { parts, .. } => {
            v["type"] = json!("End");
            v["parts"] = json!(parts);
        }
        ObserverFrame::Part { index, event, .. } => {
            v["type"] = json!("Part");
            v["index"] = json!(index);
            v["event"] = event_json(event, docs, views);
        }
        ObserverFrame::Unknown { .. } => v["type"] = json!("Unknown"),
    };
    v
}
fn lag_json(
    l: &LaggedInfo,
    docs: &HashMap<String, DocId>,
    views: &HashMap<String, ViewId>,
) -> Value {
    json!({"cursor":cursor_json(&l.cursor,docs,views),"subscription":l.subscription.0,"registered_at":l.registered_at.0,"earliest":l.earliest.0,"head":l.head.0,"capacity":l.capacity})
}
fn cache_json(
    c: &ObserverCache,
    docs: &HashMap<String, DocId>,
    views: &HashMap<String, ViewId>,
) -> Value {
    json!({"cursor":cursor_json(c.cursor(),docs,views),"installed":c.is_installed(),"stale":c.is_stale(),"partial":c.is_partial(),"state":c.state().map(|s|state_json(s,docs,views))})
}

#[derive(Default)]
pub(super) struct Wire {
    snapshots: HashMap<String, ObserverSnapshot>,
    handles: HashMap<String, SubscriptionHandle>,
    polls: HashMap<String, ObserverPoll>,
    caches: HashMap<String, ObserverCache>,
}
impl Wire {
    pub(super) fn request(
        &mut self,
        store: &mut Store,
        docs: &HashMap<String, DocId>,
        views: &HashMap<String, ViewId>,
        v: &Value,
    ) -> Result<Value> {
        let op = text(v, "op")?;
        match op {
            "obs_config" => {
                store
                    .set_observer_limits(ObserverLimits {
                        journal_batches: offset(num(v, "journal")?)?,
                        max_queue_batches: offset(num(v, "queue_max")?)?,
                        max_subscriptions: offset(num(v, "subscriptions_max")?)?,
                    })
                    .map_err(err)?;
                Ok(json!({"configured":true}))
            }
            "obs_observe" => {
                let sn = text(v, "snapshot")?.to_string();
                let hn = text(v, "handle")?.to_string();
                if self.snapshots.contains_key(&sn) || self.handles.contains_key(&hn) {
                    return Err("preserve existing snapshot/handle alias".into());
                }
                let doc = *docs.get(text(v, "doc")?).ok_or("unknown doc")?;
                let (s, h) = store
                    .observe_and_subscribe(doc, scope(v, views)?, offset(num(v, "capacity")?)?)
                    .map_err(err)?;
                let result = json!({"cursor":cursor_json(&s.cursor,docs,views),"subscription":s.subscription.0,"capacity":h.capacity,"state":state_json(&s.state,docs,views)});
                self.snapshots.insert(sn, s);
                self.handles.insert(hn, h);
                Ok(result)
            }
            "obs_snapshot" => {
                let s = self
                    .snapshots
                    .get(text(v, "snapshot")?)
                    .ok_or("unknown snapshot")?;
                Ok(
                    json!({"cursor":cursor_json(&s.cursor,docs,views),"subscription":s.subscription.0,"state":state_json(&s.state,docs,views)}),
                )
            }
            "obs_poll" => {
                let h = self
                    .handles
                    .get(text(v, "handle")?)
                    .ok_or("unknown handle")?;
                let name = text(v, "poll")?.to_string();
                if self.polls.contains_key(&name) {
                    return Err("preserve existing poll alias".into());
                }
                let poll = store.poll_observer(h).map_err(err)?;
                let result = match &poll {
                    ObserverPoll::Pending => json!({"type":"Pending"}),
                    ObserverPoll::DocGone {
                        cursor,
                        subscription,
                    } => {
                        json!({"type":"DocGone","cursor":cursor_json(cursor,docs,views),"subscription":subscription.0})
                    }
                    ObserverPoll::Lagged(l) => {
                        json!({"type":"Lagged","info":lag_json(l,docs,views)})
                    }
                    ObserverPoll::Batch(b) => {
                        json!({"type":"Batch","cursor":cursor_json(&b.cursor,docs,views),"subscription":b.subscription.0,"events":b.events.iter().map(|e|event_json(e,docs,views)).collect::<Vec<_>>(),"frames":b.frames().iter().map(|f|frame_json(f,docs,views)).collect::<Vec<_>>()})
                    }
                };
                self.polls.insert(name, poll);
                Ok(result)
            }
            "obs_unsubscribe" => {
                store
                    .unsubscribe(
                        self.handles
                            .get(text(v, "handle")?)
                            .ok_or("unknown handle")?,
                    )
                    .map_err(err)?;
                Ok(json!({"unsubscribed":true}))
            }
            "obs_resume" | "obs_resume_raw" => {
                let name = text(v, "handle")?.to_string();
                if self.handles.contains_key(&name) {
                    return Err("preserve existing handle alias".into());
                }
                let doc = *docs.get(text(v, "doc")?).ok_or("unknown doc")?;
                let cursor = if op == "obs_resume" {
                    self.caches
                        .get(text(v, "cache")?)
                        .ok_or("unknown cache")?
                        .cursor_for_resume()
                        .map_err(err)?
                } else {
                    cursor_from_json(
                        v.get("cursor").ok_or("missing complete cached cursor")?,
                        docs,
                        views,
                    )?
                };
                let h = match store.resume(
                    doc,
                    scope(v, views)?,
                    &cursor,
                    offset(num(v, "capacity")?)?,
                ) {
                    Ok(h) => h,
                    Err(e) => {
                        if op == "obs_resume" {
                            self.caches
                                .get_mut(text(v, "cache")?)
                                .unwrap()
                                .resume_rejected(&e)
                                .map_err(err)?;
                        }
                        return Err(err(e));
                    }
                };
                if op == "obs_resume" {
                    self.caches
                        .get_mut(text(v, "cache")?)
                        .unwrap()
                        .rebind_resume(&h)
                        .map_err(err)?;
                }
                let result = json!({"cursor":cursor_json(&h.baseline,docs,views),"subscription":h.id.0,"capacity":h.capacity});
                self.handles.insert(name, h);
                Ok(result)
            }
            "obs_cache_prepare" => {
                let name = text(v, "cache")?.to_string();
                if self.caches.contains_key(&name) {
                    return Err("preserve existing cache alias".into());
                }
                let s = self
                    .snapshots
                    .get(text(v, "snapshot")?)
                    .ok_or("unknown snapshot")?
                    .clone();
                let c =
                    ObserverCache::prepare(s, offset(num(v, "buffer_frames")?)?).map_err(err)?;
                let result = cache_json(&c, docs, views);
                self.caches.insert(name, c);
                Ok(result)
            }
            "obs_cache_install" => {
                let c = self
                    .caches
                    .get_mut(text(v, "cache")?)
                    .ok_or("unknown cache")?;
                c.install().map_err(err)?;
                Ok(cache_json(c, docs, views))
            }
            "obs_cache_state" => Ok(cache_json(
                self.caches.get(text(v, "cache")?).ok_or("unknown cache")?,
                docs,
                views,
            )),
            "obs_cache_resume_cursor" => {
                let c = self.caches.get(text(v, "cache")?).ok_or("unknown cache")?;
                Ok(cursor_json(
                    &c.cursor_for_resume().map_err(err)?,
                    docs,
                    views,
                ))
            }
            "obs_cache_control" => {
                let p = self.polls.get(text(v, "poll")?).ok_or("unknown poll")?;
                let c = self
                    .caches
                    .get_mut(text(v, "cache")?)
                    .ok_or("unknown cache")?;
                let outcome = match p {
                    ObserverPoll::Lagged(l) => c.receive_lagged(l),
                    ObserverPoll::DocGone {
                        cursor,
                        subscription,
                    } => c.receive_doc_gone(cursor, *subscription),
                    _ => return Err("expected Lagged/DocGone poll".into()),
                }
                .map_err(err)?;
                Ok(json!({"outcome":format!("{outcome:?}"),"cache":cache_json(c,docs,views)}))
            }
            "obs_cache_frame" => {
                let p = self.polls.get(text(v, "poll")?).ok_or("unknown poll")?;
                let ObserverPoll::Batch(b) = p else {
                    return Err("expected Batch".into());
                };
                let mut f = b
                    .frames()
                    .get(offset(num(v, "frame")?)?)
                    .ok_or("frame out of bounds")?
                    .clone();
                let (mut cursor, mut subscription) = {
                    let (c, id) = f.identity();
                    (c.clone(), id)
                };
                if let Some(n) = v.get("seq").and_then(Value::as_u64) {
                    cursor.seq = EventSeq(n);
                }
                if let Some(n) = v.get("subscription").and_then(Value::as_u64) {
                    subscription = SubscriptionId(n);
                }
                if let Some(n) = v.get("schema").and_then(Value::as_u64) {
                    cursor.scope.schema = n.try_into().map_err(|_| "schema overflow")?;
                }
                if let Some(name) = v.get("frame_doc").and_then(Value::as_str) {
                    cursor.doc = *docs.get(name).ok_or("unknown frame doc")?;
                }
                match &mut f {
                    ObserverFrame::Begin {
                        cursor: c,
                        subscription: s,
                        parts,
                    }
                    | ObserverFrame::End {
                        cursor: c,
                        subscription: s,
                        parts,
                    } => {
                        *c = cursor.clone();
                        *s = subscription;
                        if let Some(n) = v.get("parts").and_then(Value::as_u64) {
                            *parts = offset(n)?;
                        }
                    }
                    ObserverFrame::Part {
                        cursor: c,
                        subscription: s,
                        index,
                        ..
                    } => {
                        *c = cursor.clone();
                        *s = subscription;
                        if let Some(n) = v.get("part_index").and_then(Value::as_u64) {
                            *index = offset(n)?;
                        }
                    }
                    ObserverFrame::Unknown { .. } => {}
                }
                if v.get("unknown").and_then(Value::as_bool).unwrap_or(false) {
                    f = ObserverFrame::Unknown {
                        cursor,
                        subscription,
                    };
                }
                let c = self
                    .caches
                    .get_mut(text(v, "cache")?)
                    .ok_or("unknown cache")?;
                let outcome = c.receive(f).map_err(err)?;
                Ok(json!({"outcome":format!("{outcome:?}"),"cache":cache_json(c,docs,views)}))
            }
            "obs_close_document" => {
                let doc = *docs.get(text(v, "doc")?).ok_or("unknown doc")?;
                store.close_document(doc).map_err(super::error)?;
                Ok(json!({"closed":true}))
            }
            _ => Err("unknown observer operation".into()),
        }
    }
}
