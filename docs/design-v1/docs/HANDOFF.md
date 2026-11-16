# texzi Agent 团队实施交接 v1

状态：设计交付与实施任务书；本包中的高保真原型不是已完成的 Windows 应用。日期：2026-11-15。

实施目标是把已确认的 texzi 设计做成 Windows 10 x64 原生文本编辑器，并用可重复证据证明正确性和约定性能。用户已经授权完整设计交付，不需要再以“是否继续设计”为由暂停。实施团队在现有方向内自主解决常规技术选择；涉及删除 P0、改变原生路线、降低已接受目标等范围变化，先给出具体证据与替代方案，不能暗中改产品。

## 1. 阅读顺序与文件责任

| 顺序 | 文件/资源 | 用途 |
|---|---|---|
| 1 | [PRD.md](PRD.md) | 产品场景、P0/P1、功能和异常规则、FR 编号。 |
| 2 | [UI_SPEC.md](UI_SPEC.md) 与 `../concepts/` | 已确认的概念、页面/组件交互、可见状态和视觉参数。 |
| 3 | [SETTINGS.md](SETTINGS.md)、[COMMANDS.md](COMMANDS.md) | 设置 schema、作用域/依赖/默认值、Windows 快捷键及核实状态。 |
| 4 | [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md) | 模型、线程、模块、NSH 缺口、Markdown 保真、文件可靠性与平台方案。 |
| 5 | [PERFORMANCE_PLAN.md](PERFORMANCE_PLAN.md) | 目标归属、硬件、样本、测量边界、统计与证据。 |
| 6 | [ACCEPTANCE.md](ACCEPTANCE.md) | 可执行产品验收；本文件中的 TP/Gate 是工程补充，不替代 AC。 |
| 7 | `../prototype/` | 可审阅的行为参考与源文件；阅读其说明区分真实交互与模拟。 |

发生冲突时，先检查最新明确用户要求，再按 PRD 的产品合同、SETTINGS/COMMANDS 的具体表、UI_SPEC 的交互和技术方案解释；跨文档值不能各自发展。技术性能目标由 PERFORMANCE_PLAN 唯一维护；所有更改写入决策日志，指出关联 FR/AC、原因和影响。原型某按钮暂未实现不意味着该 P0 被取消；原型中的 JavaScript RegExp/浏览器 textarea/contenteditable 不构成最终编辑内核选型。

## 2. 不可擅改的基线

1. 名称 **texzi**；只做 Windows。目标架构已确认 **x64**，不再延续此前 x86 风险表，也不重新扩大到 ARM/macOS/Linux。
2. Windows 10 是产品范围；1809+ 是依据 NSH 提出的工程最低版本，主验证系统为 22H2 x64。没有测过的版本标“未测”，不能写“全 Win10 已支持”。
3. 使用 NSH `v0.5.1` / `224d9a795c7ac9d10d75750de9351adac5549675` 为基线；主编辑与 Markdown 采用原生实现。补丁 fork 必须可追溯，不能悄悄升级 main。
4. Markdown 默认真正 WYSIWYG；右上 `>_` 在当前区域切源码。不能把预览当编辑；切换不修改原文、不新增 undo、不改变 dirty。
5. 原始 Markdown 为唯一事实源。局部事务、未知语法保留、统一 history、每视图独立 selection/scroll/mode 是 P0，不是以后优化项。
6. 多标签、分屏、独立原生窗口、同文档多视图、中文 IME、编码/换行、未保存恢复、外部变更及大文件要按合同一起实现。
7. 用户接受的性能目标数值不能降低或用平均值替代 p95/p99；测试环境与场景矩阵是本方案建议的展开，不写成用户逐项指定。1–10 MiB 普通 WYSIWYG 延用输入目标单列测试，100 MiB 普通文本/源码与复杂 WYSIWYG 的范围分开报告。
8. Zen 已确认。不得重新猜“Z 模式”；不新增完整 IDE、LSP、终端、调试器或插件市场。
9. 不主动识别或分类输入的敏感程度；不因分类结果改变恢复/日志行为。
10. 默认代码字体按 SETTINGS：`Cascadia Mono, Consolas` 与 Windows 中文回退；不擅自把其他字体提升为已确认默认。

固定依据：[NSH 发布](https://github.com/longbridge/nsh/releases/tag/v0.5.1)、[固定 commit](https://github.com/longbridge/nsh/commit/224d9a795c7ac9d10d75750de9351adac5549675)、[Windows 后端与要求](https://nsh.dev/docs/windows/)。

## 3. 团队分工与可并行工作

建议四个并行席位；“负责人”是职责标签，不是必须招聘或已指定的人名。

| 席位 | 主责任 | 关键接口 |
|---|---|---|
| A：平台与工作台 | NSH Windows、窗口/标签/Dock、命令设置、主题/可访问性、输入路由 | 只调用 DocumentApi，不绕过事务写正文。 |
| B：文档与可靠性 | Rope、坐标映射、历史、文件编码保存、恢复、搜索 | 提供稳定快照/事务接口、文件身份和资源预算。 |
| C：Markdown | CST/源映射、局部 patch、富编辑投影、虚拟化、正文搜索投影 | 原文是输入输出；不能将 RichEditor 状态变成第二个存储真相。 |
| D：集成与验证 | 依赖锁、fixture、trace、Windows 实测、故障注入、打包和交接 | 审查证据，独立跑测试，不以作者演示自证通过。 |

T00/T01 先完成统一契约；随后 A 做平台壳、B 做正文核心/IO、C 做 Markdown 保真 spike、D 做 fixture/trace 并行推进。B 的文件/搜索任务可在正文接口稳定后拆给临时 Agent；任何额外 Agent 先领取独立目录/模块，不让多人同时改同一接口文件。接口变化通过短决策记录通知所有受影响席位，不能用共享工作区里的偶然文件变化传递合同。

建议代码目录：`crates/texzi-document`、`texzi-editor-adapter`、`texzi-markdown`、`texzi-search`、`texzi-files`、`texzi-recovery`、`texzi-workspace`、`texzi-commands`、`texzi-app`、`xtask`；具体名称可在 T00 一次统一。每个 PR 保持一个清楚目的，附关联任务、行为变化、验证及剩余限制。不要先做一套可丢弃的“完整 UI”，再尝试接入无法满足原文保真的文档模型。

## 4. 任务拆分与依赖

每个任务结束必须提供可运行的结果/测试证据，不接受仅写“已支持”或贴截图。下表所有 P0 项属于首版路径；字体版本、parser/binding 等工程待定项由负责人在任务中核实并锁定，无须逐项要求用户选择库版本。

| ID / 优先级 | 负责人 / 依赖 | 具体工作和交付 | Definition of Done |
|---|---|---|---|
| **T00 基线与可复现构建 / P0** | D+A / 无 | 建 Windows x64 仓库、工具链、固定 NSH commit 与 feature；建 Cargo.lock、license/SBOM、决策日志；锁定所有直接依赖与字体策略。 | 干净 Win10 x64 环境可按 README 构建 Release 原生窗口；版本/feature/编译命令可重现；G0 通过。 |
| **T01 文档与视图契约 / P0** | B+C+A / T00 | 定义 DocId/ViewId/Revision、三类文本坐标、EOL/provenance、事务和事件、共享 history/每视图状态、保存状态机。 | 契约示例覆盖跨模式/跨窗输入、异步保存时继续输入、Undo 回 clean；没有两套正文 owner。 |
| **T02 样本与追踪 / P0** | D / T00–T01 | 复用随包 tools/bench/generate_fixtures.py 与manifest，补齐未覆盖的复杂/阈值样本；建立QPC/ETW/revision-present关联、报告器与Windows trace。 | 同seed生成相同hash；覆盖清单真实；能测到实际present，未实现的测量项标未知；工具开销有对照。 |
| **T03 正文、Unicode 与历史 / P0** | B / T01 | Rope/piece 持久化快照；UTF-8/UTF-16/grapheme 映射；多选区规范化；正反 delta；saved token；历史预算。 | 随机编辑/undo property 测试与固定 Unicode 语料通过；100 万编辑不出现位置漂移；无每键全量字符串。 |
| **T04 NSH 文本适配 / P0** | A+B / T03 | 形成最小 NSH fork：外部 buffer/增量事件/外部 history/独立 view state；多光标与矩形选择；可见行布局/高亮。 | 双视图内容同步但位置独立；10 MiB 输入和 100 MiB 打开有第一轮证据；源码命令映射完毕；G1 可评估。 |
| **T05 Markdown 保真 spike / P0** | C / T01–T03 | CST/trivia/源区间、稳定 BlockId、opaque 节点、RichStep→局部源 patch；测试模式切换和未知语法。 | 无修改切换 100 次字节相同；修改某段不重写无关块；未知语法/混合 EOL 保留；不能依赖全篇 serializer。 |
| **T06 原生 WYSIWYG / P0** | C+A / T04–T05 | 集成 RichEditor native view；段落/列表/表格/代码块/任务框/链接/图片；局部重建、选择/scroll 映射、正文搜索 projection。 | 原生可直接编辑；>_ 同区切换0事务；跨模式 Undo/Redo；富编辑复杂度与虚拟化证据；G2 可评估。 |
| **T07 编码、保存与外变 / P0** | B / T03 | BOM/严格编码、GB18030标准版本、EOL表、原字节透传、同卷临时文件/替换/备份、watcher/指纹、另存为身份协调；externalChange的reloadClean/ask分支。 | 未改保存 byte-exact；编码失败不替换成问号；ask在clean时也先提示；磁盘满/外变/ReplaceFile异常不丢可恢复版本；明确链接/网络盘范围。 |
| **T08 恢复与会话 / P0** | B+D / T03、T07 | checkpoint+journal+CRC+tombstone、默认1s间隔/目标≤2s、布局与未命名内容恢复、退出前flush、损坏记录隔离。 | kill点/半写/外部修改恢复通过；明确放弃不复活；仅restoreOnStartup、restoreUnsaved与snapshotFlushSucceeded全true可关窗静默保留，否则保存/放弃/取消；G3 子项通过。 |
| **T09 搜索与替换 / P0** | B（可委派） / T03、T02 | 字面检索、PCRE2版本/UTF/UCP、全字边界、模板、零宽；可终止worker；页式匹配；Replace All完整计划单事务。 | 各模式语义矩阵/超时/取消通过；dirty snapshot查找正确；百万结果不冻结；原型JS语义不泄漏进正式契约。 |
| **T10 文件夹工作区 / P0** | A+B / T07、T09 | 单根树惰性枚举、文件操作/回收站、glob、链接环路、文件夹搜索、dirty缓冲优先、跳过/截断计数。 | 10万项目录不会递归读全文卡启动；20,000结果限制可见；跨文件替换不进入P0。 |
| **T11 标签、分屏与窗口 / P0** | A / T01、T04 | 预览/固定标签、递归分割、8组建议上限、拖出/拖回、同文档ownership、关闭计划、显示器位置恢复、Zen。 | 至少2个真实顶层HWND；同doc唯一history；拖移失败不丢来源；最后标签关闭与窗口恢复关闭语义不同且正确。 |
| **T12 IME、DPI 与剪贴板 / P0** | A牵头，B+C参与 / T04、T06、T11 | TSF focus/组合/候选矩形；多光标commit；100–200%混合DPI；多选元数据与外部纯文本粘贴。 | Pinyin新/兼容及固定Sogou版本实测；无丢字/重复/键泄漏；中英emoji/表格/代码块和跨窗通过；G4子项通过。 |
| **T13 设置与快捷键 / P0** | A / T01、T11 | 从SETTINGS/COMMANDS机器可读数据生成校验与命令注册；作用域、依赖、持久化错误、重置；冲突和IME优先级。 | 每条设置能检查有效来源/即时或下次生效；默认键逐项核实状态保留；未核实项有测试而非猜值。 |
| **T14 视觉与可访问性 / P0** | A+D / T06、T11、T13 | 按概念/视觉token完成全部状态；UIA正文范围、Narrator、键盘焦点、高对比、减少动画；窄窗口/多DPI。 | 主界面/分屏/设置/Zen/错误/大文件都有原生截图；关键流程不用鼠标可完成；没有只靠颜色表达状态。 |
| **T15 大文件与性能收敛 / P0** | D牵头，A+B+C修复 / T02、T04、T06、T09 | 全矩阵benchmark；≥20/200MiB策略；≥20,000 Unicode scalar长行保护与边界样本；高亮/解析取消；富文档虚拟化、共享buffer与内存。 | 已接受目标在锁定环境按建议场景矩阵验证；复杂Markdown及1GiB实测/未覆盖边界单列；不能用只读或占位假ready。 |
| **T16 可靠性集成 / P0** | D+B / T07–T12 | 冲突、保存竞争、恢复尾损坏、异步版本竞争、睡眠/RDP/驱动恢复等故障集成；完整AC回归。 | 无数据丢失/错误覆盖/半事务；失败场景有可理解恢复路径；G3/G4通过。 |
| **T17 安装与发行包 / P0** | D+A / T00、T14、T16 | x64用户级安装器/便携目录、图标/关联可选项、license、体积拆分、签名与校验和；升级/卸载数据策略。 | 干净Windows按指南安装启动；升级保留设置/恢复；卸载不删除用户文档；证书未就绪明确为未签测试包。 |
| **T18 最终验收与交接 / P0** | D，全员复核 / T15–T17 | 补齐FR→AC→测试证据映射、性能报告、操作/构建说明、缺陷清单、依赖patch清单及发布检查。 | G0–G5和本文件第8节全部通过；交付真实源码/包/证据；未测、失败、模拟不标完成。 |

最重要的两条风险路径为 `T01→T03→T04→T15` 和 `T01→T05→T06→T15`。T05 应早于视觉全面铺开；它失败时先解决保真/映射，不继续用会整篇改写的临时方案积累页面。T07/T08 可与 C 的富编辑工作并行，避免最后才发现保存/恢复设计需要重写文档模型。

## 5. Gate 决策与原生备选

| Gate | 可执行通过条件 | 阻塞后的下一步 |
|---|---|---|
| **G0 固定基线** | Win10 x64 release可构建/运行；NSH固定版本与features可审计；精确工具链/PCRE2等依赖锁定。 | 先修构建或最小fork；不换成Web方案绕过。 |
| **G1 文本内核** | 10MiB跨视图输入、100MiB普通文本打开、中文TSF、共享Undo、无全文onChange瓶颈。 | 补外部buffer/增量API；必要时评估NSH Native Component嵌原生编辑控件并跑同一矩阵。 |
| **G2 原文与WYSIWYG** | 支持结构可直接编辑；100轮模式切换0文本差异；未知语法保存；跨模式历史；复杂度策略可取消。 | 扩展CST桥接与原生虚拟化；不得用readonly preview替代。 |
| **G3 数据可靠性** | 编码/EOL、原子写失败、外变竞争、恢复损坏、退出flush故障全部有证据。 | 阻断发布，先修存储/事务；不以关恢复换性能。 |
| **G4 Windows体验** | 真多窗口、全部指定IME、DPI、剪贴板、UIA/键盘、窗口系统行为通过。 | 修平台/焦点/坐标接口；保留未覆盖输入法/OS清单。 |
| **G5 性能与发行** | 已接受指标按独立场景通过，数据完整；安装/升级/运行及文档可复现。 | 针对具体风险补验证/修复；任何范围变更以明确差距提出。 |

原生控件备选不是“按一个开关就能替代”。以 Scintilla 等为候选时，必须重新验证 NSH 子 HWND 的叠放/裁剪、焦点、DPI、中文 IME、辅助功能、多视图、主题、搜索装饰和共享历史，记录依赖许可。WebView2/Monaco/Electron 不能由实施 Agent 自动引入作为逃避 G1/G2 的方案。[NSH Native Components](https://nsh.dev/docs/native-components/)。

## 6. 工程验收清单：能直接执行的步骤

以下 TP 编号是对 ACCEPTANCE 的工程补充。运行前清空测试目录而非用户真实文档；所有 destructive/fault 操作只针对生成 fixture 和专用测试环境。

| 编号 / 负责人 | 执行步骤 | 预期和必须留下的证据 |
|---|---|---|
| TP-01 / D | 从干净checkout按README构建x64 Release；运行于22H2；在1809环境重复核心启动/输入/保存。 | 记录版本、命令、包hash与OS结果；1809未测不能标通过。 |
| TP-02 / B | 对TXT/UNICODE施加固定seed的随机非重叠编辑与undo/redo；对照简单reference字符串和EOL表。 | 每步内容/位置一致；Undo回初始字节；跨grapheme不拆字符；测试seed可复现。 |
| TP-03 / A+B | 同文档开4视图/2窗口；各设不同位置；A输入、B撤销、C重做；保存过程中继续输入。 | 一份内容/history，其他视图不乱跳；保存只清目标版本dirty；Trace含DocId/Revision。 |
| TP-04 / C | 含ATX/Setext、两种列表符号、引用链接、实体、空白、CRLF/混合EOL、未知块的MD来回切100次并保存。 | 文件SHA-256相同，Undo深度与dirty不变；自动格式化设置关闭。 |
| TP-05 / C | 对每个支持结构执行输入/删除/拆分/合并/表格行列操作；比较变更源区间；Undo。 | 未改区间字节不变，局部语义正确；未知块不被flatten；Undo恢复原字节。 |
| TP-06 / C+B | WYSIWYG正文查找跨粗体/链接文字及表格cell；再开regex/替换。 | 正文结果可映射；切源码后旧结果清空重算；替换原文且一次Undo。 |
| TP-07 / A | Pinyin真实候选输入、切候选、Esc、Enter；组合中切>_/标签/窗/Zen；源码多光标输入。 | 无重复/丢字/快捷键泄漏；preedit不落盘；一次commit一次Undo；录屏和输入法版本。 |
| TP-08 / A+D | 100/125/150/200%DPI及双屏跨移；选中组合emoji/CJK；候选窗、点击、拖选、截图。 | 几何一致、候选跟随caret；字体回退不致光标偏移；UIA/Narrator读到正确模式/状态。 |
| TP-09 / B | 编码变体未改保存；每种修改一个可编码字符；再插入不可编码字符尝试原编码保存。 | 未改byte-exact；修改仅改变预期段；不可编码不静默`?`；混合EOL不归一。 |
| TP-10 / B+D | 在保存临时写入/flush/replace/清理阶段分别注入失败；外部程序在检查与replace间写另一版；分别测试reloadClean/ask与clean/dirty组合。 | 可恢复原/外部/本地版本；UI不假成功；ask即使clean也先提示，dirty始终处理冲突；保留失败日志和目标/备份hash。 |
| TP-11 / B+D | 未命名文档编辑后按固定flush序号kill；破坏journal尾部；磁盘文件同时外变；重启。 | 恢复完整已flush事务，损坏条目隔离；不覆盖外部版；明确放弃的记录不自动复活。 |
| TP-12 / B | literal/全字/大小写/regex/选区组合，零宽与模板未知组，100万匹配，病态regex，连续改query。 | 有效结果与reference一致；预算错误可见；取消≤预算；旧结果不覆盖；ReplaceAll取消0修改。 |
| TP-13 / A+B | 文件夹含dirty已打开文件、UTF16、无权限、二进制、链接环、根外链接；搜索超过结果上限。 | 优先内存版本；跳过/截断可见；不无限递归；跨文件替换不意外出现。 |
| TP-14 / A | 标签预览/固定、8组、移组/复制/拖出/拖回、最后标签关闭；测试restoreOnStartup、restoreUnsaved、snapshotFlushSucceeded三项的8种组合；显示器消失恢复。 | 无丢标签/重复owner；三项全true才可关窗静默保留，否则保存/放弃/取消；显式关闭最后dirty标签仍提示；需处理DocId只问一次；窗口可见。 |
| TP-15 / D | 按性能文件跑1/10/100MiB、各模式、热/冷启动、输入/滚动/搜索/内存；1GiB探索另列。 | 完整原始数据、分位计算、失败率；无伪实测；正式达标使用默认功能与Release。 |
| TP-16 / D+A | 用户级安装、便携启动、升级、卸载；断网首启；没有额外字体的干净系统；未签包检查。 | 可运行且依赖齐全；用户文档保留；体积/许可/签名状态清楚；不把安装WebView当原生Markdown前提。 |

正确性测试选择能保护外部行为和数据的边界，不为每个getter或可逆低影响视觉数值写重复实现的单测。源码保真/编码/事务等高风险算法适合property和golden测试；IME/DPI适合真实Windows集成与人工操作；概念图像素差异只能说明视觉一致性，不能证明数据正确。

## 7. 可直接交给 Agent 的 Goal 提示词

### 7.1 团队总 Goal

```text
你负责实施 texzi Windows 10 x64 原生文本编辑器。先阅读 docs/PRD.md、UI_SPEC.md、SETTINGS.md、COMMANDS.md、TECHNICAL_DESIGN.md、PERFORMANCE_PLAN.md、ACCEPTANCE.md、HANDOFF.md 和已确认概念图。用户已确认 x64、Markdown 真正 WYSIWYG、右上 >_ 同区源码切换、Zen，并接受列出的性能目标。

以 NSH v0.5.1 commit 224d9a795c7ac9d10d75750de9351adac5549675 为基线。原始 Markdown 是唯一事实源；必须做局部 source patch、未知语法保留、同文档统一 history、每视图独立模式/选区/滚动。不得把 RichEditor 全篇 serializer、readonly preview、WebView 或 HTML 原型误当已符合要求。

按 HANDOFF 的 T00–T18 与 Gate 执行，先关闭正文增量接口和 Markdown 保真两个核心风险，再扩大 UI。把可并行模块分给不同 Agent，统一接口与任务 ownership。常规库版本/实现选择自行核实并记录，不反复问用户。只在需要改变已确认方向时提交具体差距和候选方案。

持续做到可构建、可运行、有测试证据。Windows IME、DPI、恢复与性能必须在 Windows 原生构建验证，不能用浏览器原型或 Linux 单测代替。所有未测/失败项如实列出，不伪造 benchmark。最终交付源码、依赖锁、安装/便携包、运行与构建方式、FR→AC证据映射、性能原始数据、已知限制和 NSH patch 清单。
```

### 7.2 分工 Goal

**A 平台与工作台：** 领取 T04/T10/T11/T12/T13/T14 的平台部分，先接 T01 DocumentApi，保证两个真实窗口共享 DocId 但视图状态独立。任何正文变动调用事务；实现输入法优先级和DPI坐标链；按概念做界面，按COMMANDS/SETTINGS完成行为。交付NSH补丁、原生截图、键盘/IME/DPI证据和待验证边界。

**B 文档与可靠性：** 领取 T03/T07/T08/T09，建立持久化原文、EOL/provenance、history/saved token、严格编码及保存/恢复协议。先构造反例和故障点，再接UI；保证后台任务版本校验、取消0部分事务，PCRE2预算可终止。交付property/golden/故障测试及可追溯文件hash。

**C Markdown：** 领取 T05/T06，先用包含未知语法和混合EOL的语料证明100轮切换0字节变化，再覆盖各结构编辑；实现rich positions↔source ranges↔正文search projection。每个动作提交最小源事务；对暂不支持语法保留opaque原文；富编辑可直接输入且不重建活动composition块。交付局部patch测试与复杂文档性能结果。

**D 集成与验证：** 领取 T00/T02/T15–T18，维护Windows环境、fixture/trace/report，独立复跑各模块证据。报告每个模式/规模的p95/p99而非聚合平均；核验失败/超时没有被删。完成安装升级、数据保留、许可和打包体积，统一FR→AC→commit→artifact索引。未通过Gate时指出具体阻塞，不以追加无关测试代替修复。

## 8. 项目级 Definition of Done

- 源码能够按说明从干净环境构建成 Windows x64 原生应用，固定 NSH、工具链、依赖和许可；补丁来源及变更理由清楚。
- 所有 P0 FR 均关联至少一个 AC/TP 和可重放证据，未支持项没有藏在组件默认行为里。
- 编辑/保存/恢复/模式切换/多窗口通过数据不变量；无已知数据丢失、无静默有损编码、无未知Markdown语法被自动删改。
- 原生 WYSIWYG、源码、多光标、查找替换、设置、分屏分窗与大文件行为符合确认设计；图片/原型只是外观依据。
- PERF 已接受目标在锁定主机和具体场景通过；新增建议预算有通过结果或明示差距；复杂文档/极限文件的范围和退化路径真实。
- Windows IME/DPI/UIA/快捷键冲突有真实验证；没有把 DirectWrite/TSF“存在”写成所有输入法已通过。
- 安装/便携运行方式、Release二进制hash、源码commit、依赖清单、测试和性能报告、故障恢复说明、用户可见已知限制齐全。
- 最后一次交接列出完成/未完成/不适用，明确剩余负责人。没有功能性的未完成事项时停止无必要打磨，不重复扩测已充分证明的低风险路径。

## 9. 发布证据包与建议命令

建议产物目录：`artifacts/release/` 放安装器/便携包及 SHA256；`artifacts/acceptance/` 放 AC/TP 结果及录屏；`artifacts/perf/` 放环境、fixture manifest、trace、CSV和报告；`docs/decisions/` 放技术决策；`patches/nsh/` 放上游基线与补丁说明。测试数据不能混入用户正式恢复目录。发布说明先写解决了什么、结果如何，再写必要技术细节。

本包已提供 [样本生成器](../tools/bench/generate_fixtures.py)，支持 smoke/standard/stress，并在manifest显式列出未覆盖项。默认smoke约2.2MiB，standard约443MiB，stress约643MiB；将生成样本置于交付ZIP之外。已有工具只准备输入数据，不包含Windows原生应用、基准runner或实际性能结果；完整覆盖边界见PERFORMANCE_PLAN第3/10节。

以下三条是 NSH 固定版本开发文档已列出的上游检查命令，仅在 **Windows 上的 NSH 仓库 checkout** 中使用；不是 texzi 的构建入口。NSH Windows crates 在其他系统上为空壳检查不能证明 Windows 实现通过。[固定构建说明](https://github.com/longbridge/nsh/blob/v0.5.1/docs/dev/windows.md)。

```powershell
cargo clippy -p nsh-platform-windows -p nsh-windows-host -p nsh-windows-app --all-targets
cargo test -p nsh-platform-windows
cargo build -p nsh-windows-host
```

实施仓库落地后应提供如下等价的真实命令；本文件给接口约定，**不表示本设计包现在已经包含 Rust 应用或这些测试 crate**：

```powershell
rustup show
cargo build --release --locked --target x86_64-pc-windows-msvc -p texzi-app
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --locked -p texzi-document -p texzi-markdown -p texzi-files -p texzi-search
cargo run --release -p xtask -- verify-acceptance --output artifacts/acceptance
cargo run --release -p xtask -- package --target x86_64-pc-windows-msvc --output artifacts/release
```

只有支持该任务的feature组合才执行，避免无差别 `--all-features` 把非目标平台/终端/JS又加回来。UI/IME/DPI测试在Windows跑，核心算法可在其他CI环境补充但不能替代。冷启动脚本只在明确指定的专用测试机调度重启，故障写盘只使用fixture副本；默认开发命令不关闭用户应用或重启其机器。

## 10. 架构预留的止步线

首版可以保留稳定内部 CommandId、DocumentApi、语言高亮provider trait、版本化设置/恢复格式和主题token，这些是降低内部耦合的工程接口。**不实现**外部插件加载器、市场、第三方脚本执行权限、插件API兼容承诺、扩展安装页或远程同步。NSH/RichEditor 源码中的“plugin/extension”内部机制不等于 texzi 必须开放插件生态。

后续若需要更换高亮parser、增加Markdown方言或拆进程，可通过这些内部接口演进；当前工程不能以“未来生态需要”为由增加首版启动负担、安装依赖或设置数量。
