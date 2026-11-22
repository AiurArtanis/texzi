# texzi 完整设计交付包

> **2026-11-20 更新：** v1 的范围和验收只看 [docs/V1-SCOPE.md](../V1-SCOPE.md)。视觉部分仍以 DESIGN.md、UI_SPEC、concepts 和 prototype 为准。ACCEPTANCE、PERFORMANCE_PLAN、HANDOFF、VERIFICATION 已移到 [docs/archive/2026-11-20-gates/](../archive/2026-11-20-gates/)，下文提到 T00–T18、G0–G5 和 112 条验收的部分都已失效。

**版本 1.0 · 2026-11-15 · Windows 10 x64 · 已获授权进入完整设计阶段**

本包供产品审阅与另一个 Agent 团队实施。交付的是完整设计基线、可运行的浏览器交互原型、源文件和验证工具。原生 NSH 客户端将在实施阶段开发；本包没有将浏览器原型、概念图或上游性能报告当成 Windows 产品已达标的证据。

## 从这里开始

- **审阅设计**：打开根目录的 texzi-design-handbook.html。它包含完整文档、三张确认概念图、目录导航和章节搜索，可以离线阅读。
- **操作原型**：双击 start-preview.cmd，或直接打开 prototype/index.html。独立文件 texzi-prototype.html 与此版本一致，不需安装依赖。
- **交给 Agent 团队**：先阅读 docs/HANDOFF.md，再按 T00–T18 和 G0–G5 推进；不用再次确认已明确的 x64、Zen 或 WYSIWYG 方向。
- **了解验证边界**：阅读 docs/VERIFICATION.md 与 docs/PROTOTYPE_GUIDE.md，区分实际实现、演示状态、尚未执行的验收。

## 交付 A–E 对照

| 用户交付要求 | 包内位置 | 完整内容 |
|---|---|---|
| A 完整 PRD | docs/PRD.md | 用户场景、首版与非目标、99 个 FR、状态/异常/交互、优先级与需求映射 |
| A 设置 | docs/SETTINGS.md、data/settings.json | 63 项名称、作用、类型、值域、默认值、生效时机、依赖和作用域 |
| A 命令与默认键 | docs/COMMANDS.md、data/commands.json | 115 命令，Windows 默认键、上下文、冲突、参数、VS Code 1.146.0 固定源码证据 |
| B 功能与界面 | docs/UI_SPEC.md、concepts/ | 页面、组件、颜色、DIP 尺寸、主题、选择/查找/焦点、分屏/分窗、设置和异常 |
| C 高保真原型 | prototype/index.html、prototype/src/ | 三个主场景、真实编辑及搜索、状态演示、可运行源码和能力边界 |
| D 技术方案 | docs/TECHNICAL_DESIGN.md | NSH 实际能力、文本/富编辑内核、Windows、IME、编码、恢复、多窗口与风险 |
| D 实施与验收 | docs/HANDOFF.md、docs/ACCEPTANCE.md | 19 任务、依赖、团队分工、6 道 Gate、112 条产品验收及 16 项工程验收 |
| D 性能与工具 | docs/PERFORMANCE_PLAN.md、tools/bench/、tests/ | 已接受目标、额外建议、测试环境/样本/统计、生成器、逻辑测试和浏览器流程脚本 |
| E 未决问题 | docs/DECISIONS.md | 用户明确要求、设计建议、已关闭疑问、工程验证项和责任人 |
| 证据与追溯 | docs/SOURCES.md、data/source-index.json | 固定版本、事实来源、推断边界及可核对链接 |
| 参考产品核实 | docs/REFERENCE_RESEARCH.md | quiet.md实际能力、Markdown写回、Windows、性能口径及集成限制 |

## 已确定的产品基线

名称 **texzi**，面向 Windows 10 x64。以日常文本、Markdown 和代码片段为核心，保留现代编辑器的多标签、分屏、多窗口和文件夹工作方式。首版不加入完整 IDE、插件市场、云同步或文本敏感度识别。

窗口顶栏把文件、编辑、选择、查看、转到、帮助与搜索、窗口控制放在同一行，左上不重复绘制应用标题文字。Markdown 默认所见即所得；正文右上 **>_** 在当前区域原位切换源码。Z 已确认为 **Zen**。

完整正文、保存点和撤销历史属于 Document；标签、光标、滚动和编辑模式属于 View。同一文件在多个区域/窗口打开不生成互相竞争的内容副本。切模式零文本事务，未编辑源片段保留原字节；未知 Markdown 语法保留为可定位的原文块。

## 原型运行

最便捷方式是用 Windows 桌面浏览器打开 prototype/index.html。建议桌面窗口至少 1280×800、100% 页面缩放。原型菜单动作可以替代被浏览器占用的 Ctrl+W、Ctrl+N 等按键。

如果浏览器对本地文件的 Worker、下载或存储有限制，可在你自己的电脑使用本包提供的本地服务器：

~~~powershell
py -3 tools\serve.py 8765
~~~

再打开服务器打印的地址。服务器只绑定 127.0.0.1。关闭命令窗口即可停止。也可以双击 start-local-server.cmd。

源代码拆为 HTML、CSS、JavaScript。修改后重新构建单文件：

~~~powershell
py -3 tools\build.py
node tools\build_handbook.mjs
py -3 tools\package.py
~~~

前者只需 Python 3.10+ 标准库；后者只需 Node.js 20+，Markdown 转换器随包提供，不需要网络下载。预览本身不需 Python 或 Node。

## 原型审阅路径

1. 主界面修改待办文字，通过“编辑 → 查找/替换”定位“文本”，替换后撤销。
2. 分屏编辑中直接改 Markdown 任务项、段落或表格，再按 >_ 核对源码；切回来检查内容与未保存状态。
3. 拖动标签到另一编辑区域、调整分隔线，检查焦点和同文档内容同步。
4. 设置切换主题/字号/换行，搜索“恢复”；七类导航不改变产品布局。
5. 状态演示逐项查看外部变更、未保存恢复、只读、保存失败、大文件和多光标。
6. 查看菜单进入 Zen，按两次 Esc 或右上退出按钮恢复布局。

## 原型与正式实现的界线

原型真实执行文本输入、基础 Markdown 块编辑、模式切换、撤销重做、查找替换、简单多选同步修改、标签分组、配置存储及 UTF-8 导入/下载。部分行为只用于审阅：大文件为小样本状态；外部变更、异常恢复来源和保存失败由按钮触发；新窗口是浏览器演示窗口。

原型最多 3 个编辑区域、每个导入文件最多 5 MiB；正式产品建议最多 8 组，并按独立大文件策略验收。浏览器 textarea/contenteditable、快照历史和 JavaScript RegExp 均不是正式技术选型。正式路线是 Rust + NSH Windows 原生后端、增量 DocumentStore、原文保真 Markdown 桥接和独立搜索服务。

## 验证与维护

运行随包纯逻辑测试：

~~~powershell
node tests\core.test.cjs
~~~

真实浏览器 smoke 流程和 Windows 原生性能/IME 流程的运行要求、状态与证据格式详见 VERIFICATION 和 PERFORMANCE_PLAN。所有最终文件的校验和写在 manifest-sha256.json；这是交付内容校验，不等于产品性能测试。

文档发生冲突时，以用户最新明确要求为最高依据；具体命令和设置以 JSON/对应说明为准，行为以 PRD 为准，性能口径由 PERFORMANCE_PLAN 维护。原型未覆盖某动作不等于取消该 P0。未决工程选择由团队在 Gate 中记录并关闭，不逐项要求用户选择依赖库。
