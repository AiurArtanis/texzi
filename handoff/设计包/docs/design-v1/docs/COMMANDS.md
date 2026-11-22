# texzi Windows命令与默认快捷键 v1

> **2026-11-20 起本文只作参考。** v1 的范围和验收只看 [docs/V1-SCOPE.md](../../V1-SCOPE.md)。本文中的「必须」「阻断发布」「Gate」「验收」不再构成门禁；与 V1-SCOPE 冲突时，以 V1-SCOPE 为准。

机器可读对应 `../data/commands.json`。本表覆盖115个命令，其中光标移动、扩选、矩形选择与按编号聚焦采用带参数的稳定命令；每个方向/编号的按键和VS Code命令ID另列在第4节，不依赖“与VS Code一致”这类笼统描述。

## 1. 核实基准与证据等级

**基准：Visual Studio Code 1.146.0 Stable，2026-11-11发布，tag commit `69047191fc427f2f92828279aae1d90d8eeda7ab`。核实日期：2026-11-15（Asia/Shanghai）。** 官方更新页与GitHub Release/latest均指向1.146.0；GitHub tag API返回上述commit。仅使用Windows桌面默认键，未引入Insiders、Web或macOS的覆盖。来源见[RELEASE]与[TAG]。

固定版本源码确认的键与上下文在行内标“固定版本源码已核实”。本轮所有表内VS Code默认键对照均已补充固定版本源码核实，包括鼠标多光标的默认配置与事件分派；texzi新增项或行为差异仍明确标设计建议。`reference_verified_version_pending`保留为数据格式支持的状态，当前没有此状态的命令。Windows真机、输入法与键盘布局验证尚未完成，仍是实施验收项。

**上下文桥接**：表中`context`是执行状态；源码专属且支持自动切换的命令另有`invocationContext`与`viewTransition`，可先在WYSIWYG接收调用，完成源码锚点映射后再检查执行状态。不能因为最初不满足`sourceEditorFocus`而静默禁用。机器可读JSON已逐条列出。

空快捷键表示 **Unassigned（默认未绑定）**，不是功能缺失。所有命令均应可从菜单、按钮或命令面板发现。`defaultKeys`是texzi建议值；`vsCodeKeys`是已核实/待核实的上游对照，不自动覆盖texzi设计差异。

`Ctrl+K S`表示按Ctrl+K再按S；`Ctrl+K Ctrl+S`第二段仍按Ctrl。`Ctrl+K O`是同文档新窗口视图；`Ctrl+K Ctrl+O`是打开文件夹。菜单必须按这种区别显示。

## 2. 键盘路由与冲突处理

- **ime**：composition期间输入与候选键由TSF/输入法处理；不执行本文档编辑命令，不以Alt/Ctrl/Shift的单独按下或抬起触发操作。
- **osReserved**：不注册系统全局键盘钩子；不重绑Win键组合、Alt+Tab、Ctrl+Alt+Delete；Alt+F4经系统关闭事件处理。Ctrl+Space、Shift中英切换、Ctrl+Shift与Alt+Shift布局切换保留给系统。
- **contextPriority**：IME/composition → 原生对话框或顶部模态 → 当前弹层/查找输入控件 → WYSIWYG/源码局部命令 → 窗口命令
- **chords**：空格分隔顺序按键；每一步独立记录修饰键。Ctrl+K后1.5秒建议超时；Esc取消。未匹配第二键不写入正文并提示无此组合。
- **customBindings**：用户重绑优先于同上下文默认；上下文重叠的重复键必须显式选择替换或取消，不能静默覆盖；系统保留键不可绑定。
- **wysiwyg**：WYSIWYG首版为单主选区。多光标、矩形选择和按源码行操作通过原位切源码执行，保留源码锚点；建议边界写入PRD。
- **findScopes**：WYSIWYG普通查找是可见正文；启用正则、替换或选择全部匹配时先切源码、说明范围变化并重算。部分匹配=全字关闭。
- **baselineMeaning**：verified_source核实的是VSCode1.146.0命令与默认键，texzi的业务语义与上下文仍以本文件为准。reference_verified_version_pending表示官方参考已验证，但该版本实现位置仍待复核，不能声称已完成逐项源码核验。
- **unassigned**：defaultKeys为空即默认未绑定；仍可通过菜单、按钮或命令面板触发。

### 已确定的texzi上下文差异

- WYSIWYG正文 `Ctrl+B` 加粗；其他上下文 `Ctrl+B` 切侧栏；所有正文可用 `Ctrl+K Ctrl+B` 切侧栏。WYSIWYG `Ctrl+I` 为斜体。此为texzi建议，不能宣传为VS Code的原样默认。
- `>_` 源码切换默认未绑定，使用正文右上按钮或命令面板。插入链接不占用Ctrl+K前缀。
- `Ctrl+PageUp/PageDown`限定当前组；VS Code对应nextEditor/previousEditor的跨组范围不照搬。新组不存在时Ctrl+数字不隐式创建组。
- Ctrl+Shift及Alt+Shift可能被Windows配置为语言/键盘切换；应用不拦截纯修饰键序列。Ctrl+Alt+方向键也可能被显卡热键占用；提供改键，不修改用户系统设置。
- 不绑定Ctrl+Space，不绑定Win键组合；Alt+F4走系统WM_CLOSE/关闭流程。首版不为建议、补全、调试等IDE行为占用快捷键。
- 一个快捷键只由当前焦点链中的最高优先级处理一次；搜索框的Ctrl+A/C/X/V/Z等永远针对搜索框，不越级改正文。

## 3. 命令注册表

### 文件

| 命令ID / 中文名 | Windows默认键 | 生效上下文 | 功能规则 | VS Code对照 / 证据 | Markdown行为 / 冲突 | 优先级 |
|---|---|---|---|---|---|---|
| `texzi.file.new`<br>新建文本 | Ctrl+N | windowFocus && !modalOpen && !imeComposing | 创建独立未命名缓冲区；默认纯文本，立即可输入。 | workbench.action.files.newUntitledFile<br>固定版本源码已核实 [FILE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.file.open`<br>打开文件 | Ctrl+O | windowFocus && !modalOpen && !imeComposing | 多选打开文件；已打开的规范路径复用缓冲区。 | workbench.action.files.openFile<br>固定版本源码已核实 [WORKSPACE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.folder.open`<br>打开文件夹 | Ctrl+K Ctrl+O | windowFocus && !modalOpen && !imeComposing | 打开单根文件夹；按window.openFolders决定当前/新窗口。在当前窗口替换根树时保留所有已打开标签、dirty文档和视图，不触发关闭文档的保存确认。 | workbench.action.files.openFolder<br>固定版本源码已核实 [WORKSPACE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.folder.close`<br>关闭文件夹 | Ctrl+K F | windowFocus && !modalOpen && !imeComposing | 仅移除当前工作区根树和根目录搜索范围；已打开标签、dirty文档、独立文件与视图保持，不删除磁盘文件、不触发文档关闭确认。 | workbench.action.closeFolder<br>固定版本源码已核实 [WORKSPACE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.file.save`<br>保存 | Ctrl+S | windowFocus && !modalOpen && !imeComposing | 保存活动文档；未命名时弹原生另存对话框；冲突或编码错误不覆盖。 | workbench.action.files.save<br>固定版本源码已核实 [FILE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.file.saveAs`<br>另存为 | Ctrl+Shift+S | windowFocus && !modalOpen && !imeComposing | 选路径和编码后原子写入；当前文档身份切换到新路径。 | workbench.action.files.saveAs<br>固定版本源码已核实 [FILE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.file.saveAll`<br>全部保存 | Ctrl+K S | windowFocus && !modalOpen && !imeComposing | 保存当前窗口所有已修改文档；未命名逐一选择路径；失败汇总且可重试。 | saveAll<br>固定版本源码已核实 [FILE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>第二段是S，不是Ctrl+S；Ctrl+K Ctrl+S为快捷键设置。 | P0 |
| `texzi.file.revert`<br>从磁盘重新载入 | Unassigned | windowFocus && !modalOpen && !imeComposing && fileBacked | 有未保存改动先确认并保留恢复副本，随后读取磁盘当前版本。 | workbench.action.files.revert<br>固定版本源码已核实 [FILE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.file.reopenWithEncoding`<br>以编码重新打开 | Unassigned | windowFocus && !modalOpen && !imeComposing && fileBacked | 先预览候选解码；确认后重载。未保存内容先处理，不能将解码替换字符当成成功。 | 无直接对照<br>texzi新增/建议  | 重新解码后重建派生Markdown视图；保留源码锚点。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.file.saveWithEncoding`<br>使用编码保存 | Unassigned | windowFocus && !modalOpen && !imeComposing | 选择编码后检查所有字符是否可表示；失败不落盘，可改UTF-8或另存。 | 无直接对照<br>texzi新增/建议  | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.file.changeEol`<br>转换换行符 | Unassigned | editorFocus && !imeComposing | 明确选择CRLF或LF；整个文档一次事务，混合换行不自动转换。 | 无直接对照<br>texzi新增/建议  | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.file.copyPath`<br>复制完整路径 | Ctrl+K P | windowFocus && !modalOpen && !imeComposing && fileBacked | 复制磁盘绝对路径；未命名文档禁用。 | workbench.action.files.copyPathOfActiveFile<br>固定版本源码已核实 [FILE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.file.revealInExplorer`<br>在Windows资源管理器中显示 | Ctrl+K R | windowFocus && !modalOpen && !imeComposing && fileBacked | 用Shell打开所在目录并选中文件。与应用内文件树定位不同。 | workbench.action.files.revealActiveFileInWindows<br>固定版本源码已核实 [FILENATIVE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |

### 窗口

| 命令ID / 中文名 | Windows默认键 | 生效上下文 | 功能规则 | VS Code对照 / 证据 | Markdown行为 / 冲突 | 优先级 |
|---|---|---|---|---|---|---|
| `texzi.window.new`<br>新建窗口 | Ctrl+Shift+N | windowFocus && !modalOpen && !imeComposing | 创建空原生窗口，共用应用进程内文档服务。 | workbench.action.newWindow<br>固定版本源码已核实 [WINDOW] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.window.close`<br>关闭窗口 | Alt+F4<br>Ctrl+Shift+W | windowFocus && !modalOpen && !imeComposing | 关闭当前窗口：仅当session.restoreOnStartup与session.restoreUnsaved均为true且该窗口恢复快照成功刷盘，才可保留dirty文档后直接关闭；否则对将失去最后视图的dirty文档按DocId执行保存/放弃/取消。仍被其他窗口引用的文档只移除本窗口视图。失败或取消时保留未完成关闭的窗口和数据。 | workbench.action.closeWindow<br>固定版本源码已核实 [WINNATIVE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>Alt+F4交由Windows系统关闭消息处理，不用全局钩子拦截；Ctrl+Shift+W为应用入口。 | P0 |

### 编辑

| 命令ID / 中文名 | Windows默认键 | 生效上下文 | 功能规则 | VS Code对照 / 证据 | Markdown行为 / 冲突 | 优先级 |
|---|---|---|---|---|---|---|
| `texzi.edit.undo`<br>撤销 | Ctrl+Z | textInputFocus && !imeComposing | 正文按文档事务撤销；搜索/设置输入框只撤销该输入框；模式切换不占撤销步。 | undo<br>固定版本源码已核实 [EXT] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.redo`<br>重做 | Ctrl+Y / Ctrl+Shift+Z | textInputFocus && !imeComposing | 重做同一文档历史；发生新编辑后清空对应redo分支。 | redo<br>固定版本源码已核实 [EXT] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.cut`<br>剪切 | Ctrl+X / Shift+Delete | textInputFocus && writable && !imeComposing | 剪切选区；源码空选区且设置开启时剪切当前行；多选区一次事务。 | editor.action.clipboardCutAction<br>固定版本源码已核实 [CLIP] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.copy`<br>复制 | Ctrl+C / Ctrl+Insert | textInputFocus && !imeComposing | 复制选区；源码空选区按editor.copyEmptySelection；WYSIWYG提供纯文本及兼容HTML剪贴板表示。 | editor.action.clipboardCopyAction<br>固定版本源码已核实 [CLIP] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.paste`<br>粘贴 | Ctrl+V<br>Shift+Insert | textInputFocus && writable && !imeComposing | 普通粘贴优先读取剪贴板纯文字，不解析HTML或Markdown结构。源码多光标：texzi多选元数据与光标数相等时一对一；普通外部文本完整粘到每个光标；元数据数量不匹配则提示并取消。换行按目标文档规则，一次事务提交。 | editor.action.clipboardPasteAction<br>固定版本源码已核实 [CLIP] | WYSIWYG按字面文字插入，Markdown符号不自动转结构；只有显式“粘贴Markdown”才解析结构。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.pastePlain`<br>原样粘贴文本 | Ctrl+Shift+V | markdownWysiwygFocus && writable && !imeComposing | 显式忽略HTML、Markdown及多选剪贴板元数据，作为普通文字插入；Markdown符号按字面插入，不触发自动结构转换。 | 无直接对照<br>texzi新增/建议  | 仅在WYSIWYG正文使用；与普通粘贴相同的纯文字结果，保留明确的无格式入口。<br>仅WYSIWYG定义；源码Ctrl+V本来就是文本粘贴，Ctrl+Shift+V可作为同义键。 | P0 |
| `texzi.edit.selectAll`<br>全选 | Ctrl+A | textInputFocus && !imeComposing | 全选当前正文或当前输入控件内容；不跨文档。 | editor.action.selectAll<br>固定版本源码已核实 [EXT] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.deleteBackward`<br>向前删除 | Backspace / Shift+Backspace | editorFocus && !imeComposing && writable | 有选区删除选区，否则删除前一个grapheme cluster；不拆开代理对/组合字符。 | deleteLeft<br>固定版本源码已核实 [CORE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.deleteForward`<br>向后删除 | Delete | editorFocus && !imeComposing && writable | 有选区删除选区，否则删除后一个grapheme cluster。 | deleteRight<br>固定版本源码已核实 [CORE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.deleteWordLeft`<br>删除前一个词 | Ctrl+Backspace | editorFocus && !imeComposing && writable | 按明确Unicode词边界向前删除；中文不声称智能分词。 | deleteWordLeft<br>固定版本源码已核实 [WORDS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.deleteWordRight`<br>删除后一个词 | Ctrl+Delete | editorFocus && !imeComposing && writable | 按词边界向后删除；多个选区合并重叠编辑后一次提交。 | deleteWordRight<br>固定版本源码已核实 [WORDS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.indent`<br>增加缩进 | Ctrl+] | editorFocus && !imeComposing && writable | 源码对涉及行增加一级缩进；WYSIWYG对列表项增加层级，普通段落不插入不可逆样式。 | editor.action.indentLines<br>固定版本源码已核实 [LINES] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.outdent`<br>减少缩进 | Ctrl+[ | editorFocus && !imeComposing && writable | 源码减少不超过一级缩进；WYSIWYG列表减少层级。 | editor.action.outdentLines<br>固定版本源码已核实 [LINES] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.tab`<br>Tab输入或结构导航 | Tab | editorFocus && !imeComposing && writable && !tabMovesFocus | 源码插入缩进/缩进选中行；WYSIWYG表格进入下一格，列表增加层级，其余按既定Tab规则。 | tab<br>固定版本源码已核实 [CORE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>tabMovesFocus=true时交给焦点导航；不在设置表单中插入Tab字符。 | P0 |
| `texzi.edit.shiftTab`<br>反向Tab或减少缩进 | Shift+Tab | editorFocus && !imeComposing && writable && !tabMovesFocus | 源码减少缩进；WYSIWYG表格上一格、列表减少层级。 | outdent<br>固定版本源码已核实 [CORE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.moveLineUp`<br>上移行 | Alt+Up | sourceEditorFocus && !imeComposing && writable | 作用于所有选区覆盖的逻辑行，重叠行去重；文档边界不额外制造空行，一次撤销恢复。 | editor.action.moveLinesUpAction<br>固定版本源码已核实 [LINES] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.moveLineDown`<br>下移行 | Alt+Down | sourceEditorFocus && !imeComposing && writable | 作用于所有选区覆盖的逻辑行，重叠行去重；文档边界不额外制造空行，一次撤销恢复。 | editor.action.moveLinesDownAction<br>固定版本源码已核实 [LINES] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.copyLineUp`<br>向上复制行 | Shift+Alt+Up | sourceEditorFocus && !imeComposing && writable | 作用于所有选区覆盖的逻辑行，重叠行去重；文档边界不额外制造空行，一次撤销恢复。 | editor.action.copyLinesUpAction<br>固定版本源码已核实 [LINES] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.copyLineDown`<br>向下复制行 | Shift+Alt+Down | sourceEditorFocus && !imeComposing && writable | 作用于所有选区覆盖的逻辑行，重叠行去重；文档边界不额外制造空行，一次撤销恢复。 | editor.action.copyLinesDownAction<br>固定版本源码已核实 [LINES] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.deleteLine`<br>删除整行 | Ctrl+Shift+K | sourceEditorFocus && !imeComposing && writable | 删除选区覆盖的完整逻辑行；最后一行与唯一空行单独处理，不越界。 | editor.action.deleteLines<br>固定版本源码已核实 [LINES] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.insertLineAbove`<br>在上方插入行 | Ctrl+Shift+Enter | sourceEditorFocus && !imeComposing && writable | 在当前逻辑行前插入一行，按自动缩进规则定位光标。 | editor.action.insertLineBefore<br>固定版本源码已核实 [LINES] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.insertLineBelow`<br>在下方插入行 | Ctrl+Enter | sourceEditorFocus && !imeComposing && writable | 在当前逻辑行后插入一行，保留当前行内容。 | editor.action.insertLineAfter<br>固定版本源码已核实 [LINES] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.edit.pasteLinesToCursors`<br>按行分配粘贴 | Unassigned | sourceEditorFocus && writable && !imeComposing && multipleCursors | 显式将剪贴板纯文字按CRLF/LF/CR拆成逻辑行，按文档顺序一行对应一个光标；末尾终止换行不额外产生一行。行数与光标数必须相等，不等则说明实际数量并取消；一次事务提交。 | 无直接对照<br>texzi新增/建议  | 仅源码/代码多光标可用；WYSIWYG首版单选区，命令禁用并说明需先进入源码建立多个光标。<br>默认未绑定；通过上下文菜单、格式按钮或命令面板调用，遵守IME与焦点优先级。 | P0 |

### 导航

| 命令ID / 中文名 | Windows默认键 | 生效上下文 | 功能规则 | VS Code对照 / 证据 | Markdown行为 / 冲突 | 优先级 |
|---|---|---|---|---|---|---|
| `texzi.cursor.move`<br>移动光标（逐方向） | 逐项见第4节；12组输入 | editorFocus && !imeComposing | 方向逐项见variants；上下移动按视觉行并记忆目标列；水平移动按grapheme cluster。 | 见variants<br>固定版本源码已核实 [CORE] [WORDS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.navigation.gotoLine`<br>转到行/列 | Ctrl+G | editorFocus && !imeComposing | 输入行号或行:列；非法范围显示校验，不悄悄跳到错误位置。Markdown转源码后定位。 | workbench.action.gotoLine<br>固定版本源码已核实 [GOTO] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.navigation.quickOpen`<br>快速打开文件 | Ctrl+P / Ctrl+E | windowFocus && !modalOpen && !imeComposing | 搜索已打开/最近/当前根目录文件；只做文件定位，不索引代码符号。 | workbench.action.quickOpen<br>固定版本源码已核实 [QUICK] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.navigation.back`<br>后退到上个位置 | Alt+Left | windowFocus && !modalOpen && !imeComposing | 按窗口内显式跳转历史回退，不把每个按键加入历史。 | workbench.action.navigateBack<br>固定版本源码已核实 [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.navigation.forward`<br>前进到下个位置 | Alt+Right | windowFocus && !modalOpen && !imeComposing | 在仍有效的导航历史中前进。 | workbench.action.navigateForward<br>固定版本源码已核实 [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |

### 选择

| 命令ID / 中文名 | Windows默认键 | 生效上下文 | 功能规则 | VS Code对照 / 证据 | Markdown行为 / 冲突 | 优先级 |
|---|---|---|---|---|---|---|
| `texzi.selection.extend`<br>扩展选区（逐方向） | 逐项见第4节；12组输入 | editorFocus && !imeComposing | 保留anchor移动head；跨行、反向选择与软换行均保持可恢复的源码位置。 | 见variants<br>固定版本源码已核实 [CORE] [WORDS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.selection.line`<br>选择当前行 | Ctrl+L | sourceEditorFocus && !imeComposing | 选当前逻辑行；重复执行向后增加一行。 | expandLineSelection<br>固定版本源码已核实 [LINESEL] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |

### 多光标

| 命令ID / 中文名 | Windows默认键 | 生效上下文 | 功能规则 | VS Code对照 / 证据 | Markdown行为 / 冲突 | 优先级 |
|---|---|---|---|---|---|---|
| `texzi.multicursor.addMouse`<br>鼠标添加光标 | Alt+Click | sourceEditorFocus && !imeComposing | 在点击处添加/切换一个光标；重复位置去重，设置可改Ctrl。 | createCursor<br>固定版本源码已核实 [OPTIONS] [VIEWCTRL] [CORE] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>受editor.multiCursorModifier影响；不抢系统窗口拖动行为。 | P0 |
| `texzi.multicursor.above`<br>在上方添加光标 | Ctrl+Alt+Up | sourceEditorFocus && !imeComposing | 在上一视觉行相同目标列添加光标，短行停在行尾。 | editor.action.insertCursorAbove<br>固定版本源码已核实 [MULTI] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>可能被显卡旋转热键捕获；允许改键，不修改系统/驱动快捷键。 | P0 |
| `texzi.multicursor.below`<br>在下方添加光标 | Ctrl+Alt+Down | sourceEditorFocus && !imeComposing | 在下一视觉行相同目标列添加光标。 | editor.action.insertCursorBelow<br>固定版本源码已核实 [MULTI] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>可能被显卡旋转热键捕获；允许改键，不修改系统/驱动快捷键。 | P0 |
| `texzi.multicursor.addNext`<br>选择下一个相同内容 | Ctrl+D | sourceEditorFocus && !imeComposing | 无选区先选当前词；再加入下一个相同文本，循环后不重复添加。 | editor.action.addSelectionToNextFindMatch<br>固定版本源码已核实 [MULTI] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.multicursor.skipNext`<br>跳过当前匹配选择下一个 | Ctrl+K Ctrl+D | sourceEditorFocus && !imeComposing | 移动最后添加的选区到下一个相同文本，其他选区不变。 | editor.action.moveSelectionToNextFindMatch<br>固定版本源码已核实 [MULTI] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.multicursor.selectAllOccurrences`<br>选择所有相同内容 | Ctrl+Shift+L | sourceEditorFocus && !imeComposing | 选择当前选区全部出现位置；超过受测多光标预算时给出提示，不创建无界光标。 | editor.action.selectHighlights<br>固定版本源码已核实 [MULTI] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.multicursor.selectWordOccurrences`<br>选择当前词全部出现位置 | Ctrl+F2 | sourceEditorFocus && !imeComposing | 按当前词和明确边界规则添加所有匹配。 | editor.action.changeAll<br>固定版本源码已核实 [MULTI] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.multicursor.undo`<br>撤销上次光标操作 | Ctrl+U | sourceEditorFocus && !imeComposing | 只恢复上一次选区/光标集合，不撤销正文输入。 | cursorUndo<br>固定版本源码已核实 [CURSOR] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.multicursor.lineEnds`<br>在所选行末添加光标 | Shift+Alt+I | sourceEditorFocus && !imeComposing | 在每个被选中的逻辑行末尾添加光标，空行位置有效。 | editor.action.insertCursorAtEndOfEachLineSelected<br>固定版本源码已核实 [MULTI] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.selection.column`<br>矩形选择 | Ctrl+Shift+Alt+Left<br>Ctrl+Shift+Alt+Right<br>Ctrl+Shift+Alt+Up<br>Ctrl+Shift+Alt+Down<br>Ctrl+Shift+Alt+PageUp<br>Ctrl+Shift+Alt+PageDown<br>Shift+Alt+Drag | sourceEditorFocus && !imeComposing | 按显示列建立矩形范围；Tab跨度与短行按视觉位置映射，不自动补入空格。 | 见variants；columnSelect<br>固定版本源码已核实 [CORE] [OPTIONS] [VIEWCTRL] [MOUSE] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |

### 查找

| 命令ID / 中文名 | Windows默认键 | 生效上下文 | 功能规则 | VS Code对照 / 证据 | Markdown行为 / 冲突 | 优先级 |
|---|---|---|---|---|---|---|
| `texzi.find.open`<br>查找 | Ctrl+F | editorFocus && !imeComposing | 源码搜索原始文本；WYSIWYG保持排版态搜索可见正文含代码块，栏内标正文，不搜索Markdown标记。 | actions.find<br>固定版本源码已核实 [FIND] | 保持WYSIWYG；显式显示搜索范围“正文”。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.find.replaceOpen`<br>替换 | Ctrl+H | editorFocus && !imeComposing | 展开替换行；WYSIWYG切源码并提示范围变为源码，重算匹配数。 | editor.action.startFindReplaceAction<br>固定版本源码已核实 [FIND] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.find.next`<br>下一个匹配 | F3 / Enter | findWidgetVisible && !imeComposing；各替代键上下文见第5节 | 跳到下一个匹配；是否循环由search.wrapAround决定。 | editor.action.nextMatchFindAction<br>固定版本源码已核实 [FIND] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.find.previous`<br>上一个匹配 | Shift+F3 / Shift+Enter | findWidgetVisible && !imeComposing；各替代键上下文见第5节 | 跳到上一个匹配。 | editor.action.previousMatchFindAction<br>固定版本源码已核实 [FIND] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.find.toggleCase`<br>区分大小写 | Alt+C | findWidgetVisible && !imeComposing | 切换大小写匹配并重新计算结果。 | toggleFindCaseSensitive<br>固定版本源码已核实 [FIND] [FINDMODEL] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>仅查找栏可见时消费Alt+C；菜单助记键在其他上下文正常工作。 | P0 |
| `texzi.find.toggleWholeWord`<br>全字/部分匹配 | Alt+W | findWidgetVisible && !imeComposing | 切换全字；关闭即部分匹配。词边界不等同中文语义分词。 | toggleFindWholeWord<br>固定版本源码已核实 [FIND] [FINDMODEL] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.find.toggleRegex`<br>正则表达式 | Alt+R | findWidgetVisible && !imeComposing | 切换PCRE2正则模式；语法错误即时显示、不启动替换。WYSIWYG开启时转源码并重算。 | toggleFindRegex<br>固定版本源码已核实 [FIND] [FINDMODEL] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.find.toggleSelection`<br>仅在选区中查找 | Alt+L | findWidgetVisible && !imeComposing | 锁定当前非空选区为搜索范围，编辑后用锚点追踪边界；无选区禁用。 | toggleFindInSelection<br>固定版本源码已核实 [FIND] [FINDMODEL] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.find.replaceOne`<br>替换当前匹配 | Ctrl+Shift+1<br>Enter | findWidgetVisible && !imeComposing && writable && validPattern | 替换当前匹配，一次事务；按同一快照确定下一匹配。零宽枚举后前进一个Unicode scalar（不是grapheme或UTF-16码元），EOF零宽最多一次，防止原地循环。 | editor.action.replaceOne<br>固定版本源码已核实 [FIND] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>Enter仅在替换输入框执行替换；查找输入框Enter是下一个匹配。 | P0 |
| `texzi.find.replaceAll`<br>替换全部匹配 | Ctrl+Alt+Enter | findWidgetVisible && !imeComposing && writable && validPattern | 以同一文档快照计算替换，一次原子事务；可一次撤销，不跨文件执行。 | editor.action.replaceAll<br>固定版本源码已核实 [FIND] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.find.selectAllMatches`<br>选中全部查找匹配 | Alt+Enter | findWidgetVisible && !imeComposing | 源码建立匹配选区集合；WYSIWYG先切源码并重新搜索，再建立选区。 | editor.action.selectAllMatches<br>固定版本源码已核实 [FIND] | WYSIWYG执行时先原位切换源码，映射主选区/视口后执行；模式切换不进入撤销栈。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.find.close`<br>关闭查找栏 | Escape / Shift+Escape | findWidgetVisible && !imeComposing | 关闭查找栏，保留查询状态但移除临时匹配层，焦点回正文。 | closeFindWidget<br>固定版本源码已核实 [FIND] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>IME候选/组合拥有Escape优先权；关闭最上层菜单/弹层后才关闭查找。 | P0 |
| `texzi.find.inFolder`<br>在文件夹中查找 | Ctrl+Shift+F | windowFocus && !modalOpen && !imeComposing && workspaceOpen | 搜索当前根内文本文件，遵循排除规则/结果上限，可取消；点击结果定位源码。 | workbench.action.findInFiles<br>固定版本源码已核实 [SEARCHFIND] [SEARCHCONST] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |

### 标签

| 命令ID / 中文名 | Windows默认键 | 生效上下文 | 功能规则 | VS Code对照 / 证据 | Markdown行为 / 冲突 | 优先级 |
|---|---|---|---|---|---|---|
| `texzi.tab.close`<br>关闭标签 | Ctrl+W / Ctrl+F4 | windowFocus && !modalOpen && !imeComposing && activeEditor | 关闭当前区域的文档视图；最后一个视图关闭时处理未保存内容。空编辑组可按同键关闭。 | workbench.action.closeActiveEditor<br>固定版本源码已核实 [TABS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.tab.closeOthers`<br>关闭同组其他标签 | Unassigned | windowFocus && !modalOpen && !imeComposing && activeEditor | 仅关闭当前组其他标签，默认排除固定标签并在菜单注明；最后视图的dirty文档才确认。批量处理允许取消剩余关闭操作，不重复询问同一文档。 | workbench.action.closeOtherEditors<br>固定版本源码已核实 [TABS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.tab.closeAll`<br>关闭窗口所有标签 | Ctrl+K Ctrl+W | windowFocus && !modalOpen && !imeComposing | 关闭当前窗口所有组中的标签；不影响其他原生窗口。 | workbench.action.closeAllEditors<br>固定版本源码已核实 [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.tab.reopenClosed`<br>重新打开已关闭标签 | Ctrl+Shift+T | windowFocus && !modalOpen && !imeComposing | 恢复最近关闭的文档视图及位置；若文件不存在显示恢复选择。 | workbench.action.reopenClosedEditor<br>固定版本源码已核实 [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.tab.keepOpen`<br>将预览标签转为常驻 | Ctrl+K Enter | windowFocus && !modalOpen && !imeComposing && previewTab | 取消预览复用标记；与固定到标签栏左侧的“固定”不同。 | workbench.action.keepEditor<br>固定版本源码已核实 [TABS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.tab.pin`<br>固定/取消固定标签 | Ctrl+K Shift+Enter | windowFocus && !modalOpen && !imeComposing && activeEditor | 切换标签固定状态，固定标签置于组内左侧。 | workbench.action.pinEditor / workbench.action.unpinEditor<br>固定版本源码已核实 [TABS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.tab.next`<br>下一个标签 | Ctrl+PageDown | windowFocus && !modalOpen && !imeComposing && activeEditor | 按组内显示顺序向后循环；texzi范围限定当前组。 | workbench.action.nextEditor<br>源码键已核实；texzi范围调整 [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.tab.previous`<br>上一个标签 | Ctrl+PageUp | windowFocus && !modalOpen && !imeComposing && activeEditor | 按组内显示顺序向前循环。 | workbench.action.previousEditor<br>源码键已核实；texzi范围调整 [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.tab.recent`<br>按最近使用顺序切换标签 | Ctrl+Tab / Ctrl+Shift+Tab | windowFocus && !modalOpen && !imeComposing && activeEditor | 按住Ctrl展示MRU列表，Tab向前，Shift+Tab向后，松开Ctrl确认。 | workbench.action.quickOpenPreviousRecentlyUsedEditorInGroup / workbench.action.quickOpenLeastRecentlyUsedEditorInGroup<br>固定版本源码已核实 [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.tab.moveLeft`<br>将标签左移 | Ctrl+Shift+PageUp | windowFocus && !modalOpen && !imeComposing && activeEditor | 在当前组内左移一个位置，不跨固定/非固定区边界。 | workbench.action.moveEditorLeftInGroup<br>固定版本源码已核实 [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.tab.moveRight`<br>将标签右移 | Ctrl+Shift+PageDown | windowFocus && !modalOpen && !imeComposing && activeEditor | 在当前组内右移一个位置。 | workbench.action.moveEditorRightInGroup<br>固定版本源码已核实 [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |

### 分屏分窗

| 命令ID / 中文名 | Windows默认键 | 生效上下文 | 功能规则 | VS Code对照 / 证据 | Markdown行为 / 冲突 | 优先级 |
|---|---|---|---|---|---|---|
| `texzi.group.splitRight`<br>向右拆分 | Ctrl+\ | windowFocus && !modalOpen && !imeComposing && activeEditor | 在右侧新建编辑组，共享当前文档缓冲区，各自选区/滚动/模式独立。 | workbench.action.splitEditor<br>源码键已核实；texzi固定方向 [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.group.splitDown`<br>向下拆分 | Ctrl+K Ctrl+\ | windowFocus && !modalOpen && !imeComposing && activeEditor | 在下方新建编辑组；与默认右拆分方向正交，超过最小可用尺寸时提示。 | workbench.action.splitEditorOrthogonal<br>源码键已核实；texzi固定方向 [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.group.focusIndex`<br>聚焦编辑组 | Ctrl+1 / Ctrl+2 / Ctrl+3 / Ctrl+4 / Ctrl+5 / Ctrl+6 / Ctrl+7 / Ctrl+8 | windowFocus && !modalOpen && !imeComposing | 按视觉从左到右、从上到下编号；不存在的组不创建新文档。 | 见variants<br>固定版本源码已核实 [TABS] [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.group.focusDirection`<br>聚焦相邻编辑组 | Ctrl+K Ctrl+Left / Ctrl+K Ctrl+Right / Ctrl+K Ctrl+Up / Ctrl+K Ctrl+Down | windowFocus && !modalOpen && !imeComposing | 按空间方向寻找相邻组；没有邻居时保持原组。 | workbench.action.focusLeftGroup / focusRightGroup / focusAboveGroup / focusBelowGroup<br>固定版本源码已核实 [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.group.joinAll`<br>合并为单一编辑组 | Unassigned | windowFocus && !modalOpen && !imeComposing | 按当前视觉顺序合并标签；同一文档重复视图合并但不丢未保存缓冲区。 | workbench.action.joinAllGroups<br>固定版本源码已核实 [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.tab.moveToNewWindow`<br>将标签移到新窗口 | Unassigned | windowFocus && !modalOpen && !imeComposing && activeEditor | 移动当前视图到新的原生窗口；未保存文档保持同一ID和恢复记录。 | workbench.action.moveEditorToNewWindow<br>固定版本源码已核实 [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.tab.copyToNewWindow`<br>在新窗口打开同一文档 | Ctrl+K O | windowFocus && !modalOpen && !imeComposing && activeEditor | 新窗口新建同一文档的视图，原窗口保留；不是复制为新文件。 | workbench.action.copyEditorToNewWindow<br>固定版本源码已核实 [GROUPS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>第二段是O；Ctrl+K Ctrl+O是打开文件夹。 | P0 |

### 界面

| 命令ID / 中文名 | Windows默认键 | 生效上下文 | 功能规则 | VS Code对照 / 证据 | Markdown行为 / 冲突 | 优先级 |
|---|---|---|---|---|---|---|
| `texzi.sidebar.toggle`<br>显示/隐藏侧栏 | Ctrl+B / Ctrl+K Ctrl+B | windowFocus && !modalOpen && !imeComposing；各替代键上下文见第5节 | 切换文件夹侧栏；Ctrl+K Ctrl+B在所有正文模式通用。 | workbench.action.toggleSidebarVisibility<br>源码键已核实；texzi上下文改写 [LAYOUT] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>WYSIWYG正文Ctrl+B用于加粗；全局组合键为texzi建议，菜单显示当前上下文可用键。 | P0 |
| `texzi.view.toggleZen`<br>进入/退出Zen | Ctrl+K Z | windowFocus && !modalOpen && !imeComposing | 临时收起标签/侧栏/状态栏并按Zen设置居中；退出恢复原布局。 | workbench.action.toggleZenMode<br>固定版本源码已核实 [ZEN] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>第二段为Z不是Ctrl+Z；不抢正文Ctrl+Z撤销。 | P0 |
| `texzi.view.exitZen`<br>退出Zen | Escape Escape | windowFocus && !modalOpen && !imeComposing && zenActive | 连续两次Escape退出；有IME、菜单、查找时先处理更高优先级，不把消除弹层算成双击退出。 | workbench.action.exitZenMode<br>固定版本源码已核实 [ZEN] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.view.toggleFullScreen`<br>切换全屏 | F11 | windowFocus && !modalOpen && !imeComposing | 切换当前原生窗口全屏；与Zen是独立状态。 | workbench.action.toggleFullScreen<br>固定版本源码已核实 [WINDOW] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.editor.toggleWrap`<br>切换自动换行 | Alt+Z | sourceEditorFocus && !imeComposing | 只切换当前文档软换行，不修改实际换行；大文件保护生效时说明限制。 | editor.action.toggleWordWrap<br>固定版本源码已核实 [WRAP] | WYSIWYG正文始终按可用宽度排版；此命令在源码上下文生效。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.settings.open`<br>打开设置 | Ctrl+, | windowFocus && !modalOpen && !imeComposing | 在当前窗口打开单个设置页；已有设置页则聚焦，不重复创建。 | workbench.action.openSettings<br>固定版本源码已核实 [PREFS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.settings.keybindings`<br>打开键盘快捷键 | Ctrl+K Ctrl+S | windowFocus && !modalOpen && !imeComposing | 打开快捷键管理；按命令/键组合搜索，显示上下文与冲突。 | workbench.action.openGlobalKeybindings<br>固定版本源码已核实 [PREFS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.commandPalette.open`<br>打开命令面板 | Ctrl+Shift+P<br>F1 | windowFocus && !modalOpen && !imeComposing | 搜索所有可用命令；禁用项显示原因，不混入IDE命令。 | workbench.action.showCommands<br>固定版本源码已核实 [PALETTE] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.editor.toggleTabFocus`<br>Tab切换焦点模式 | Ctrl+M | editorFocus && !imeComposing | 切换Tab是在编辑器内缩进/导航结构，还是移到下一个界面控件；状态栏提示当前模式。 | editor.action.toggleTabFocusMode<br>固定版本源码已核实 [TABFOCUS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.file.changeLanguage`<br>更改语言高亮模式 | Ctrl+K M | windowFocus && !modalOpen && !imeComposing && activeEditor | 切换当前文档语言标记，只影响高亮/Markdown视图可用性，不转换文件。 | workbench.action.editor.changeLanguageMode<br>固定版本源码已核实 [EDITORSTATUS] | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |

### Markdown

| 命令ID / 中文名 | Windows默认键 | 生效上下文 | 功能规则 | VS Code对照 / 证据 | Markdown行为 / 冲突 | 优先级 |
|---|---|---|---|---|---|---|
| `texzi.markdown.toggleSource`<br>切换Markdown编辑/源码 | Unassigned | editorFocus && !imeComposing && languageMarkdown | 正文右上>_切换当前区域；文档版本、未保存标记、撤销历史不变，选区与视口按源码锚点映射。 | 无直接对照<br>texzi新增/建议  | WYSIWYG与源码双向切换；纯文本/代码文件不显示此按钮。<br>默认未绑定；不借用VSCode只读Markdown预览快捷键冒充。 | P0 |
| `texzi.markdown.bold`<br>切换加粗 | Ctrl+B | markdownWysiwygFocus && writable && !imeComposing | 对选区切换粗体，空选区切换后续输入格式；以源码局部事务记录。 | 无直接对照<br>texzi新增/建议  | 仅在WYSIWYG正文上下文执行，修改通过局部源码事务进入共享撤销历史；源码模式可直接编辑标记或使用>_进入排版。<br>覆盖此上下文中的侧栏Ctrl+B；侧栏可用Ctrl+K Ctrl+B。 | P0 |
| `texzi.markdown.italic`<br>切换斜体 | Ctrl+I | markdownWysiwygFocus && writable && !imeComposing | 切换选区/后续输入斜体；不启用代码建议等IDE功能。 | 无直接对照<br>texzi新增/建议  | 仅在WYSIWYG正文上下文执行，修改通过局部源码事务进入共享撤销历史；源码模式可直接编辑标记或使用>_进入排版。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.markdown.insertLink`<br>插入或编辑链接 | Unassigned | markdownWysiwygFocus && writable && !imeComposing | 工具栏/菜单打开链接浮层；确认时只修改目标源范围。 | 无直接对照<br>texzi新增/建议  | 仅在WYSIWYG正文上下文执行，修改通过局部源码事务进入共享撤销历史；源码模式可直接编辑标记或使用>_进入排版。<br>不占用Ctrl+K前缀；默认未绑定。 | P0 |
| `texzi.markdown.toggleSyncScroll`<br>切换关联区域同步滚动 | Unassigned | windowFocus && !modalOpen && !imeComposing && linkedMarkdownViews | 切换同文档源码/WYSIWYG区域的锚点同步；不强制两个区域同模式。 | 无直接对照<br>texzi新增/建议  | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |
| `texzi.markdown.pasteMarkdown`<br>粘贴Markdown | Unassigned | markdownWysiwygFocus && writable && !imeComposing | 显式读取剪贴板Markdown格式，缺少时读取纯文字作为Markdown源片段；解析为首版支持结构后一次插入。未知语法保留原样片段，不从任意HTML推断无损结构；失败不部分插入。 | 无直接对照<br>texzi新增/建议  | 仅在WYSIWYG当前文档上下文执行；内容修改均为一次局部源码事务，可一次撤销。<br>默认未绑定；通过上下文菜单、格式按钮或命令面板调用，遵守IME与焦点优先级。 | P0 |
| `texzi.markdown.copyMarkdown`<br>复制Markdown | Unassigned | editorFocus && languageMarkdown && nonEmptySelection && !imeComposing | 复制选区对应的原始Markdown片段。源码选区直接取源范围；WYSIWYG通过SourceMap取覆盖当前选择的最小连续源范围，包含范围内必要标记；无法可靠映射则禁用并给源码入口。无选区时禁用，全篇可先全选。 | 无直接对照<br>texzi新增/建议  | 两种Markdown模式可用，只读也可复制，不修改文档或撤销历史。<br>默认未绑定；通过上下文菜单、格式按钮或命令面板调用，遵守IME与焦点优先级。 | P0 |
| `texzi.markdown.exitCodeBlock`<br>退出代码块 | Unassigned | markdownWysiwygFocus && writable && !imeComposing && codeBlockFocus | 将插入点移至后续可编辑块开头；文末没有后续可编辑块时，在代码块后创建一个空段落并移入。纯焦点移动不生成撤销步，创建段落为一个事务。 | 无直接对照<br>texzi新增/建议  | 仅在WYSIWYG当前文档上下文执行；内容修改均为一次局部源码事务，可一次撤销。<br>默认未绑定；通过上下文菜单、格式按钮或命令面板调用，遵守IME与焦点优先级。 | P0 |
| `texzi.markdown.strikethrough`<br>切换删除线 | Unassigned | markdownWysiwygFocus && writable && !imeComposing | 切换选区或后续输入的删除线标记，只修改所需定界符；与输入法未提交文字隔离。 | 无直接对照<br>texzi新增/建议  | 仅在WYSIWYG当前文档上下文执行；内容修改均为一次局部源码事务，可一次撤销。<br>默认未绑定；通过上下文菜单、格式按钮或命令面板调用，遵守IME与焦点优先级。 | P0 |
| `texzi.markdown.inlineCode`<br>切换行内代码 | Unassigned | markdownWysiwygFocus && writable && !imeComposing | 切换选区或后续输入的行内代码格式；根据内容中的反引号选择足够的定界符长度，保留节点外原文。跨不支持的块选择时禁用并解释。 | 无直接对照<br>texzi新增/建议  | 仅在WYSIWYG当前文档上下文执行；内容修改均为一次局部源码事务，可一次撤销。<br>默认未绑定；通过上下文菜单、格式按钮或命令面板调用，遵守IME与焦点优先级。 | P0 |
| `texzi.markdown.toggleTask`<br>切换任务完成状态 | Unassigned | markdownWysiwygFocus && writable && !imeComposing && taskItemFocus | 切换当前任务框勾选，仅修改该项任务标记，保留缩进与正文。按钮点击与键盘调用使用同一事务。 | 无直接对照<br>texzi新增/建议  | 仅在WYSIWYG当前文档上下文执行；内容修改均为一次局部源码事务，可一次撤销。<br>默认未绑定；通过上下文菜单、格式按钮或命令面板调用，遵守IME与焦点优先级。 | P0 |
| `texzi.markdown.setCodeLanguage`<br>设置代码块语言 | Unassigned | markdownWysiwygFocus && writable && !imeComposing && codeBlockFocus | 打开语言选择，更新当前围栏语言标记并保留其他可识别信息；只影响高亮，不执行或格式化代码。 | 无直接对照<br>texzi新增/建议  | 仅在WYSIWYG当前文档上下文执行；内容修改均为一次局部源码事务，可一次撤销。<br>默认未绑定；通过上下文菜单、格式按钮或命令面板调用，遵守IME与焦点优先级。 | P0 |
| `texzi.markdown.table.insertRow`<br>插入表格行 | Unassigned | markdownWysiwygFocus && writable && !imeComposing && tableCellFocus | 参数position=before/after，默认after；在当前正文行前/后插入空行，聚焦对应列。表头的before不可用，after插入首个正文行。整次结构修改一个事务。 | 无直接对照<br>texzi新增/建议  | 仅在WYSIWYG当前文档上下文执行；内容修改均为一次局部源码事务，可一次撤销。<br>默认未绑定；通过上下文菜单、格式按钮或命令面板调用，遵守IME与焦点优先级。 | P0 |
| `texzi.markdown.table.deleteRow`<br>删除表格行 | Unassigned | markdownWysiwygFocus && writable && !imeComposing && tableCellFocus | 删除当前行；删除表头时首个正文行升为表头。删除表格最后一行时移除该表格并留下可输入空段落；一次撤销恢复完整表格与选区。 | 无直接对照<br>texzi新增/建议  | 仅在WYSIWYG当前文档上下文执行；内容修改均为一次局部源码事务，可一次撤销。<br>默认未绑定；通过上下文菜单、格式按钮或命令面板调用，遵守IME与焦点优先级。 | P0 |
| `texzi.markdown.table.insertColumn`<br>插入表格列 | Unassigned | markdownWysiwygFocus && writable && !imeComposing && tableCellFocus | 参数position=before/after，默认after；在当前列前/后插入空列，所有行保持矩形。新列默认未指定对齐，聚焦新列；一次事务。 | 无直接对照<br>texzi新增/建议  | 仅在WYSIWYG当前文档上下文执行；内容修改均为一次局部源码事务，可一次撤销。<br>默认未绑定；通过上下文菜单、格式按钮或命令面板调用，遵守IME与焦点优先级。 | P0 |
| `texzi.markdown.table.deleteColumn`<br>删除表格列 | Unassigned | markdownWysiwygFocus && writable && !imeComposing && tableCellFocus | 删除当前列并映射单元格选区；删除最后一列时移除表格并留下可输入空段落。一次撤销恢复完整源片段。 | 无直接对照<br>texzi新增/建议  | 仅在WYSIWYG当前文档上下文执行；内容修改均为一次局部源码事务，可一次撤销。<br>默认未绑定；通过上下文菜单、格式按钮或命令面板调用，遵守IME与焦点优先级。 | P0 |
| `texzi.markdown.table.alignColumn`<br>设置表格列对齐 | Unassigned | markdownWysiwygFocus && writable && !imeComposing && tableCellFocus | 参数alignment=none/left/center/right；修改当前列对应分隔单元的冒号对齐标记，保留其他列及表格外原文；一次事务。 | 无直接对照<br>texzi新增/建议  | 仅在WYSIWYG当前文档上下文执行；内容修改均为一次局部源码事务，可一次撤销。<br>默认未绑定；通过上下文菜单、格式按钮或命令面板调用，遵守IME与焦点优先级。 | P0 |

### 恢复

| 命令ID / 中文名 | Windows默认键 | 生效上下文 | 功能规则 | VS Code对照 / 证据 | Markdown行为 / 冲突 | 优先级 |
|---|---|---|---|---|---|---|
| `texzi.recovery.open`<br>打开恢复中心 | Unassigned | windowFocus && !modalOpen && !imeComposing | 列出恢复副本与保存失败记录，可逐一打开、另存、比较或明确丢弃。 | 无直接对照<br>texzi新增/建议  | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |

### 大文件

| 命令ID / 中文名 | Windows默认键 | 生效上下文 | 功能规则 | VS Code对照 / 证据 | Markdown行为 / 冲突 | 优先级 |
|---|---|---|---|---|---|---|
| `texzi.performance.reassessDocument`<br>重新评估当前文档模式 | Unassigned | windowFocus && !modalOpen && !imeComposing && activeEditor | 根据规模和选项重新选择标准编辑/大文件轻量编辑；尝试完整模式前显示代价并可取消。 | 无直接对照<br>texzi新增/建议  | 两种Markdown编辑模式均可用，按当前文档事务执行。<br>遵守全局路由与IME规则；无同上下文默认冲突。 | P0 |

## 4. 参数命令逐项默认键

### 移动光标（逐方向） — `texzi.cursor.move`

| 操作 | 参数 | Windows默认键 | VS Code命令ID | 核实 |
|---|---|---|---|---|
| 前一字符 | {"direction": "left"} | Left | cursorLeft | 固定版本源码已核实 [CORE] |
| 后一字符 | {"direction": "right"} | Right | cursorRight | 固定版本源码已核实 [CORE] |
| 上一视觉行 | {"direction": "up"} | Up | cursorUp | 固定版本源码已核实 [CORE] |
| 下一视觉行 | {"direction": "down"} | Down | cursorDown | 固定版本源码已核实 [CORE] |
| 行首/首个非空字符 | {"direction": "home"} | Home | cursorHome | 固定版本源码已核实 [CORE] |
| 行尾 | {"direction": "end"} | End | cursorEnd | 固定版本源码已核实 [CORE] |
| 文档开头 | {"direction": "documentStart"} | Ctrl+Home | cursorTop | 固定版本源码已核实 [CORE] |
| 文档结尾 | {"direction": "documentEnd"} | Ctrl+End | cursorBottom | 固定版本源码已核实 [CORE] |
| 上一屏 | {"direction": "pageUp"} | PageUp | cursorPageUp | 固定版本源码已核实 [CORE] |
| 下一屏 | {"direction": "pageDown"} | PageDown | cursorPageDown | 固定版本源码已核实 [CORE] |
| 前一词边界 | {"direction": "wordLeft"} | Ctrl+Left | cursorWordLeft | 固定版本源码已核实 [WORDS] |
| 后一词结尾 | {"direction": "wordRight"} | Ctrl+Right | cursorWordEndRight | 固定版本源码已核实 [WORDS] |

### 扩展选区（逐方向） — `texzi.selection.extend`

| 操作 | 参数 | Windows默认键 | VS Code命令ID | 核实 |
|---|---|---|---|---|
| 向前扩选 | {"direction": "left"} | Shift+Left | cursorLeftSelect | 固定版本源码已核实 [CORE] |
| 向后扩选 | {"direction": "right"} | Shift+Right | cursorRightSelect | 固定版本源码已核实 [CORE] |
| 向上扩选 | {"direction": "up"} | Shift+Up | cursorUpSelect | 固定版本源码已核实 [CORE] |
| 向下扩选 | {"direction": "down"} | Shift+Down | cursorDownSelect | 固定版本源码已核实 [CORE] |
| 扩选到行首 | {"direction": "home"} | Shift+Home | cursorHomeSelect | 固定版本源码已核实 [CORE] |
| 扩选到行尾 | {"direction": "end"} | Shift+End | cursorEndSelect | 固定版本源码已核实 [CORE] |
| 扩选到文档开头 | {"direction": "documentStart"} | Ctrl+Shift+Home | cursorTopSelect | 固定版本源码已核实 [CORE] |
| 扩选到文档结尾 | {"direction": "documentEnd"} | Ctrl+Shift+End | cursorBottomSelect | 固定版本源码已核实 [CORE] |
| 向上一屏扩选 | {"direction": "pageUp"} | Shift+PageUp | cursorPageUpSelect | 固定版本源码已核实 [CORE] |
| 向下一屏扩选 | {"direction": "pageDown"} | Shift+PageDown | cursorPageDownSelect | 固定版本源码已核实 [CORE] |
| 向前按词扩选 | {"direction": "wordLeft"} | Ctrl+Shift+Left | cursorWordLeftSelect | 固定版本源码已核实 [WORDS] |
| 向后按词扩选 | {"direction": "wordRight"} | Ctrl+Shift+Right | cursorWordEndRightSelect | 固定版本源码已核实 [WORDS] |

### 矩形选择 — `texzi.selection.column`

| 操作 | 参数 | Windows默认键 | VS Code命令ID | 核实 |
|---|---|---|---|---|
| 向左扩列 | {"direction": "left"} | Ctrl+Shift+Alt+Left | cursorColumnSelectLeft | 固定版本源码已核实 [CORE] |
| 向右扩列 | {"direction": "right"} | Ctrl+Shift+Alt+Right | cursorColumnSelectRight | 固定版本源码已核实 [CORE] |
| 向上扩行 | {"direction": "up"} | Ctrl+Shift+Alt+Up | cursorColumnSelectUp | 固定版本源码已核实 [CORE] |
| 向下扩行 | {"direction": "down"} | Ctrl+Shift+Alt+Down | cursorColumnSelectDown | 固定版本源码已核实 [CORE] |
| 向上一屏扩行 | {"direction": "pageUp"} | Ctrl+Shift+Alt+PageUp | cursorColumnSelectPageUp | 固定版本源码已核实 [CORE] |
| 向下一屏扩行 | {"direction": "pageDown"} | Ctrl+Shift+Alt+PageDown | cursorColumnSelectPageDown | 固定版本源码已核实 [CORE] |

### 聚焦编辑组 — `texzi.group.focusIndex`

| 操作 | 参数 | Windows默认键 | VS Code命令ID | 核实 |
|---|---|---|---|---|
| 聚焦第1组 | {"index": 1} | Ctrl+1 | workbench.action.focusFirstEditorGroup | 固定版本源码已核实 [TABS] [GROUPS] |
| 聚焦第2组 | {"index": 2} | Ctrl+2 | workbench.action.focusSecondEditorGroup | 固定版本源码已核实 [TABS] [GROUPS] |
| 聚焦第3组 | {"index": 3} | Ctrl+3 | workbench.action.focusThirdEditorGroup | 固定版本源码已核实 [TABS] [GROUPS] |
| 聚焦第4组 | {"index": 4} | Ctrl+4 | workbench.action.focusFourthEditorGroup | 固定版本源码已核实 [TABS] [GROUPS] |
| 聚焦第5组 | {"index": 5} | Ctrl+5 | workbench.action.focusFifthEditorGroup | 固定版本源码已核实 [TABS] [GROUPS] |
| 聚焦第6组 | {"index": 6} | Ctrl+6 | workbench.action.focusSixthEditorGroup | 固定版本源码已核实 [TABS] [GROUPS] |
| 聚焦第7组 | {"index": 7} | Ctrl+7 | workbench.action.focusSeventhEditorGroup | 固定版本源码已核实 [TABS] [GROUPS] |
| 聚焦第8组 | {"index": 8} | Ctrl+8 | workbench.action.focusEighthEditorGroup | 固定版本源码已核实 [TABS] [GROUPS] |

鼠标矩形选择另为 `Shift+Alt+Drag`。固定版本mouseHandler将Shift转换为选择模式，viewController结合Alt分派到columnSelect；默认多光标修饰键配置为Alt，已核实[OPTIONS] [MOUSE] [VIEWCTRL]。组合字符移动与列映射必须用源位置/字形位置映射；不能用UTF-16码元下标直接当光标停靠点。


### 相邻组与MRU参数映射补充

| 命令 | 参数 | Windows默认键 | VS Code命令ID |
|---|---|---|---|
| `texzi.tab.recent` | `{"direction": "next"}` | Ctrl+Tab | `workbench.action.quickOpenPreviousRecentlyUsedEditorInGroup` [GROUPS] |
| `texzi.tab.recent` | `{"direction": "previous"}` | Ctrl+Shift+Tab | `workbench.action.quickOpenLeastRecentlyUsedEditorInGroup` [GROUPS] |
| `texzi.group.focusDirection` | `{"direction": "left"}` | Ctrl+K Ctrl+Left | `workbench.action.focusLeftGroup` [GROUPS] |
| `texzi.group.focusDirection` | `{"direction": "right"}` | Ctrl+K Ctrl+Right | `workbench.action.focusRightGroup` [GROUPS] |
| `texzi.group.focusDirection` | `{"direction": "up"}` | Ctrl+K Ctrl+Up | `workbench.action.focusAboveGroup` [GROUPS] |
| `texzi.group.focusDirection` | `{"direction": "down"}` | Ctrl+K Ctrl+Down | `workbench.action.focusBelowGroup` [GROUPS] |

## 5. 同键与替代键的精确上下文

| 命令 | 键 | 条件 |
|---|---|---|
| texzi.find.next | F3 | editorFocus && findQueryExists && !imeComposing |
| texzi.find.next | Enter | findInputFocus && !imeComposing |
| texzi.find.previous | Shift+F3 | editorFocus && findQueryExists && !imeComposing |
| texzi.find.previous | Shift+Enter | findInputFocus && !imeComposing |
| texzi.find.replaceOne | Ctrl+Shift+1 | findWidgetVisible && writable && validPattern && !imeComposing |
| texzi.find.replaceOne | Enter | replaceInputFocus && writable && validPattern && !imeComposing |
| texzi.sidebar.toggle | Ctrl+B | windowFocus && !markdownWysiwygFocus && !modalOpen && !imeComposing |
| texzi.sidebar.toggle | Ctrl+K Ctrl+B | windowFocus && !modalOpen && !imeComposing |

其他规则：Escape优先处理IME，其次关闭最上层菜单/浮层，再关闭查找，最后才参与Zen退出序列；关闭查找不触发正文删除。Native对话框期间应用组合键停止派发。只读模式保留导航、选择、复制与查找，剪切/删除/替换显示禁用原因。

### Windows剪贴板替代键

VS Code剪切还有Shift+Delete、复制还有Ctrl+Insert、粘贴还有Shift+Insert。固定版本clipboard源码已验证这些替代键[CLIP]。texzi同时启用这些替代键。它们只作用于当前聚焦的文本控件，不越级触发文件树删除或正文编辑。

## 6. 编辑语义的实现契约

1. 每次命令先解析目标文档ID、版本、选择集合和只读状态。多光标所有替换针对同一旧版本计算，重叠范围合并，从后向前或通过等价的事务机制应用，避免偏移漂移。
2. 连续输入可按明确时间窗合并撤销，但移动光标、粘贴、搜索替换、切换输入来源须分隔事务。每次IME组合提交作为一个正文事务；组合中的候选变更不制造撤销噪声。
3. WYSIWYG/source共享撤销栈、未保存状态和文档revision。视图切换不触发序列化回写。源模式才能表达的多光标/矩形操作先以源码锚点原位切换再执行，不能静默无效。
4. WYSIWYG普通查找按可见正文范围；正则/替换按源码范围。范围改变后不得沿用旧匹配索引、旧计数或旧选区偏移。源正则采用PCRE2 UTF/UCP并设置执行预算；语法错误、超时和取消是不同状态。
5. 部分匹配对应全字关闭；全字不是完整行，不是必须和整个文档相等。大小写开关独立。正则与全字同时开启时以正则结果的边界验证，不把用户表达式字符串拼进不透明包装。
6. 替换支持明确的捕获组引用规则，实际语法在技术方案固定；UI同时显示正则错误与替换模板错误。替换全部先基于同一快照生成补丁列表，再一次提交，不在后台逐次改变可见文档。
7. 空串查找不启动全文件零长度枚举；合法零长度正则匹配必须前进一个Unicode scalar，EOF零宽最多一次；此枚举规则与UI按grapheme移动光标分开。超长匹配/大量光标触发预算提示，不能冻结窗口。
8. 每个文档保存一次，所有视图观察相同版本；每个区域独立持有选择、滚动、模式和折叠状态。移动到新窗口与复制视图不产生第二套互相覆盖的正文缓存。

8. 普通WYSIWYG粘贴按纯文字，不自动解析Markdown或HTML。显式粘贴Markdown、复制Markdown、按行分配粘贴及表格结构动作使用独立命令；表格动作参数枚举在JSON的argsSchema与表内行为中列明，均默认未绑定。
9. 打开、替换或关闭根文件夹不关闭已打开标签。窗口关闭允许保留dirty内容的条件是两个自动恢复开关同时开启且快照刷盘成功；显式关闭最后dirty标签仍执行保存/放弃/取消。

## 7. 快捷键管理界面与验收

快捷键页按命令名、ID、键组合搜索。每行显示默认/用户覆盖、上下文、来源核实状态和冲突。录制键先显示候选并列出将覆盖的同上下文命令；按Escape退出录制。恢复默认可逐条或整组进行。系统保留键直接说明不可绑定，不尝试通过钩子抢占。

- 在Win10 x64微软拼音、新版/兼容模式、小狼毫上，用未完成拼音串逐一触发Esc、Enter、Backspace、方向与候选数字，不能执行正文命令；具体IME矩阵以验收文件为准。
- 在源码、WYSIWYG、查找输入、替换输入、设置输入、文件树、模态对话框分别验证Ctrl+A/C/X/V/Z和Escape的目标唯一性。
- 验证所有Ctrl+K双段组合：第二段有/无Ctrl不同，超时/取消/错误第二键不意外写入正文。
- 验证Ctrl+B在两种编辑模式的不同路由及统一侧栏组合键；标签、菜单的快捷键提示随上下文正确显示。
- 验证中文、emoji代理对、组合变音、ZWJ序列、CRLF、Tab、软换行和超长行上的移动/选择/矩形映射，不能出现半字符选区。
- 多光标输入、剪切、粘贴和替换全部均可一次撤销；Ctrl+U只撤销光标操作；切换源码后历史不重置。
- 多窗口共享同文档，移动/复制视图后编辑、保存、外部重载与关闭行为保持一致。
- 正式实现冻结前，使用固定版本干净配置导出的Windows默认键表进行运行时复核，特别检查系统键盘布局和IME设置的影响；本次已完成源码核实，但不宣称已运行Windows真机默认键导出。后续新增而未核实的项必须显式标“固定版本默认待核实”。

## 8. 一手来源

[CORE]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/editor/browser/coreCommands.ts "VS Code 1.146.0 src/vs/editor/browser/coreCommands.ts"
[EXT]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/editor/browser/editorExtensions.ts "VS Code 1.146.0 src/vs/editor/browser/editorExtensions.ts"
[WORDS]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/editor/contrib/wordOperations/browser/wordOperations.ts "VS Code 1.146.0 src/vs/editor/contrib/wordOperations/browser/wordOperations.ts"
[LINES]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/editor/contrib/linesOperations/browser/linesOperations.ts "VS Code 1.146.0 src/vs/editor/contrib/linesOperations/browser/linesOperations.ts"
[MULTI]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/editor/contrib/multicursor/browser/multicursor.ts "VS Code 1.146.0 src/vs/editor/contrib/multicursor/browser/multicursor.ts"
[CURSOR]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/editor/contrib/cursorUndo/browser/cursorUndo.ts "VS Code 1.146.0 src/vs/editor/contrib/cursorUndo/browser/cursorUndo.ts"
[CLIP]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/editor/contrib/clipboard/browser/clipboard.ts "VS Code 1.146.0 src/vs/editor/contrib/clipboard/browser/clipboard.ts"
[LINESEL]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/editor/contrib/lineSelection/browser/lineSelection.ts "VS Code 1.146.0 src/vs/editor/contrib/lineSelection/browser/lineSelection.ts"
[FIND]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/editor/contrib/find/browser/findController.ts "VS Code 1.146.0 src/vs/editor/contrib/find/browser/findController.ts"
[FINDMODEL]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/editor/contrib/find/browser/findModel.ts "VS Code 1.146.0 src/vs/editor/contrib/find/browser/findModel.ts"
[FILE]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/contrib/files/browser/fileCommands.ts "VS Code 1.146.0 src/vs/workbench/contrib/files/browser/fileCommands.ts"
[FILECONST]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/contrib/files/browser/fileConstants.ts "VS Code 1.146.0 src/vs/workbench/contrib/files/browser/fileConstants.ts"
[WORKSPACE]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/browser/actions/workspaceActions.ts "VS Code 1.146.0 src/vs/workbench/browser/actions/workspaceActions.ts"
[TABS]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/browser/parts/editor/editorCommands.ts "VS Code 1.146.0 src/vs/workbench/browser/parts/editor/editorCommands.ts"
[GROUPS]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/browser/parts/editor/editorActions.ts "VS Code 1.146.0 src/vs/workbench/browser/parts/editor/editorActions.ts"
[WINDOW]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/browser/actions/windowActions.ts "VS Code 1.146.0 src/vs/workbench/browser/actions/windowActions.ts"
[WINNATIVE]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/electron-browser/actions/windowActions.ts "VS Code 1.146.0 src/vs/workbench/electron-browser/actions/windowActions.ts"
[LAYOUT]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/browser/actions/layoutActions.ts "VS Code 1.146.0 src/vs/workbench/browser/actions/layoutActions.ts"
[PREFS]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/contrib/preferences/browser/preferences.contribution.ts "VS Code 1.146.0 src/vs/workbench/contrib/preferences/browser/preferences.contribution.ts"
[GOTO]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/contrib/codeEditor/browser/quickaccess/gotoLineQuickAccess.ts "VS Code 1.146.0 src/vs/workbench/contrib/codeEditor/browser/quickaccess/gotoLineQuickAccess.ts"
[QUICK]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/browser/actions/quickAccessActions.ts "VS Code 1.146.0 src/vs/workbench/browser/actions/quickAccessActions.ts"
[WRAP]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/contrib/codeEditor/browser/toggleWordWrap.ts "VS Code 1.146.0 src/vs/workbench/contrib/codeEditor/browser/toggleWordWrap.ts"
[SEARCH]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/contrib/search/browser/search.contribution.ts "VS Code 1.146.0 src/vs/workbench/contrib/search/browser/search.contribution.ts"
[RELEASE]: https://code.visualstudio.com/updates/v1_146 "VS Code 1.146 Stable（2026-11-11，下载版本1.146.0）"
[TAG]: https://api.github.com/repos/microsoft/vscode/git/ref/tags/1.146.0 "1.146.0 tag指向固定commit"
[REF]: https://code.visualstudio.com/docs/reference/default-keybindings "官方Windows默认快捷键参考（动态页面，不能单独证明固定版本）"
[PDF]: https://code.visualstudio.com/shortcuts/keyboard-shortcuts-windows.pdf "官方Windows快捷键PDF（动态文件）"

[ZEN]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/browser/workbench.zenMode.contribution.ts "VS Code 1.146.0 src/vs/workbench/browser/workbench.zenMode.contribution.ts"
[PALETTE]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/contrib/quickaccess/browser/commandsQuickAccess.ts "VS Code 1.146.0 src/vs/workbench/contrib/quickaccess/browser/commandsQuickAccess.ts"
[FILENATIVE]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/contrib/files/electron-browser/fileActions.contribution.ts "VS Code 1.146.0 src/vs/workbench/contrib/files/electron-browser/fileActions.contribution.ts"
[TABFOCUS]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/editor/contrib/toggleTabFocusMode/browser/toggleTabFocusMode.ts "VS Code 1.146.0 src/vs/editor/contrib/toggleTabFocusMode/browser/toggleTabFocusMode.ts"
[EDITORSTATUS]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/browser/parts/editor/editorStatus.ts "VS Code 1.146.0 src/vs/workbench/browser/parts/editor/editorStatus.ts"
[SEARCHFIND]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/contrib/search/browser/searchActionsFind.ts "VS Code 1.146.0 src/vs/workbench/contrib/search/browser/searchActionsFind.ts"
[SEARCHCONST]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/workbench/contrib/search/common/constants.ts "VS Code 1.146.0 src/vs/workbench/contrib/search/common/constants.ts"
[OPTIONS]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/editor/common/config/editorOptions.ts "VS Code 1.146.0 src/vs/editor/common/config/editorOptions.ts"
[VIEWCTRL]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/editor/browser/view/viewController.ts "VS Code 1.146.0 src/vs/editor/browser/view/viewController.ts"

[MOUSE]: https://github.com/microsoft/vscode/blob/69047191fc427f2f92828279aae1d90d8eeda7ab/src/vs/editor/browser/controller/mouseHandler.ts "VS Code 1.146.0 src/vs/editor/browser/controller/mouseHandler.ts"

上述代码来自Microsoft官方microsoft/vscode仓库并固定到commit；官方PDF与网页是动态参考，冲突时以固定版本Windows注册源码和干净配置导出为准。用户要求覆盖的文本编辑规则由本文件定义；未纳入命令的IDE、扩展、终端、调试、Git与AI功能不属于首版。
