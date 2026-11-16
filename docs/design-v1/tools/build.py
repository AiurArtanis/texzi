#!/usr/bin/env python3
"""Build the dependency-free texzi prototype. Python 3.10+; no third-party modules."""
from pathlib import Path
import json

ROOT = Path(__file__).resolve().parents[1]

def build_prototype():
    src = ROOT / "prototype" / "src"
    settings = json.loads((ROOT / "data" / "settings.json").read_text(encoding="utf-8"))
    commands_path = ROOT / "data" / "commands.json"
    commands = json.loads(commands_path.read_text(encoding="utf-8")) if commands_path.exists() else {"commands": []}
    data_js = "window.TEXZI_SETTINGS=" + json.dumps(settings, ensure_ascii=False).replace("</", "<\\/") + ";\n"
    data_js += "window.TEXZI_COMMANDS=" + json.dumps(commands, ensure_ascii=False).replace("</", "<\\/") + ";\n"
    html = (src / "index.html").read_text(encoding="utf-8")
    css = (src / "app.css").read_text(encoding="utf-8")
    js = (src / "app.js").read_text(encoding="utf-8")
    html = html.replace('<link rel="stylesheet" href="app.css">', "<style>\n" + css + "\n</style>")
    html = html.replace('<script src="app.js"></script>', "<script>\n" + data_js + js + "\n</script>")
    target = ROOT / "prototype" / "index.html"
    target.write_text(html, encoding="utf-8")
    print(f"Built {target.name}: {target.stat().st_size:,} bytes, no network dependencies")
    return target

if __name__ == "__main__":
    build_prototype()
