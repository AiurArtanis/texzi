use nsh_core::prelude::*;
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

pub const THEME: &str = "appearance.theme";
pub const FONT_FAMILY: &str = "editor.fontFamily";
pub const FONT_SIZE: &str = "editor.fontSize";
pub const LINE_NUMBERS: &str = "editor.lineNumbers";
pub const TAB_SIZE: &str = "editor.tabSize";
pub const WRAP_TEXT: &str = "editor.wordWrap.text";
pub const WRAP_MARKDOWN: &str = "editor.wordWrap.markdown";
pub const WRAP_CODE: &str = "editor.wordWrap.code";
pub const DEFAULT_ENCODING: &str = "files.defaultEncoding";
pub const DEFAULT_EOL: &str = "files.defaultEol";
pub const MARKDOWN_VIEW: &str = "markdown.defaultView";
pub const TAB_CLOSE: &str = "workbench.tabCloseButton";
pub const MARKDOWN_SOURCE_ONLY_BYTES: u64 = 50_000_000;

pub const ENABLED: &[&str] = &[
    THEME,
    FONT_FAMILY,
    FONT_SIZE,
    WRAP_TEXT,
    WRAP_MARKDOWN,
    WRAP_CODE,
    LINE_NUMBERS,
    TAB_SIZE,
    DEFAULT_ENCODING,
    DEFAULT_EOL,
    MARKDOWN_VIEW,
    TAB_CLOSE,
];

pub fn definition(id: &str) -> &'static Value {
    static DEFINITIONS: OnceLock<Value> = OnceLock::new();
    let registry = DEFINITIONS.get_or_init(|| {
        serde_json::from_str(include_str!("../../../docs/design-v1/data/settings.json"))
            .expect("bundled settings registry")
    });
    registry["settings"]
        .as_array()
        .expect("settings list")
        .iter()
        .find(|setting| setting["id"] == id)
        .expect("implemented setting is in the registry")
}

pub fn folder_supported(id: &str) -> bool {
    definition(id)["scope"]
        .as_array()
        .is_some_and(|scopes| scopes.iter().any(|scope| scope == "workspaceLocal"))
}

fn validate(id: &str, value: &Value) -> bool {
    let spec = definition(id);
    match spec["type"].as_str() {
        Some("integer") => value.as_i64().is_some_and(|n| {
            n >= spec["min"].as_i64().unwrap() && n <= spec["max"].as_i64().unwrap()
        }),
        Some("boolean") => value.is_boolean(),
        Some("string") => value.as_str().is_some_and(|value| {
            value
                .split(',')
                .all(|family| !family.trim().is_empty() && !family.chars().any(char::is_control))
        }),
        Some("enum") => spec["options"]
            .as_array()
            .is_some_and(|options| options.iter().any(|option| &option["value"] == value)),
        _ => false,
    }
}

pub fn new_document_format(
    encoding: &str,
    eol: &str,
) -> (texzi_document::files::Format, texzi_document::Eol) {
    use texzi_document::{
        Eol,
        files::{Encoding, Format},
    };
    let (encoding, bom) = match encoding {
        "utf-8-bom" => (Encoding::Utf8, true),
        "utf-16le" => (Encoding::Utf16Le, true),
        "utf-16be" => (Encoding::Utf16Be, true),
        "gbk" => (Encoding::GbkWebEncodingRs0844, false),
        "gb18030" => (Encoding::Gb18030WebEncodingRs0844, false),
        "windows-1252" => (Encoding::Windows1252, false),
        _ => (Encoding::Utf8, false),
    };
    let eol = if eol == "LF" { Eol::Lf } else { Eol::CrLf };
    (Format { encoding, bom }, eol)
}

pub fn source_font_stack(value: &str) -> String {
    if value.eq_ignore_ascii_case("system") {
        "system-ui".into()
    } else {
        format!("{value}, Consolas, system-ui")
    }
}

fn empty() -> Value {
    json!({"version": 1, "user": {}, "folders": {}})
}

fn read(path: &Path) -> io::Result<Value> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(empty()),
        Err(error) => return Err(error),
    };
    let data: Value = serde_json::from_slice(&bytes)?;
    if !data.is_object()
        || ["user", "folders"]
            .iter()
            .any(|key| data.get(key).is_some_and(|entry| !entry.is_object()))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "设置文件结构无效",
        ));
    }
    Ok(data)
}

fn folder_key(path: &Path) -> String {
    use std::os::windows::ffi::OsStrExt;
    match path.to_str() {
        Some(text) => text.to_owned(),
        None => path
            .as_os_str()
            .encode_wide()
            .fold(String::from("utf16:"), |mut key, unit| {
                use std::fmt::Write as _;
                write!(key, "{unit:04x}").unwrap();
                key
            }),
    }
}

fn effective(data: &Value, id: &str, root: Option<&Path>) -> (Value, &'static str) {
    if folder_supported(id)
        && let Some(root) = root
    {
        let selected = &data["folders"][folder_key(root)][id];
        if !selected.is_null() && validate(id, selected) {
            return (selected.clone(), "当前文件夹");
        }
    }
    let selected = &data["user"][id];
    if !selected.is_null() && validate(id, selected) {
        return (selected.clone(), "用户");
    }
    (definition(id)["default"].clone(), "默认")
}

fn change(data: &mut Value, id: &str, root: Option<&Path>, value: Option<Value>) -> io::Result<()> {
    let scope = match root {
        Some(root) => &mut data["folders"][folder_key(root)],
        None => &mut data["user"],
    };
    if let Some(value) = value {
        if scope.is_null() {
            *scope = json!({});
        }
        let object = scope
            .as_object_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "该范围的设置结构无效"))?;
        object.insert(id.into(), value);
    } else if let Some(object) = scope.as_object_mut() {
        object.remove(id);
    }
    Ok(())
}

fn write(path: &Path, id: &str, root: Option<&Path>, value: Option<Value>) -> io::Result<Value> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = path.parent().expect("settings path has a parent");
    fs::create_dir_all(directory)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("settings.lock"))?;
    lock.lock()?;
    let mut data = read(path)?;
    change(&mut data, id, root, value)?;
    let temporary = directory.join(format!(
        "settings-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let write_result = (|| {
        serde_json::to_writer_pretty(&mut file, &data)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_preferences_keep_valid_lower_scopes_and_their_defaults() {
        let root = Path::new(r"C:\work");
        let mut data = empty();
        change(&mut data, FONT_SIZE, None, Some(json!(18))).unwrap();
        change(&mut data, FONT_SIZE, Some(root), Some(json!(22))).unwrap();
        assert_eq!(
            effective(&data, FONT_SIZE, Some(root)),
            (json!(22), "当前文件夹")
        );
        change(&mut data, FONT_SIZE, Some(root), Some(json!(37))).unwrap();
        assert_eq!(effective(&data, FONT_SIZE, Some(root)), (json!(18), "用户"));
        assert_eq!(effective(&data, TAB_SIZE, Some(root)), (json!(4), "默认"));
        assert_eq!(effective(&data, WRAP_MARKDOWN, None).0, json!("on"));
    }

    #[test]
    fn separate_writes_merge_without_dropping_unrecognized_fields() {
        let directory =
            std::env::temp_dir().join(format!("texzi-settings-test-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("settings.json");
        fs::write(&path, br#"{"user":{"future.preference":1},"folders":{}}"#).unwrap();
        write(&path, THEME, None, Some(json!("dark"))).unwrap();
        write(&path, FONT_SIZE, None, Some(json!(19))).unwrap();
        let data = write(&path, THEME, None, None).unwrap();
        assert_eq!(
            data["user"],
            json!({"future.preference": 1, "editor.fontSize": 19})
        );
        fs::remove_file(path).unwrap();
        fs::remove_file(directory.join("settings.lock")).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}

pub struct Settings {
    path: Option<PathBuf>,
    data: Signal<Value>,
    pub error: Signal<Option<String>>,
}

impl Settings {
    pub fn new() -> Self {
        let path = if cfg!(test) {
            None
        } else {
            std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|path| path.join("texzi").join("settings.json"))
        };
        let settings = Self {
            path,
            data: signal(empty()),
            error: signal(None),
        };
        settings.refresh();
        settings
    }

    pub fn refresh(&self) {
        if let Some(path) = &self.path {
            match read(path) {
                Ok(data) => {
                    self.data.set(data);
                    self.error.set(None);
                }
                Err(error) => self.error.set(Some(format!("无法读取设置：{error}"))),
            }
        }
    }

    pub fn effective(&self, id: &str, root: Option<&Path>) -> (Value, &'static str) {
        self.data.with(|data| effective(data, id, root))
    }

    pub fn apply(&self, id: &str, root: Option<&Path>, value: Option<Value>) -> bool {
        let spec = definition(id);
        let name = spec["name"].as_str().unwrap();
        if root.is_some() && !folder_supported(id) {
            self.error.set(Some(format!("{name}只支持用户范围")));
            return false;
        }
        if value.as_ref().is_some_and(|value| !validate(id, value)) {
            let instruction = match spec["type"].as_str() {
                Some("integer") => format!("请输入 {}–{} 之间的整数", spec["min"], spec["max"]),
                Some("string") => "请输入字体名称、以逗号分隔的字体列表或 system".into(),
                _ => "请选择支持的值".into(),
            };
            self.error.set(Some(format!("{name}：{instruction}")));
            return false;
        }
        let result = match &self.path {
            Some(path) => write(path, id, root, value),
            None if cfg!(test) => {
                let mut data = self.data.get();
                change(&mut data, id, root, value).map(|()| data)
            }
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "未找到本机应用数据目录",
            )),
        };
        match result {
            Ok(data) => {
                self.data.set(data);
                self.error.set(None);
                true
            }
            Err(error) => {
                self.error
                    .set(Some(format!("设置未保存：{error}。请重试。")));
                false
            }
        }
    }
}
