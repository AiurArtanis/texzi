//! Application-owned image presentation. Loading/selection is local viewing
//! state; document image attributes remain untouched in the shared Store.
use nsh_base::rich_editor::{
    model::Node,
    state::NodeSelection,
    view::{EditorSession, NodeView, NodeViewCx},
};
use nsh_core::prelude::*;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct ImagePlan {
    original: String,
    resolved: Option<String>,
    alt: String,
    remote: bool,
    notice: String,
}
fn decoded_path(raw: &str) -> String {
    let mut bytes = Vec::new();
    let input = raw.as_bytes();
    let mut at = 0;
    while at < input.len() {
        if input[at] == b'%'
            && at + 2 < input.len()
            && input[at + 1].is_ascii_hexdigit()
            && input[at + 2].is_ascii_hexdigit()
        {
            if let Ok(value) = u8::from_str_radix(&raw[at + 1..at + 3], 16) {
                bytes.push(value);
                at += 3;
                continue;
            }
        }
        bytes.push(input[at]);
        at += 1;
    }
    String::from_utf8(bytes).unwrap_or_else(|_| raw.into())
}
fn plan(src: &str, alt: &str, document: Option<&Path>) -> ImagePlan {
    let lower = src.to_ascii_lowercase();
    let file_authority = lower
        .strip_prefix("file://")
        .and_then(|tail| tail.split(['/', '\\']).next())
        .is_some_and(|host| !host.is_empty() && host != "localhost");
    let remote = lower.starts_with("https://")
        || lower.starts_with("http://")
        || file_authority
        || src.starts_with("//")
        || src.starts_with("\\\\");
    let (resolved, notice) = if src.is_empty() {
        (None, "图片地址为空，可选择图片后编辑属性。".into())
    } else if remote {
        (
            Some(if src.starts_with("//") {
                format!("https:{src}")
            } else {
                src.into()
            }),
            "远程图片尚未加载。".into(),
        )
    } else if lower.starts_with("file:") || lower.starts_with("data:") {
        (Some(src.into()), "正在加载图片…".into())
    } else {
        let path = PathBuf::from(decoded_path(src));
        if path.is_absolute() {
            (
                Some(path.to_string_lossy().into_owned()),
                "正在加载图片…".into(),
            )
        } else if src.contains("://")
            || lower.starts_with("blob:")
            || lower.starts_with("javascript:")
        {
            (None, "暂不支持此图片地址，可选择图片后编辑属性。".into())
        } else if let Some(parent) = document
            .and_then(Path::parent)
            .filter(|p| !p.as_os_str().is_empty())
        {
            (
                Some(parent.join(path).to_string_lossy().into_owned()),
                "正在加载图片…".into(),
            )
        } else {
            (
                None,
                "未保存文档无法定位相对图片，请先保存文档或修改图片地址。".into(),
            )
        }
    };
    ImagePlan {
        original: src.into(),
        resolved,
        alt: if alt.trim().is_empty() {
            "（未填写替代文本）".into()
        } else {
            alt.into()
        },
        remote,
        notice,
    }
}
fn preview_size(width: f32, height: f32) -> (f32, f32) {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return (240.0, 160.0);
    }
    let scale = (300.0 / width).min(180.0 / height).min(1.0);
    ((width * scale).max(16.0), (height * scale).max(16.0))
}

pub fn select_image(session: &EditorSession, pos: usize) -> Result<(), String> {
    if session.composing() {
        return Err("请先完成或取消中文组合，再选择图片".into());
    }
    let state = session.state();
    if !state
        .doc()
        .node_at(pos)
        .is_some_and(|node| node.ty().name() == "image")
    {
        return Err("图片位置已变化，请重新选择".into());
    }
    let mut tr = state.tr();
    tr.set_selection(NodeSelection::create(state.doc(), pos));
    session.dispatch(tr).map_err(|e| e.to_string())
}

struct ImageNodeView {
    document: Option<PathBuf>,
}
impl NodeView for ImageNodeView {
    fn create(&self, cx: &NodeViewCx, node: &Node) -> AnyElement {
        let attr = |name: &str| {
            node.attrs()
                .get(name)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
        };
        let display = plan(attr("src"), attr("alt"), self.document.as_deref());
        let requested = signal(!display.remote && display.resolved.is_some());
        let failed = signal(false);
        let generation = signal(0u64);
        let notice = signal(display.notice.clone());
        let size = signal((240.0, 160.0));
        let source = display.resolved.clone();
        let accessible = display.alt.clone();
        let can_load = display.resolved.is_some();
        let get_pos = cx.get_pos();
        let session = cx.session.clone();
        let load_label = if display.remote {
            "加载远程图片"
        } else {
            "重试加载"
        };
        view()
            .w(px(320.0))
            .max_w(relative(1.0))
            .min_h(px(76.0))
            .p(px(8.0))
            .gap(px(6.0))
            .bg(crate::app_shell::background)
            .text_color(crate::app_shell::foreground)
            .child(text(format!("图片 · {}", display.alt)))
            .child(text(format!(
                "地址：{}",
                if display.original.is_empty() {
                    "（空）"
                } else {
                    &display.original
                }
            )))
            .child(dynamic(move || {
                if !requested.get() {
                    return None;
                }
                let src = source.clone()?;
                let alt = accessible.clone();
                let key = generation.get();
                Some(
                    image(src)
                        .key(format!("preview-{key}"))
                        .object_fit(ResizeMode::Contain)
                        .label(alt)
                        .w(move || px(size.get().0))
                        .h(move || px(size.get().1))
                        .on_load(move |actual: &Size<DevicePixels>| {
                            size.set(preview_size(actual.width.0 as f32, actual.height.0 as f32));
                            failed.set(false);
                            notice.set(String::new());
                        })
                        .on_error(move |error: &str| {
                            failed.set(true);
                            notice.set(format!("图片暂不能显示：{error}"));
                        })
                        .into_element(),
                )
            }))
            .child(text(move || notice.get()))
            .child(when(
                move || can_load && (!requested.get() || failed.get()),
                move || {
                    nsh_base::button()
                        .child(text(load_label))
                        .on_press(move |_: &()| {
                            failed.set(false);
                            notice.set("正在加载图片…".into());
                            generation.update(|v| *v += 1);
                            requested.set(true);
                        })
                },
            ))
            .child(
                nsh_base::button()
                    .child(text("选择图片以编辑属性"))
                    .on_press(move |_: &()| {
                        let result = get_pos()
                            .ok_or_else(|| "图片位置已变化，请重新选择".to_owned())
                            .and_then(|pos| select_image(&session, pos));
                        if let Err(error) = result {
                            notice.set(error);
                        }
                    }),
            )
            .into_element()
    }
}

/// Call before mounting the RichEditor, using the document's actual file path.
/// Other application node views (for example literal source blocks) are kept.
pub fn install(session: &EditorSession, document_path: Option<PathBuf>) {
    let mut views = session.node_views();
    views.insert(
        "image",
        Arc::new(ImageNodeView {
            document: document_path,
        }),
    );
    session.set_node_views(views);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{editor_owner::NativeView, rich_owner::ModeGate, rich_projection::Projection};
    use nsh_base::editor::ExternalDocument;
    use nsh_base::rich_editor::state::{EditorState, EditorStateConfig};
    use nsh_base::rich_editor::view::NodeViews;

    fn run(f: impl FnOnce()) {
        let runtime = nsh_core::element::Runtime::default();
        let _entered = runtime.enter();
        f();
    }
    #[test]
    fn image_plan_preserves_original_address_and_never_auto_loads_remote() {
        let remote = plan("https://example.invalid/missing.png", "远程替代文本", None);
        assert!(remote.remote);
        assert_eq!(remote.alt, "远程替代文本");
        assert!(remote.notice.contains("尚未加载"));
        assert_eq!(remote.original, "https://example.invalid/missing.png");
        let share = plan("file://server/share/image.png", "网络图片", None);
        assert!(share.remote);
        assert!(share.notice.contains("尚未加载"));
        assert!(!plan("file:///D:/fixtures/image.png", "local", None).remote);
        assert!(!plan("file://localhost/D:/fixtures/image.png", "local", None).remote);
        let local = plan(
            "./images/my%20photo.png",
            "本地替代文本",
            Some(Path::new("D:/notes/article.md")),
        );
        assert!(!local.remote);
        assert_eq!(
            PathBuf::from(local.resolved.unwrap()),
            Path::new("D:/notes").join("./images/my photo.png")
        );
        assert_eq!(local.original, "./images/my%20photo.png");
        let unsaved = plan("./missing.png", "", None);
        assert!(unsaved.resolved.is_none());
        assert!(!unsaved.alt.is_empty());
        assert!(!unsaved.notice.is_empty());
    }
    #[test]
    fn missing_or_invalid_image_sizes_always_keep_nonzero_preview_and_caption() {
        for (w, h) in [
            (0.0, 0.0),
            (f32::NAN, 5.0),
            (-1.0, 20.0),
            (4000.0, 2000.0),
            (1.0, 1.0),
        ] {
            let size = preview_size(w, h);
            assert!(size.0 >= 16.0 && size.1 >= 16.0);
            assert!(size.0 <= 300.0 && size.1 <= 180.0);
        }
        let failed = plan("", "原始替代文本", None);
        assert_eq!(failed.alt, "原始替代文本");
        assert!(!failed.notice.is_empty());
    }
    #[test]
    fn installing_image_node_view_preserves_other_node_views_and_document() {
        struct Other;
        impl NodeView for Other {
            fn create(&self, _: &NodeViewCx, _: &Node) -> AnyElement {
                text("other").into_element()
            }
        }
        run(|| {
            let manager = crate::rich_projection::manager();
            let projection =
                Projection::parse(manager.schema(), "![alt](./photo.png)".into()).unwrap();
            let state = EditorState::create(
                EditorStateConfig::new(manager.schema().clone()).doc(projection.doc.clone()),
            )
            .unwrap();
            let session =
                EditorSession::new(state, NodeViews::new().with("codeBlock", Arc::new(Other)));
            let before = session.state().doc().clone();
            install(&session, Some(PathBuf::from("D:/notes/article.md")));
            assert!(session.node_views().get("image").is_some());
            assert!(session.node_views().get("codeBlock").is_some());
            assert_eq!(session.state().doc(), &before);
        });
    }
    #[test]
    fn selecting_image_uses_owner_selection_without_changing_source_or_history() {
        run(|| {
            let raw = "before\r\n\r\n![alt](./missing.png)\n";
            let source = NativeView::fixture(raw);
            let gate = ModeGate::new(source.clone(), source.sibling().unwrap());
            gate.to_rich();
            assert!(gate.rich_mode.get());
            let before = source.inspect(|s, id| {
                let d = s.view(id).unwrap().doc;
                (
                    s.snapshot(d).unwrap().token,
                    s.history_depth(d).unwrap(),
                    s.dirty(d).unwrap(),
                )
            });
            let session = gate.session();
            let logical = source.snapshot().unwrap().rope.to_string();
            let p = Projection::parse(session.state().schema(), logical.clone()).unwrap();
            let pos = p.rich_position(logical.find("![alt]").unwrap()).unwrap();
            select_image(&session, pos).unwrap();
            assert!(gate.status.get().is_empty(), "{}", gate.status.get());
            assert_eq!(session.state().selection().from(), pos);
            assert_eq!(
                before,
                source.inspect(|s, id| {
                    let d = s.view(id).unwrap().doc;
                    (
                        s.snapshot(d).unwrap().token,
                        s.history_depth(d).unwrap(),
                        s.dirty(d).unwrap(),
                    )
                })
            );
            assert_eq!(
                source.inspect(|s, id| s.snapshot(s.view(id).unwrap().doc).unwrap().body.to_utf8()),
                raw.as_bytes()
            );
        });
    }
}
