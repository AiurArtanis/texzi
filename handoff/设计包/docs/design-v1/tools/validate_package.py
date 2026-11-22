#!/usr/bin/env python3
"""Validate delivered documentation/data integrity; does not run native or UI tests."""
from pathlib import Path
import ast
import json
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
errors = []
required = ["README.md", "docs/PRD.md", "docs/UI_SPEC.md", "docs/COMMANDS.md",
            "docs/SETTINGS.md", "docs/TECHNICAL_DESIGN.md", "docs/PERFORMANCE_PLAN.md",
            "docs/HANDOFF.md", "docs/ACCEPTANCE.md", "docs/DECISIONS.md",
            "docs/PROTOTYPE_GUIDE.md", "docs/VERIFICATION.md", "docs/SOURCES.md", "docs/REFERENCE_RESEARCH.md",
            "prototype/index.html", "texzi-design-handbook.html",
            "data/commands.json", "data/settings.json", "data/acceptance.json",
            "tools/bench/generate_fixtures.py", "tests/core.test.cjs", "tests/browser_smoke.py"]
for name in required:
    if not (ROOT / name).is_file():
        errors.append(f"Missing: {name}")
for file in ROOT.rglob("*.json"):
    try:
        json.loads(file.read_text(encoding="utf-8"))
    except Exception as exc:
        errors.append(f"JSON: {file.relative_to(ROOT)}: {exc}")
for name, key, expected in [("commands", "commands", 115), ("settings", "settings", 63)]:
    data = json.loads((ROOT / "data" / (name + ".json")).read_text(encoding="utf-8"))[key]
    ids = [row["id"] for row in data]
    if len(ids) != expected or len(set(ids)) != expected:
        errors.append(f"Catalog {name}: expected {expected} unique entries")
acceptance = (ROOT / "docs/ACCEPTANCE.md").read_text(encoding="utf-8")
ids = re.findall(r"^\| (AC-\d{3}) \|", acceptance, re.M)
if ids != [f"AC-{i:03}" for i in range(1, 113)]:
    errors.append("Acceptance IDs are not continuous AC-001–AC-112")
for md in [ROOT / "README.md", *sorted((ROOT / "docs").glob("*.md"))]:
    content = md.read_text(encoding="utf-8")
    expected = None
    for n, line in enumerate(content.splitlines(), 1):
        if line.startswith("|"):
            count = len(re.split(r"(?<!\\)\|", line)) - 2
            if expected is None:
                expected = count
            elif count != expected:
                errors.append(f"Table shape: {md.name}:{n} {count} != {expected}")
        else:
            expected = None
    for target in re.findall(r"\]\(([^)\s]+)\)", content):
        if target.startswith(("http:", "https:", "#", "mailto:")):
            continue
        target = target.split("#")[0]
        if target and not (md.parent / target).resolve().exists():
            errors.append(f"Local link: {md.name} -> {target}")
for py in [*ROOT.glob("tools/*.py"), *ROOT.glob("tools/bench/*.py"), *ROOT.glob("tests/*.py")]:
    try:
        ast.parse(py.read_text(encoding="utf-8"), filename=str(py))
    except SyntaxError as exc:
        errors.append(f"Python syntax: {exc}")
for name in ["prototype/index.html", "texzi-design-handbook.html"]:
    file = ROOT / name
    if not file.exists():
        continue
    text = file.read_text(encoding="utf-8")
    if re.search(r'<(?:script|link|img)\b[^>]*(?:src|href)=["\']https?://', text):
        errors.append(f"Network asset in offline artifact: {name}")
    if name.endswith("handbook.html") and "{{CSS}}" in text:
        errors.append("Handbook template not fully rendered")
print(json.dumps({"scope": "design-package-integrity", "result": "Fail" if errors else "Pass",
                  "errors": errors, "commandCount": 115, "settingCount": 63,
                  "nativeAcceptanceCount": 112, "nativeAcceptanceStatus": "NotRun",
                  "browserStatus": "NotRun"}, ensure_ascii=False, indent=2))
sys.exit(1 if errors else 0)
