# T05/T06 首版语法验收矩阵

依据 [PRD 第 8 节](../design-v1/docs/PRD.md#8-原生-markdown-所见即所得)，保留首版 16 行语法矩阵与 FR-MD-04–13 全部要求。原设计文件未修改。

本轮固定组件语料已实际执行：`runtime03` 的52项全部通过，包含本清单16行、8组合的所列断言；整体仍 **PARTIAL**，GUI/人工 **NOT_RUN**。原32-suite、构建成功和其他候选GUI不能替代本轮证据。原样展示、源码可达、直接编辑分别取证。

界面入口增量的 `runtime12` 已实际执行89/89、exit0（2026-11-18，本地未提交整合反馈，固定身份以随后包内 build-identity 与实际重跑为准）。新增真实 Owner/Store 断言包括：链接/图片属性与引用定义、原样块局部源码和明确整块删除、图片占位/地址解析策略、列表 Enter/空项退出/普通列表缩进、表格行列/对齐/表头与未编辑单元格字节、表内行内代码转义管道。它们是组件通过，不代表这些新界面入口已由 GUI 验收。runtime05–11的实际失败全部保留。

标题自动转换另捕获触发空格已提交、结构转换尚未发生的真实 Store 状态，比较 Undo/Redo 的完整字节、token、Profile、EOL、provenance、dirty 及历史深度；源码往返不清 redo。富文本字面标记允许在新增局部用 Markdown 转义/实体编码，例 `\###&#32;` 显示为 `### `，不是承诺把字面标记裸写源码后仍作为普通段落。

代码块/表格按键的下一增量已在 `compile20/runtime15` 实际编译及运行99/99、native和wrapper均exit0。新增8项覆盖围栏语言词和额外info保留、取消、退出优先复用后续正文/文末补段、真实Tab/Shift+Tab混合EOL及Source Undo、原样/缩进/未闭围栏/IME/过期拒绝和伪meta；新增2项覆盖installed Enter向表格下一行同列导航、末行提示、零正文/history/token/dirty增量及已有Source Undo。完整代码高亮/所有导航与GUI不由此关闭。`runtime14`的97项原native0但wrapper沿用先前rg的exit1，原回执保留，不冒wrapper0或产品FAIL。

机器可读固定语料与分层状态在 [SYNTAX-MATRIX.json](SYNTAX-MATRIX.json)，执行代码在 [syntax_matrix_tests.rs](../../crates/texzi-app/src/syntax_matrix_tests.rs)。JSON 的每一行分别保留 original_bytes、formatted_display、direct_edit、source_reachable、component、gui、manual 和证据引用。

| ID | PRD 原语法行 | 已编写的严格组件行为 | 仍须独立覆盖 |
|---|---|---|---|
| MD01 | 段落、空行、软/硬换行 | 可见叶输入，软/硬换行节点保留 | Enter 拆段、Shift+Enter、Backspace、真实输入 |
| MD02 | ATX 与 Setext 标题 | H1–H6/Setext 节点，ATX/两类 Setext 正文改字保原形式 | 改级别必要定界符、菜单及视觉 |
| MD03 | 粗体、斜体、删除线 | 三类 marks，编辑正文保星号/下划线选择 | 格式动作/后续样式、跨标记选择、交叉未知定界 |
| MD04 | 行内代码 | 正文编辑；内部新增反引号仅增长当前围栏 | 真实按键、更多空格/反引号边界 |
| MD05 | 无序/有序/嵌套列表 | bullet/ordered/nested 节点，文字编辑保符号/起始数 | Enter/空项退出、缩进/反缩进、降级/合并 |
| MD06 | 任务列表 | 文字编辑；单 checked 状态只改一个任务标记 | 原生框视觉、点击与文字命中、键盘焦点 |
| MD07 | 引用、分隔线 | 引用文字与分隔线语义节点 | 引用结构操作、分隔线整体选择/剪切/删除 |
| MD08 | 普通/自动/引用链接 | 显示文字与 href mark；文字编辑保持引用式及原定义 | 地址浮层、引用目标只改对应定义、悬停 |
| MD09 | 围栏与缩进代码块 | 两种 codeBlock 文字编辑保原文、语言和围栏 | Enter/Tab、退出/方向键、高亮、语言及闭合围栏边界 |
| MD10 | GFM 管道表格 | 表头/正文节点、单元格改字保对齐/间距 | 增删行列、对齐、全部导航/表头升级/末行列删除 |
| MD11 | 本地/相对路径图片 | image 节点；src/alt 局部修改与共享历史 | 浮层、路径行为、无资产搬移/上传 |
| MD12 | 远程图片 | image 节点；src/alt 局部修改与共享历史 | ask/占位/取消/失败重试、网络与浮层 |
| MD13 | 原始 HTML | 完整惰性原样文本块；返回源码真实输入并撤销 | 局部编辑源码动作、整块选择；运行层脚本不执行 |
| MD14 | 数学 | 行内/块公式含定界符的完整原样块；源码可达 | 局部源码动作及展示；不承诺数学排版 |
| MD15 | Mermaid 围栏 | 代码块展示、代码文本改字 | 原生键盘编辑；不承诺图形渲染 |
| MD16 | 脚注/自定义容器/未知方言 | 三种原样块完整内容、顺序与源码可达 | 局部源码动作、整块选择删除/剪切 |

以上“已编写”已取得本轮固定组件运行证据，但各行完整行为仍为 **PARTIAL / GUI NOT_RUN / 人工 NOT_RUN**。MD13、MD14、MD16 按 PRD 走源码编辑，不以原样块显示或源码可达宣称结构直接编辑完成。链接/图片属性通过程序事务调用也不证明浮层入口已实现。

每项固定原文都同时包含 CRLF、LF、CR 与独立未知 HTML 邻块。单个改字的预期由人工指定的 before/after 范围生成完整原始字节预期，未使用 serializer/parser 的输出作为期望值。断言还核对范围外原始 provenance，不仅比对渲染文本。

组件路径为 public RichOwner::new → EditorSession.dispatch → NativeView → 唯一 Store。每次选择均要求正文 revision/token/history/dirty 不变；每次已实现直接动作要求一次 Store 历史、完整字节预期、RichEditor 已安装的 Ctrl-Z/Ctrl-Y，以及源码适配器 Undo/Redo。

ModeGate 测试对 16 项及 8 种组合分别执行 100 次真实组件 source→rich→source 调用并挂载源码；核对原始字节、revision、token、EOL、provenance、history 和 dirty 不变。另对全部 16 项预置 redo 后往返 100 次，验证 redo 未被切换清空。它是有真实 Store 的组件测试，**不是 100 次 GUI 切换**。

组合语料独立列在 JSON：

- COMBO01：引用 → 有序列表 → 粗体，含续行。
- COMBO02：嵌套任务列表/普通列表/链接。
- COMBO03：表格单元格内格式、链接、代码及转义管道。
- COMBO04：同文重复于粗体、普通字、斜体与行内代码，逐一修改四个节点。
- COMBO05：反斜杠转义、命名实体、emoji 数字实体及版权数字实体；逐项替换。
- COMBO06：Setext 标题内实体与重复可见文字。
- COMBO07：围栏语言、代码首行与后行同文，定位正文而非语言。
- COMBO08：BOM 与中英/emoji 组合、混合换行和未知邻块。

FR-MD-04–13 在 JSON rules 中逐条保留，包含现有矩阵未实际执行的块结构操作、IME、表格全部动作、代码块退出、并排与同步滚动、多光标回源码、剪贴板、拖放及后台 revision/预算。它们保持 NOT_RUN；此任务未访问剪贴板、未下载远程图片、未执行 T07、未安装、未启动 GUI、未提交。

运行本候选后才可补证据：

~~~powershell
cargo test --locked -p texzi-app syntax_matrix_tests -- --nocapture
~~~

候选 commit、二进制 SHA-256、测试日志/退出码、各项组件结果和后续 GUI/人工记录须分别填入 JSON。遇到拒绝或不满足的原样/格式语义，保留失败及精确行为；不能改为较弱的“不丢原文”断言后填直接编辑 PASS。

