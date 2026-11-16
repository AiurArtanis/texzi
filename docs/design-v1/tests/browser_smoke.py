#!/usr/bin/env python3
"""Real-browser smoke checks for the review prototype, supplied as NOT RUN.

The authoring environment did not permit a browser run. This script does not
launch a server, discover another browser, change sandbox flags, or retry via a
different route. Run it only in an environment where browser execution and the
explicit target URL are permitted.

Install Playwright in the implementation team's approved environment, build the
single-file prototype, and supply its permitted review URL:

    python tests/browser_smoke.py --list
    python tests/browser_smoke.py --run --url <permitted-prototype-url>

No --run means NOT RUN, not PASS. Native NSH/Windows/IME/performance acceptance
remains independent even if these browser checks subsequently pass.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Callable, Any


def source_of(page: Any, doc_id: str) -> str:
    """Read-only observation; tests drive actual controls, not API mutations."""
    return page.evaluate(
        "id => window.texziReview.getState().docs.find(d => d.id === id).source",
        doc_id,
    )


def expect_source_contains(page: Any, doc_id: str, text: str) -> None:
    page.wait_for_function(
        "arg => window.texziReview.getState().docs.find(d => d.id === arg.id).source.includes(arg.text)",
        arg={"id": doc_id, "text": text},
    )


def select_scenario(page: Any, name: str) -> None:
    page.locator("#scenario-trigger").click()
    page.get_by_role("menuitem", name=name, exact=True).click()


def smoke_startup(page: Any, expect: Any) -> None:
    expect(page.locator(".editor-group")).to_have_count(1)
    expect(page.locator(".source-input")).to_have_value(re.compile("今日待办"))
    expect(page.locator(".titlebar")).not_to_contain_text("texzi")
    expect(page.locator("#quick-open")).to_be_visible()
    expect(page.locator("#prototype-help")).to_be_visible()


def smoke_plain_edit_undo(page: Any, expect: Any) -> None:
    textarea = page.locator(".source-input")
    original = textarea.input_value()
    textarea.click()
    textarea.press("Control+End")
    page.keyboard.insert_text("\nSMOKE 中文输入样本")
    expect(textarea).to_have_value(original + "\nSMOKE 中文输入样本")
    expect_source_contains(page, "todo", "SMOKE 中文输入样本")
    textarea.press("Control+z")
    expect(textarea).to_have_value(original)
    # insert_text is ordinary text insertion; this does not test real IME.


def smoke_split_rich_edit(page: Any, expect: Any) -> None:
    page.locator('[data-scene="split"]').click()
    expect(page.locator(".editor-group")).to_have_count(3)
    rich = page.locator(".rich-document").first
    heading = rich.locator('h1[contenteditable="true"]')
    heading.fill("texzi · Smoke 标题")
    expect_source_contains(page, "project", "# texzi · Smoke 标题")
    checkbox = rich.locator(".task-box").nth(1)
    checkbox.check()
    expect_source_contains(page, "project", "- [x] 确认概念设计")
    expect(rich.locator("table")).to_be_visible()
    expect(rich.locator('code[contenteditable="true"]')).to_be_visible()


def smoke_mode_roundtrip(page: Any, expect: Any) -> None:
    page.locator('.tab[data-doc="project"]').click()
    before = source_of(page, "project")
    page.locator(".md-toggle").click()
    expect(page.locator('.source-input[data-doc="project"]')).to_have_value(before)
    page.locator(".md-toggle").click()
    expect(page.locator(".rich-document")).to_be_visible()
    assert source_of(page, "project") == before, "Mode switching rewrote source"


def smoke_rich_find_then_regex(page: Any, expect: Any) -> None:
    page.locator('.tab[data-doc="project"]').click()
    page.locator(".rich-document h1").click()
    page.keyboard.press("Control+f")
    page.get_by_label("查找内容", exact=True).fill("概念")
    expect(page.locator(".find-scope")).to_have_text("正文")
    expect(page.locator(".rich-document")).to_be_visible()
    page.wait_for_function("window.texziReview.getState().searchResult.matches.length > 0")
    page.locator('[data-find-option="regex"]').click()
    expect(page.locator(".find-scope")).to_have_text("源码")
    expect(page.locator('.source-input[data-doc="project"]')).to_be_visible()
    page.get_by_label("查找内容", exact=True).fill("^#")
    page.wait_for_function("window.texziReview.getState().searchResult.matches.length > 0")
    # RegExp here is the prototype's JS engine, not the proposed PCRE2 engine.


def smoke_replace_undo(page: Any, expect: Any) -> None:
    original = source_of(page, "todo")
    page.locator(".source-input").click()
    page.keyboard.press("Control+h")
    page.get_by_label("查找内容", exact=True).fill("文本")
    page.get_by_label("替换为", exact=True).fill("SMOKE文字")
    page.wait_for_function("window.texziReview.getState().searchResult.matches.length > 0")
    page.locator(".replace-all").click()
    expect_source_contains(page, "todo", "SMOKE文字")
    page.get_by_label("关闭查找", exact=True).click()
    page.locator(".source-input").click()
    page.keyboard.press("Control+z")
    expect(page.locator(".source-input")).to_have_value(original)


def smoke_settings_and_persistence(page: Any, expect: Any) -> None:
    page.locator('[data-scene="settings"]').click()
    expect(page.locator(".settings-nav [data-category]")).to_have_count(7)
    expect(page.locator('[data-setting="markdown.defaultView"]')).to_have_value("wysiwyg")
    expect(page.locator('[data-setting="files.autoSave"]')).to_have_value("off")
    expect(page.locator('[data-setting="session.restoreOnStartup"]')).to_have_attribute("aria-checked", "true")
    page.locator('[data-setting="appearance.theme"]').select_option("dark")
    expect(page.locator("body")).to_have_class(re.compile(r"\bdark\b"))
    page.locator('[data-setting="editor.fontSize"]').fill("18")
    page.locator('[data-setting="editor.fontSize"]').press("Tab")
    page.wait_for_function(
        "JSON.parse(localStorage.getItem('texzi.review.v1') || '{}').settings?.['editor.fontSize'] === 18"
    )
    page.reload()
    page.wait_for_function("Boolean(window.texziReview)")
    expect(page.locator("body")).to_have_class(re.compile(r"\bdark\b"))
    page.locator('[data-scene="settings"]').click()
    expect(page.locator('[data-setting="editor.fontSize"]')).to_have_value("18")
    page.get_by_label("关闭设置", exact=True).click()
    expect(page.locator(".settings-page")).to_have_count(0)


def smoke_zen_roundtrip(page: Any, expect: Any) -> None:
    page.locator('[data-scene="split"]').click()
    page.locator(".rich-document h1").click()
    page.keyboard.press("Control+k")
    page.keyboard.press("z")
    expect(page.locator("body")).to_have_class(re.compile(r"\bzen\b"))
    page.keyboard.press("Escape")
    page.keyboard.press("Escape")
    expect(page.locator("body")).not_to_have_class(re.compile(r"\bzen\b"))
    expect(page.locator(".editor-group:visible")).to_have_count(3)


def smoke_dirty_close_cancel(page: Any, expect: Any) -> None:
    textarea = page.locator(".source-input")
    textarea.click()
    textarea.press("Control+End")
    page.keyboard.insert_text("\nDIRTY-SMOKE")
    page.locator('.tab-close[data-close="todo"]').click()
    expect(page.get_by_role("dialog")).to_be_visible()
    page.get_by_role("dialog").get_by_role("button", name="取消", exact=True).click()
    expect(page.locator('.tab[data-doc="todo"]')).to_be_visible()
    expect(page.locator(".source-input")).to_have_value(re.compile("DIRTY-SMOKE"))


def smoke_large_and_readonly_labels(page: Any, expect: Any) -> None:
    select_scenario(page, "模拟大文件模式")
    expect(page.locator(".inline-banner")).to_contain_text("场景演示")
    expect(page.locator(".inline-banner")).to_contain_text("小样本")
    expect(page.locator(".source-input")).to_be_editable()
    actual_bytes = page.evaluate(
        "new TextEncoder().encode(window.texziReview.getState().docs.find(d => d.id === 'large-demo').source).length"
    )
    assert actual_bytes < 1024 * 1024, "The sample must not be misreported as a measured 100 MiB file"
    select_scenario(page, "模拟只读文件")
    expect(page.locator(".source-input")).not_to_be_editable()
    expect(page.locator(".inline-banner")).to_contain_text("只读")
    page.get_by_role("button", name="退出只读演示", exact=True).click()
    expect(page.locator(".source-input")).to_be_editable()


def smoke_unknown_markdown(page: Any, expect: Any) -> None:
    select_scenario(page, "未知 Markdown 语法保留")
    expect(page.locator(".raw-block")).to_have_count(2)
    before = source_of(page, "raw-demo")
    page.locator(".md-toggle").click()
    expect(page.locator(".source-input")).to_have_value(before)
    page.locator(".md-toggle").click()
    assert source_of(page, "raw-demo") == before
    expect(page.locator(".raw-block").first).to_contain_text("custom: keep-me")


CASES: list[tuple[str, Callable[..., None]]] = [
    ("B01_startup", smoke_startup),
    ("B02_plain_edit_undo", smoke_plain_edit_undo),
    ("B03_split_rich_edit", smoke_split_rich_edit),
    ("B04_mode_roundtrip", smoke_mode_roundtrip),
    ("B05_rich_find_to_regex", smoke_rich_find_then_regex),
    ("B06_replace_undo", smoke_replace_undo),
    ("B07_settings_persistence", smoke_settings_and_persistence),
    ("B08_zen_roundtrip", smoke_zen_roundtrip),
    ("B09_dirty_close_cancel", smoke_dirty_close_cancel),
    ("B10_large_readonly_labels", smoke_large_and_readonly_labels),
    ("B11_unknown_markdown", smoke_unknown_markdown),
]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--run", action="store_true", help="Explicitly execute in a browser-permitted environment")
    parser.add_argument("--list", action="store_true", help="List supplied cases; does not run a browser")
    parser.add_argument("--url", help="Explicit permitted URL for the built review prototype")
    parser.add_argument("--headed", action="store_true", help="Show the actual browser window")
    parser.add_argument("--output", type=Path, default=Path("test-results/browser-smoke"))
    args = parser.parse_args()
    if not args.run:
        print(json.dumps({
            "status": "NotRun",
            "reason": "Browser execution was not authorized/available in the authoring environment. Supply --run and an explicitly permitted --url in the implementation environment.",
            "scope": "Real browser review prototype only; never native NSH/Windows acceptance",
            "cases": [name for name, _ in CASES],
        }, ensure_ascii=False, indent=2))
        return 0
    if not args.url:
        parser.error("--run requires an explicit permitted --url; there is no server or fallback route")
    try:
        from playwright.sync_api import sync_playwright, expect
    except ImportError as exc:
        print(json.dumps({"status": "Blocked", "reason": str(exc)}, ensure_ascii=False))
        return 2

    report: dict[str, Any] = {
        "startedAt": datetime.now(timezone.utc).isoformat(),
        "scope": "Browser review prototype, not native NSH/Windows/IME/performance evidence",
        "target": args.url,
        "cases": [],
    }
    args.output.mkdir(parents=True, exist_ok=True)
    try:
        with sync_playwright() as playwright:
            # No sandbox overrides or alternate browser/session fallback.
            browser = playwright.chromium.launch(headless=not args.headed)
            try:
                for name, callback in CASES:
                    context = browser.new_context(viewport={"width": 1600, "height": 1000}, locale="zh-CN")
                    page = context.new_page()
                    page.set_default_timeout(7000)
                    errors: list[str] = []
                    page.on("pageerror", lambda error: errors.append(str(error)))
                    item: dict[str, Any] = {"id": name, "status": "NotRun"}
                    try:
                        page.goto(args.url, wait_until="load")
                        page.wait_for_function("Boolean(window.texziReview)")
                        callback(page, expect)
                        if errors:
                            raise AssertionError("Uncaught page errors: " + " | ".join(errors))
                        page.screenshot(path=str(args.output / (name + ".png")), full_page=True)
                        item["status"] = "Pass"
                    except Exception as exc:
                        item.update(status="Fail", error=str(exc), pageErrors=errors)
                        try:
                            page.screenshot(path=str(args.output / (name + "-failed.png")), full_page=True)
                        except Exception:
                            pass
                    finally:
                        context.close()
                    report["cases"].append(item)
            finally:
                browser.close()
    except Exception as exc:
        report.update(status="Blocked", error=str(exc))
    else:
        report["status"] = "Fail" if any(item["status"] != "Pass" for item in report["cases"]) else "Pass"
    report["endedAt"] = datetime.now(timezone.utc).isoformat()
    (args.output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0 if report["status"] == "Pass" else 1


if __name__ == "__main__":
    sys.exit(main())
