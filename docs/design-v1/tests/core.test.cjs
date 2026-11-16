'use strict';

// Run: node --test tests/core.test.cjs
// This suite evaluates pure prototype logic and catalog data only.
// No DOM, browser, IME, native file IO or NSH behavior is simulated or certified.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { TextEncoder } = require('node:util');

const root = path.resolve(__dirname, '..');
const settings = JSON.parse(fs.readFileSync(path.join(root, 'data/settings.json'), 'utf8'));
const commands = JSON.parse(fs.readFileSync(path.join(root, 'data/commands.json'), 'utf8'));
const sourcePath = path.join(root, 'prototype/src/app.js');
const original = fs.readFileSync(sourcePath, 'utf8');
assert.match(original, /\bboot\(\);\s*$/, 'Expected one final boot() invocation to remove');
const source = original.replace(/\bboot\(\);\s*$/, '');
const storage = new Map();
const sandbox = {
  window: { TEXZI_SETTINGS: settings, TEXZI_COMMANDS: commands },
  localStorage: {
    getItem(key) { return storage.has(key) ? storage.get(key) : null; },
    setItem(key, value) { storage.set(key, String(value)); },
    removeItem(key) { storage.delete(key); }
  },
  TextEncoder,
  console,
  setTimeout() { throw new Error('A pure-function test unexpectedly scheduled UI work'); },
  clearTimeout() {},
};
const context = vm.createContext(sandbox);
vm.runInContext(source + '\n;globalThis.__pure = {searchCore, parseMarkdown, sourceLines, expandReplacement, renderMarkdown, inlineMarkdown, esc, makeDocument, textBytes};', context, {
  filename: sourcePath,
  timeout: 2000,
});
const api = sandbox.__pure;
const plain = value => JSON.parse(JSON.stringify(value));
const search = (text, query, options = {}) => plain(api.searchCore({ text, query, options }));
const render = (text, readOnly = false) => api.renderMarkdown({ source: text, readOnly });

function assertSourceRanges(text) {
  const blocks = plain(api.parseMarkdown(text));
  let end = 0;
  let rebuilt = '';
  for (const block of blocks) {
    assert.ok(Number.isInteger(block.start) && Number.isInteger(block.end));
    assert.ok(block.start >= end && block.end > block.start && block.end <= text.length);
    assert.equal(block.raw, text.slice(block.start, block.end), block.type + ' has an inaccurate source span');
    rebuilt += text.slice(end, block.start) + block.raw;
    end = block.end;
  }
  rebuilt += text.slice(end);
  assert.equal(rebuilt, text, 'Blocks plus untouched gaps must reconstruct the exact original string');
  return blocks;
}

test('harness excludes boot and contains no fake DOM', () => {
  assert.equal(typeof sandbox.document, 'undefined');
  assert.equal(typeof api.searchCore, 'function');
  assert.equal(typeof api.renderMarkdown, 'function');
  assert.equal(storage.size, 0);
});

test('empty search returns no matches and no error', () => {
  const result = search('text', '');
  assert.deepEqual(result.matches, []);
  assert.equal(result.error, null);
});

test('literal search escapes regex punctuation', () => {
  const result = search('a.[x]* aZxx a.[x]*', 'a.[x]*');
  assert.equal(result.error, null);
  assert.equal(result.matches.length, 2);
  assert.deepEqual(result.matches.map(m => m.text), ['a.[x]*', 'a.[x]*']);
});

test('case and whole-word flags follow the documented ASCII fixture', () => {
  const text = 'alpha alphabet Alpha alpha_1';
  assert.equal(search(text, 'alpha').matches.length, 4);
  assert.equal(search(text, 'alpha', { case: true }).matches.length, 3);
  assert.equal(search(text, 'alpha', { word: true }).matches.length, 2);
  assert.equal(search(text, 'alpha', { word: true, case: true }).matches.length, 1);
});

test('Chinese whole-word search does not pretend to use semantic segmentation', () => {
  const text = '猫咪 猫咪们 猫';
  assert.equal(search(text, '猫').matches.length, 3);
  assert.equal(search(text, '猫', { word: true }).matches.length, 1);
});

test('combining marks stay within a Unicode word boundary', () => {
  const result = search('e\u0301 e', 'e', { word: true, case: true });
  assert.deepEqual(result.matches.map(m => [m.start, m.end]), [[3, 4]]);
});

test('regex supports explicit cross-line matches and multiline anchors', () => {
  assert.equal(search('one\ntwo\n', 'one\\ntwo', { regex: true }).matches[0].text, 'one\ntwo');
  assert.equal(search('one\ntwo', '^two$', { regex: true }).matches.length, 1);
  assert.equal(search('one\ntwo', 'one.*two', { regex: true }).matches.length, 0);
});

test('zero-width iteration advances across an astral scalar without splitting it', () => {
  const result = search('😀a', '(?=.)', { regex: true });
  assert.deepEqual(result.matches.map(m => [m.start, m.end]), [[0, 0], [2, 2]]);
});

test('zero-width EOF is emitted at most once', () => {
  const result = search('😀', '(?:)', { regex: true });
  assert.deepEqual(result.matches.map(m => m.start), [0, 2]);
  assert.equal(search('', '$', { regex: true }).matches.length, 1);
});

test('invalid regex reports an error without a misleading match', () => {
  const result = search('unchanged', '(', { regex: true });
  assert.match(result.error, /表达式无效/);
  assert.deepEqual(result.matches, []);
});

test('match spans are exact, ordered and non-overlapping', () => {
  const text = '😀 xx 中文 xx\nxx';
  const result = search(text, 'xx');
  let lastEnd = 0;
  for (const match of result.matches) {
    assert.ok(match.start >= lastEnd);
    assert.equal(text.slice(match.start, match.end), match.text);
    lastEnd = match.end;
  }
  assert.equal(result.matches.length, 3);
});

test('high-volume search explicitly marks truncated results', () => {
  // This is a linear literal query, deliberately not a pathological regex.
  const result = search('a'.repeat(20001), 'a');
  assert.equal(result.truncated, true);
  assert.equal(result.matches.length, 20000);
});

test('named and numeric captures are preserved in search results', () => {
  const match = search('alpha-12', '(?<name>[a-z]+)-(\\d+)', { regex: true }).matches[0];
  assert.deepEqual(match.captures, ['alpha', '12']);
  assert.deepEqual(match.groups, { name: 'alpha' });
});

test('replacement expands whole match, captures, names and literal dollar', () => {
  const match = search('alpha-12', '(?<name>[a-z]+)-(\\d+)', { regex: true }).matches[0];
  assert.equal(api.expandReplacement('$0|$&|$1|$2|${name}|$$', match), 'alpha-12|alpha-12|alpha|12|alpha|$');
});

test('an existing optional capture that did not participate expands to empty', () => {
  const match = api.searchCore({ text: 'b', query: '(a)?(?<tail>b)', options: { regex: true } }).matches[0];
  assert.equal(api.expandReplacement('$1:${tail}', match), ':b');
});

test('unknown numeric replacement group is rejected rather than deleting text', () => {
  const match = search('a', '(a)', { regex: true }).matches[0];
  assert.throws(() => api.expandReplacement('$2', match), /捕获|分组|group|不存在/i);
});

test('unknown named replacement group is rejected', () => {
  const match = search('a', '(?<known>a)', { regex: true }).matches[0];
  assert.throws(() => api.expandReplacement('${missing}', match), /捕获|分组|group|不存在/i);
});

test('source lines retain CRLF and a final line without newline', () => {
  const text = 'A\r\n\r\nB😀';
  const rows = plain(api.sourceLines(text));
  assert.equal(rows.map(row => row.raw).join(''), text);
  assert.deepEqual(rows.map(row => [row.start, row.end]), [[0, 3], [3, 5], [5, 8]]);
});

test('Markdown headings, tasks, tables and fenced code expose real source spans', () => {
  const text = '# 标题\r\n\r\n- [x] 完成\r\n- [ ] 待办\r\n\r\n| A | B |\r\n| --- | ---: |\r\n| 一 | 二 |\r\n\r\n```js\r\nconst x = 1;\r\n```';
  const blocks = assertSourceRanges(text);
  assert.deepEqual(blocks.map(block => block.type), ['heading', 'tasks', 'table', 'code']);
  assert.equal(blocks[1].items[0].checked, true);
  assert.equal(blocks[1].items[1].checked, false);
  assert.equal(blocks[3].text, 'const x = 1;');
});

test('untouched gaps, trailing spaces, blank lines and unknown nodes remain recoverable', () => {
  const text = '\n\n# 一  \r\n\r\n::: custom\r\n保留内容  \r\n:::\r\n\r\n结束  ';
  const blocks = assertSourceRanges(text);
  const raw = blocks.find(block => block.type === 'raw');
  assert.ok(raw);
  assert.equal(raw.raw, '::: custom\r\n保留内容  \r\n:::\r\n');
});

test('front matter and unsupported formulas remain source-backed raw blocks', () => {
  const text = '---\ntitle: keep-me\ncustom: 12\n---\n\n$$\nx+y\n$$\n';
  const blocks = assertSourceRanges(text);
  assert.deepEqual(blocks.map(block => block.type), ['raw', 'raw']);
  assert.match(blocks[0].raw, /custom: 12/);
  assert.match(blocks[1].raw, /\$\$/);
});

test('unclosed fenced code retains the entire remaining source span', () => {
  const text = '```txt\n<not-an-element>\n尾部没有围栏';
  const blocks = assertSourceRanges(text);
  assert.equal(blocks.length, 1);
  assert.equal(blocks[0].raw, text);
  assert.equal(blocks[0].text, '<not-an-element>\n尾部没有围栏');
});

test('Markdown rendering does not mutate source, revision or history', () => {
  const document = api.makeDocument('md-test', 'sample.md', 'sample.md', '# 标题\n\n正文\n');
  const before = JSON.stringify(document);
  const html = api.renderMarkdown(document);
  assert.match(html, /<h1/);
  assert.equal(JSON.stringify(document), before);
});

test('raw HTML is displayed as escaped text rather than executable markup', () => {
  const html = render('<script>globalThis.pwned=1</script>\n\n<img src=x onerror=alert(1)>\n');
  assert.doesNotMatch(html, /<script\b|<img\b/i);
  assert.match(html, /&lt;script&gt;/);
  assert.match(html, /&lt;img/);
});

test('code and inline text escape script and attribute injection', () => {
  const html = render('```html\n<img src=x onerror="alert(1)">\n```\n\n正文 <svg onload="alert(2)">\n');
  assert.doesNotMatch(html, /<img\b|<svg\b|<script\b/i);
  assert.match(html, /&lt;img/);
  assert.match(html, /&quot;alert\(1\)&quot;/);
});

test('rendered link destination cannot break out of its data attribute', () => {
  const html = api.inlineMarkdown('[label](javascript:evil" onmouseover="x)');
  assert.match(html, /<a href="#" data-link="/);
  assert.doesNotMatch(html, /href="javascript:/i);
  assert.doesNotMatch(html, /"\s+onmouseover="/i);
  assert.match(html, /&quot;/);
});

test('read-only Markdown emits no editable text and disables task boxes', () => {
  const html = render('# 标题\n\n- [ ] 任务\n', true);
  assert.doesNotMatch(html, /contenteditable="true"/);
  assert.match(html, /contenteditable="false"/);
  assert.match(html, /type="checkbox"[^>]*disabled/);
});

test('catalog IDs are unique and command source references are defined', () => {
  assert.equal(new Set(settings.settings.map(item => item.id)).size, settings.settings.length);
  assert.equal(new Set(commands.commands.map(item => item.id)).size, commands.commands.length);
  const sourceIds = new Set(commands.baseline.sources.map(item => item.id));
  for (const command of commands.commands) {
    assert.ok(command.id && command.name && command.context && command.behavior);
    assert.ok(Array.isArray(command.defaultKeys));
    for (const id of command.sourceIds || []) assert.ok(sourceIds.has(id), command.id + ': unresolved source ' + id);
  }
});

test('catalog defaults satisfy their declared type, enum and numeric range', () => {
  for (const setting of settings.settings) {
    const value = setting.default;
    if (setting.type === 'boolean') assert.equal(typeof value, 'boolean', setting.id);
    if (setting.type === 'integer') assert.ok(Number.isInteger(value), setting.id);
    if (setting.type === 'number') assert.equal(typeof value, 'number', setting.id);
    if (setting.type === 'string') assert.equal(typeof value, 'string', setting.id);
    if (setting.type === 'stringArray') assert.ok(Array.isArray(value) && value.every(v => typeof v === 'string'), setting.id);
    if (setting.options && setting.options.length) assert.ok(setting.options.some(option => option.value === value), setting.id + ': default missing from options');
    if (setting.min !== undefined) assert.ok(value >= setting.min, setting.id);
    if (setting.max !== undefined) assert.ok(value <= setting.max, setting.id);
    assert.ok(setting.apply && setting.description && Array.isArray(setting.dependencies), setting.id);
  }
});

test('catalog preserves confirmed defaults and seven settings navigation entries', () => {
  const defaults = Object.fromEntries(settings.settings.map(item => [item.id, item.default]));
  assert.equal(defaults['markdown.defaultView'], 'wysiwyg');
  assert.equal(defaults['files.autoSave'], 'off');
  assert.equal(defaults['session.restoreOnStartup'], true);
  assert.equal(defaults['session.restoreUnsaved'], true);
  assert.equal(defaults['editor.fontSize'], 16);
  assert.equal(defaults['performance.largeFileThresholdMiB'], 20);
  assert.equal(defaults['performance.hugeFileThresholdMiB'], 200);
  assert.equal(defaults['performance.longLineThresholdChars'], 20000);
  assert.deepEqual(settings.navigation.map(item => item.name), ['常用', '编辑器', '外观', '文件与恢复', 'Markdown', '窗口与标签', '快捷键']);
  assert.equal(settings.lifecyclePolicies.thresholdComparison, '>=');
  assert.equal(settings.lifecyclePolicies.windowCloseWithoutSavePrompt, 'session.restoreOnStartup && session.restoreUnsaved && snapshotFlushSucceeded');
});

test('VS Code baseline is pinned and source-mode transitions are explicit', () => {
  assert.equal(commands.baseline.vscodeVersion, '1.146.0');
  assert.equal(commands.baseline.commit, '69047191fc427f2f92828279aae1d90d8eeda7ab');
  for (const command of commands.commands.filter(item => item.viewTransition)) {
    assert.ok(command.invocationContext, command.id + ' lacks invocation context');
  }
  const pasteMarkdown = commands.commands.find(item => item.id === 'texzi.markdown.pasteMarkdown');
  assert.ok(pasteMarkdown, 'Explicit Markdown paste must remain separately available');
  assert.deepEqual(pasteMarkdown.defaultKeys, []);
});
