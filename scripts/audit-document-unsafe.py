"""Read fixed Git blobs and inventory Rust source/lint boundaries. No rustc/cargo.

This is a deliberately limited lexical/module-layout audit, not a Rust parser,
macro expander, cfg evaluator, build-argument resolver or memory-safety proof.
"""
import argparse
import hashlib
import json
import pathlib
import re
import subprocess

PACKAGE = "crates/texzi-document/"
SRC = PACKAGE + "src/"
TESTS = PACKAGE + "tests/"


def mask_non_code(text):
    """Keep code offsets/newlines, masking comments and ordinary/raw literals."""
    result = list(text)

    def blank(start, end):
        for k in range(start, end):
            if result[k] not in "\r\n":
                result[k] = " "

    i = 0
    while i < len(text):
        if text.startswith("//", i):
            end = text.find("\n", i)
            end = len(text) if end < 0 else end
        elif text.startswith("/*", i):
            end, depth = i + 2, 1
            while depth and end < len(text):
                if text.startswith("/*", end):
                    depth, end = depth + 1, end + 2
                elif text.startswith("*/", end):
                    depth, end = depth - 1, end + 2
                else:
                    end += 1
            if depth:
                raise ValueError("unterminated block comment")
        else:
            raw = re.match(r'(?:b|c)?r(#{0,255})"', text[i:])
            if raw:
                closing = '"' + raw.group(1)
                at = text.find(closing, i + raw.end())
                if at < 0:
                    raise ValueError("unterminated raw literal")
                end = at + len(closing)
            elif text[i] == '"':
                end = i + 1
                while end < len(text):
                    if text[end] == "\\":
                        end += 2
                    elif text[end] == '"':
                        end += 1
                        break
                    else:
                        end += 1
            elif text[i] == "'":
                char = re.match(r"'(?:[^'\\\r\n]|\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F_]+\}|.))'", text[i:])
                if not char:  # A lifetime, not a char literal.
                    i += 1
                    continue
                end = i + char.end()
            elif text.startswith("r#", i):
                ident = re.match(r"r#\w+", text[i:])
                if not ident:
                    i += 1
                    continue
                end = i + ident.end()
            else:
                i += 1
                continue
        blank(i, end)
        i = end
    return "".join(result)


def line_at(text, index):
    return text.count("\n", 0, index) + 1


def attributes(text, code):
    rows = []
    end = -1
    for match in re.finditer(r"#!?\s*\[", code):
        if match.start() < end:
            continue
        end, depth = match.end(), 1
        while depth and end < len(code):
            depth += (code[end] == "[") - (code[end] == "]")
            end += 1
        if depth:
            raise ValueError("unbalanced attribute")
        rows.append({"start": match.start(), "end": end,
                     "line": line_at(text, match.start()),
                     "inner": code[match.start():match.end()].startswith("#!"),
                     "raw": text[match.start():end], "code": code[match.start():end]})
    return rows


def attached(attrs, code, start):
    found = []
    for attr in reversed(attrs):
        if attr["end"] > start:
            continue
        if code[attr["end"]:start].strip():
            break
        found.insert(0, attr)
        start = attr["start"]
    return found


def cfg_items(attrs):
    return [" ".join(a["raw"].split()) for a in attrs
            if re.search(r"\bcfg\s*\(", a["code"])]


def lint_for(attrs, inherited):
    level, origin = inherited
    for attr in attrs:
        match = re.search(r"\b(allow|expect|warn|deny|forbid)\s*\(\s*unsafe_code\b", attr["code"])
        if match:
            new = match.group(1)
            if level == "forbid" and new != "forbid":
                raise ValueError("attempted override under forbid")
            level, origin = new, attr["line"]
    return level, origin


def cfg_ranges(code, attrs):
    """Item-body cfg ranges for current outline-only source layout."""
    ranges = []
    for attr in attrs:
        if not cfg_items([attr]) or attr["inner"]:
            continue
        start = attr["end"]
        tail = code[start:]
        # Skip subsequent attributes on the same item.
        for following in attrs:
            if following["start"] >= start and not code[start:following["start"]].strip():
                start = following["end"]
        tail = code[start:]
        if not re.match(r"\s*(?:pub(?:\([^)]*\))?\s+)?(?:(?:unsafe|async)\s+)?(?:fn|impl|mod|enum|struct|trait)\b", tail):
            continue
        opening = code.find("{", start)
        semicolon = code.find(";", start)
        if opening < 0 or (semicolon >= 0 and semicolon < opening):
            continue
        end, depth = opening + 1, 1
        while depth and end < len(code):
            depth += (code[end] == "{") - (code[end] == "}")
            end += 1
        if depth:
            raise ValueError("unbalanced item body")
        ranges.append((opening, end, cfg_items([attr])[0]))
    return ranges


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=pathlib.Path, required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--json-out", type=pathlib.Path, required=True)
    parser.add_argument("--markdown-out", type=pathlib.Path, required=True)
    args = parser.parse_args()
    if args.json_out.exists() or args.markdown_out.exists():
        raise RuntimeError("preserve existing inventory outputs")

    def git(*cmd):
        return subprocess.check_output(["git", "-C", str(args.repo), *cmd])

    commit = git("rev-parse", args.commit).decode().strip()
    tracked = git("ls-tree", "-r", "--name-only", commit).decode().splitlines()
    rust_paths = [p for p in tracked if p.endswith(".rs") and p.startswith((SRC, TESTS))]
    blobs, info = {}, {}
    for path in rust_paths:
        blob = git("show", commit + ":" + path)
        text = blob.decode("utf-8")
        code = mask_non_code(text)
        attrs = attributes(text, code)
        blobs[path] = blob
        info[path] = {"text": text, "code": code, "attrs": attrs,
                      "ranges": cfg_ranges(code, attrs)}

    roots = [("lib", SRC + "lib.rs", "texzi_document", [])]
    roots += [("binary-layout", p, "bin::" + pathlib.PurePosixPath(p).stem, ["feature=probe (manifest)"])
              for p in rust_paths if p.startswith(SRC + "bin/") and p.count("/") == (SRC + "bin/").count("/")]
    roots += [("integration-layout", p, "test::" + pathlib.PurePosixPath(p).stem, [])
              for p in rust_paths if p.startswith(TESTS) and p.count("/") == TESTS.count("/")]
    nodes, edges, sites = [], [], []

    def walk(path, kind, module, inherited, cfg, root=False):
        item = info[path]
        inner = [a for a in item["attrs"] if a["inner"]]
        lint = lint_for(inner, inherited)
        cfg = list(dict.fromkeys(cfg + cfg_items(inner)))
        node = {"crate_kind": kind, "module": module, "path": path,
                "effective_source_lint": lint[0], "cfg_not_evaluated": cfg}
        nodes.append(node)
        for match in re.finditer(r"\bunsafe\b", item["code"]):
            next_token = re.match(r"\s*(\{|[A-Za-z_]\w*)", item["code"][match.end():])
            local = [raw for start, end, raw in item["ranges"] if start < match.start() < end]
            conditions = list(dict.fromkeys(cfg + local))
            sites.append({**node, "line": line_at(item["text"], match.start()),
                          "kind": "block" if next_token and next_token.group(1) == "{" else "other-unsafe-keyword",
                          "cfg_not_evaluated": conditions,
                          "test_source": any(re.search(r"\btest\b", value) for value in conditions)})
        for match in re.finditer(r"(?:pub(?:\([^)]*\))?\s+)?\bmod\s+([A-Za-z_]\w*)\s*([;{])", item["code"]):
            if match.group(2) != ";":
                raise ValueError("inline modules require explicit audit support")
            attrs = attached(item["attrs"], item["code"], match.start())
            explicit = next((re.search(r'\bpath\s*=\s*"([^"]+)"', a["raw"]) for a in attrs
                             if re.search(r"\bpath\s*=", a["code"])), None)
            parent = pathlib.PurePosixPath(path).parent
            if explicit:
                candidates = [str(parent / explicit.group(1))]
            else:
                base = parent if root or path.endswith("/mod.rs") else parent / pathlib.PurePosixPath(path).stem
                candidates = [str(base / (match.group(1) + ".rs")), str(base / match.group(1) / "mod.rs")]
            resolved = [p for p in candidates if p in info]
            if len(resolved) != 1:
                raise ValueError({"module": module, "candidates": candidates})
            child = module + "::" + match.group(1)
            edges.append({"from": module, "to": child, "declaration_path": path,
                          "line": line_at(item["text"], match.start()), "resolved_path": resolved[0],
                          "attributes": [" ".join(a["raw"].split()) for a in attrs]})
            walk(resolved[0], kind, child, lint_for(attrs, lint), cfg + cfg_items(attrs))

    for kind, path, module, cfg in roots:
        walk(path, kind, module, ("rust-default/not-inherited-from-lib", None), cfg, root=True)
    lint_attrs, path_entries, includes = [], [], []
    for path, item in info.items():
        for attr in item["attrs"]:
            if re.search(r"\b(?:allow|expect|warn|deny|forbid|cfg_attr)\s*\(", attr["code"]):
                lint_attrs.append({"path": path, "line": attr["line"], "attribute": " ".join(attr["raw"].split())})
            if re.search(r"\bpath\s*=", attr["code"]):
                path_entries.append({"path": path, "line": attr["line"], "attribute": " ".join(attr["raw"].split())})
        for match in re.finditer(r"\b(include|include_str|include_bytes)\s*!\s*\(", item["code"]):
            includes.append({"path": path, "line": line_at(item["text"], match.start()),
                             "kind": match.group(1), "role": "code" if match.group(1) == "include" else "data"})
    config_paths = [p for p in tracked if p.endswith("Cargo.toml") or p.startswith(".cargo/")
                    or p.startswith(".github/workflows/") or (p.startswith("scripts/") and p.endswith((".ps1", ".py", ".sh")))]
    flag_mentions = []
    for path in config_paths:
        for line, value in enumerate(git("show", commit + ":" + path).decode("utf-8").splitlines(), 1):
            tokens = re.findall(r"\b(?:[A-Z_]*RUSTFLAGS|RUSTDOCFLAGS|RUSTC_BOOTSTRAP)\b|--cap-lints|-[AWDF]\s*(?:unsafe[-_]code|warnings|all)\b", value)
            if tokens:
                flag_mentions.append({"path": path, "line": line, "tokens": tokens})
    assert set(rust_paths) == {n["path"] for n in nodes}, "unmapped Rust source"
    lib_sites = [s for s in sites if s["crate_kind"] == "lib"]
    assert all(s["effective_source_lint"] == "allow" for s in lib_sites)
    result = {"scope": "static fixed-source lexical/module-layout inventory; NOT a compiler or memory-safety proof",
              "product_commit": commit, "source_files": [{"path": p, "bytes": len(blobs[p]),
              "sha256": hashlib.sha256(blobs[p]).hexdigest().upper()} for p in rust_paths],
              "crate_roots": [{"kind": k, "path": p, "module": m} for k, p, m, _ in roots],
              "module_nodes": nodes, "module_edges": edges, "unsafe_sites": sites,
              "lint_attributes": lint_attrs, "path_attributes": path_entries,
              "include_entries": includes, "configuration_scan_paths": config_paths,
              "configuration_flag_mentions": flag_mentions,
              "counts": {"rust_files": len(rust_paths), "module_contexts": len(nodes),
                         "lib_files": sum(n["crate_kind"] == "lib" for n in nodes),
                         "unsafe_sites": len(sites), "lib_test_unsafe": sum(s["test_source"] for s in lib_sites),
                         "lib_non_test_unsafe": sum(not s["test_source"] for s in lib_sites),
                         "lint_attributes": len(lint_attrs)},
              "limitations": ["No cfg evaluation, macro expansion, dependency expansion or Cargo metadata execution.",
                              "Current outline-module layout only; no general Rust parsing claim.",
                              "Independent binary/test roots do not inherit lib lint; include_str is data.",
                              "Tracked config text only; no global/ambient compiler argument guarantee."],
              "typecheck": "NOT_RUN", "build": "NOT_RUN", "runtime": "NOT_RUN"}
    for path, content in [(args.json_out, json.dumps(result, ensure_ascii=False, indent=2) + "\n"),
                          (args.markdown_out, markdown(result, args.json_out))]:
        path.parent.mkdir(parents=True, exist_ok=True)
        with path.open("x", encoding="utf-8", newline="\n") as stream:
            stream.write(content)
    print(json.dumps({"product_commit": commit, **result["counts"], "typecheck": "NOT_RUN"}))


def markdown(data, json_path):
    c = data["counts"]
    lines = ["# T07 unsafe 模块边界静态清单", "",
             f"产品源码固定 `{data['product_commit']}`；本次只增加清单/工具，不改 Rust、Cargo target/feature 或编译 flags。",
             "类型检查、build/smoke、全部组件/IO/ACL/应用运行均 **NOT_RUN**。词法和模块布局枚举不等于 Rust 编译器语义或内存安全证明。", "",
             f"共 {c['rust_files']} 个 Rust 文件、{c['module_contexts']} 个 crate/module 归属，库树 {c['lib_files']} 文件；实际 unsafe 关键字 {c['unsafe_sites']} 处，库中 Windows 非 test {c['lib_non_test_unsafe']}、test {c['lib_test_unsafe']}。", "",
             "## lint 和例外", "",
             "根 deny；十个纯内核声明和 staging 为 forbid。三个 cfg(windows) allow 覆盖整个子树及嵌套测试，不是三个函数。files 其它代码继续受 deny；allow 只表示 lint 许可，每块 SAFETY 不变量仍须审查。", "",
             "| 声明位置 | 属性 |", "| --- | --- |"]
    for item in data["lint_attributes"]:
        lines.append(f"| `{item['path']}:{item['line']}` | `{item['attribute']}` |")
    lines += ["", "三个例外的源码共享原因在 files.rs 注释；没有伪造 reason 属性。具体原因：windows_identity 封装活句柄/缓冲区的 FileId/GUID 查询；windows_save 封装原句柄 rename 及隔离 fixture 的 DuplicateHandle；windows_security 封装 descriptor/SID/DACL 的原生内存和 handle API。", "",
              "## 全部实际 unsafe 位置", "", "以下位置从掩去注释/字符串后的固定源码枚举；cfg 只记录、不求值。", "",
              "| 路径 | 非 test 行号 | test 行号 |", "| --- | --- | --- |"]
    for path in sorted({s["path"] for s in data["unsafe_sites"]}):
        groups = [[str(s["line"]) for s in data["unsafe_sites"] if s["path"] == path and s["test_source"] == test] for test in [False, True]]
        lines.append(f"| `{path}` | {', '.join(groups[0]) or '无'} | {', '.join(groups[1]) or '无'} |")
    lines += ["", "## 模块子树与独立 crate", "", "| crate 类别 | 模块 | 源文件 | 继承的源码 lint |", "| --- | --- | --- | --- |"]
    for node in data["module_nodes"]:
        lines.append(f"| {node['crate_kind']} | `{node['module']}` | `{node['path']}` | {node['effective_source_lint']} |")
    lines += ["", "document-probe 的两个 path 子模块属于独立 bin；contracts.rs 的模块属于独立 integration-test。native_plans.rs 同时是 contracts 子模块及自动发现的独立 test 根候选，两种情况都不继承 lib lint。布局来自 manifest/固定目录，未执行 Cargo metadata 验证。", "",
              "## path/include 与降级入口", "", "| 位置 | 入口/类别 |", "| --- | --- |"]
    for item in data["path_attributes"]:
        lines.append(f"| `{item['path']}:{item['line']}` | `{item['attribute']}` |")
    for item in data["include_entries"]:
        lines.append(f"| `{item['path']}:{item['line']}` | `{item['kind']}!`，{item['role']} |")
    lines += ["", "完整属性、每个 unsafe 的 module/cfg、固定字节 SHA256、扫描过的配置和 flags 词法命中都在配套 JSON。没有修改 RUSTFLAGS、cap-lints、feature、target 或依赖。配置文本命中不是实际执行参数；用户/全局 Cargo 配置与 ambient 环境不由本源码清单证明。", "",
              "本范围仅 texzi-document 库树和该包独立入口的对照，不证明 texzi-app、workspace、依赖或宏展开全部无 unsafe；当前独立入口未见 unsafe 也不等于获得库 forbid。", "",
              f"机器清单：`{json_path.as_posix()}`。所有旧失败/接受及未跟踪文件保留，不构建/运行、不重试受限通道。", ""]
    return "\n".join(lines)


if __name__ == "__main__":
    main()
