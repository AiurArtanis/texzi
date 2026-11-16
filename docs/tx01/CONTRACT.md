# TX-01 / T01 文档契约草案

本文给出拟议的规范性约束，供先审核后实现。所有“必须”均为后续实现要求，
不是已经运行通过的声明。行为预期见 [EXAMPLES.md](EXAMPLES.md)。

## 1. 权威状态与身份

**C01 单一 owner。** 进程级 DocumentStore 是正文和内容历史的唯一 owner，
只有一个串行写者可以发布它的状态。线程位置由 T04 的 NSH 适配落实；
Win32/TSF、源码编辑器、富编辑器、IO 工作线程都只投递意图或不可变结果。
不得让某个 View、NSH Editor/RichEditor session、AST 或恢复日志成为第二个可写正文。

| 概念类型 | 身份与约束 |
|---|---|
| DocId | 不透明、进程会话内不复用；文档销毁后重开用新 DocId。路径不是 DocId |
| ViewId | 不透明、不复用；绑定一个存活 DocId，独立拥有位置/模式/交互状态 |
| Revision | 每 DocId 从 0 开始单调递增；成功内容事务、Undo/Redo、显式基线重载均前进；不倒退或复用 |
| StateToken | 序列化语义状态的不透明身份；不是 revision 或逐键全文 hash；Undo/Redo 可以返回先前 token |
| TxnId / HistoryGroupId | 内容提交身份 / 可撤销组身份；一次 Undo 可以包含多次已合并输入事务 |
| SaveOpId / TargetKey / BindingEpoch | 保存操作、目的文件协调键、文档磁盘绑定代次；与内容 revision 分离 |
| ViewEpoch / JobId | 视图请求代次 / 异步工作身份；取消或新请求可递增代次，不产生正文事务 |

DocId/ViewId 的位宽和生成器不在此冻结，但禁止将这些类型互换或跨 DocId 比较 revision。
计数器溢出必须返回明确错误/停止接受相应提交，不允许绕回导致旧任务重新有效。
识别到同一文件身份的多条路径必须解析到同一 DocId；文件身份与别名解析在 T07 落实，
不把路径字符串相等当作完整身份判定。未能证明同一身份时保留受控回退和冲突检查。

**C02 权威快照。** DocumentSnapshot 包含：DocId、Revision、StateToken、
UTF-8 逻辑原文、EOL 表、编码/BOM/profile 版本、不可变 provenance 引用。
逻辑文本中的换行为 LF，Markdown 标记、空白、未知语法均是原文的一部分。
快照持有不可变持久化数据；后台任务不能写回它，前台不要求每键复制全篇字符串。
输出策略须解析进捕获快照；后来的全局设置变化不能改变已经接受的保存操作。

StateToken 的序列化语义覆盖逻辑原文、逐换行类型、编码/BOM 和明确的输出策略。
纯来源索引重排、缓存更新、文件指纹刷新不创建 token；若来源差异会改变输出字节，
则必须视为语义状态改变，不能伪装缓存更新。一个 token 对应的输出依据不得暗中变化。

## 2. 坐标和边界

**C03 带域坐标。** 文档坐标必须带 `(DocId, Revision, unit)`；
SourceByte、Utf16Offset、GraphemeOffset 为不同类型，不接受无单位整数混传。
范围为半开区间 `[start,end)`，偏移 0 到末尾长度（含 EOF）有效，start 不大于 end。

| 单位 | 定义 | 非法边界 |
|---|---|---|
| SourceByte | 规范逻辑原文的 UTF-8 字节偏移 | UTF-8 continuation byte 内、越界、负值 |
| Utf16Offset | 同一逻辑原文的 UTF-16 code unit 偏移，LF 算 1 unit | surrogate pair 中间、越界 |
| GraphemeOffset | 从头计数的扩展 grapheme 边界，末尾为 cluster 数 | cluster 内不是整数边界 |
| Line/Column | 0-based 逻辑 LF 行号；列类型必须显式标 Byte/Utf16/Grapheme | 不得混用视觉折行、glyph、tab 展开列 |
| OriginalByteOffset | 特定不可变原始字节源中的位置 | 不是 SourceByte，不可直接交给 submit |
| RichPosition | 派生投影位置，绑定 SourceMap 的 revision/epoch | 无当前映射或 opaque 内不明确时拒绝富编辑步骤 |

严格转换 `convert(snapshot, offset, target_unit, Exact)`：非法标量边界返回
InvalidBoundary；合法标量边界位于 grapheme 内时，转换到 Exact Grapheme 返回
InsideGrapheme。禁止悄悄截断 surrogate、替换 U+FFFD 或自动 NFC/NFD 归一化。

光标移动、常规 UI 选择/删除必须用 grapheme 边界。UI hit-test 若需吸附，
必须显式用 `Floor/Ceil + Affinity`，返回吸附后位置和原因；不能冒充 Exact 成功。
底层程序化源 patch 可以声明 ScalarBoundary，但必须证明不切断 Unicode scalar；
不得把此能力用于默认 Backspace/Delete 拆坏 emoji/组合字符。T03 固定 Unicode
数据/分段库版本并冻结语料；本候选未虚构该依赖已锁定。

转换读取同一 revision 快照；旧 revision 的位置不直接用于新正文。
行/块索引和 grapheme 缓存必须按 delta 失效；复杂度和长行降级由 T03/T15 验证，
不得用每键从文件头扫描作为默认适配接口。

## 3. EOL、编码和 provenance

**C04 无损依据。** `eols.len == source 中 LF 数量`，按当前 LF 顺序记录
CRLF/LF/CR；原文末尾是否有换行由 source 本身表达，不额外补齐。
未编辑区间的 immutable origin span 包括源身份、原字节范围和解码 profile；
它只用于无损透传/映射，不能与 source 竞争可编辑所有权。

每个编辑要一并规划文本 delta、EOL delta、来源 span 的拆分/失效/映射及逆操作。
Undo 恢复相应原始来源；不得只还原屏幕文字而丢失混合 EOL/BOM/未知片段字节依据。
来源透传只有在编码/profile 与跨度语义仍一致时可用，否则严格重新编码或报错。
非法/不确定解码与用户选择由 T07 处理，不得先悄悄替换坏字节再把它当权威可保存原文。

普通输入/粘贴入口先把 CRLF/CR 解析成逻辑 LF，再生成配套 EOL delta；
submit 不接受含原始 CR 的“规范插入文本”。新插入换行按提交前快照的主导类型；
平票取文件第一个换行，没有换行则取已解析的新文件默认（设计基线 CRLF）。
原有换行不随插入统一。显式 EOL 转换、编码/BOM 修改是可撤销内容事务，
即使逻辑文字不变也前进 revision/token，并参与 dirty。

保存本身不格式化、不去尾空格、不补末尾换行。若用户设置要求这些转换，
必须先在写者中规划成明确的可撤销事务，再捕获保存快照；IO 线程不得暗改正文。
实际编码器/GB18030 标准、原子写盘及链接策略由 T07 固定和测试。

## 4. 视图与模式

**C05 独立 ViewState。** 每 View 拥有 mode、SelectionSet、主选区、scroll
anchor+pixel bias、折叠、焦点、composition/preedit、ViewEpoch。
来源锚点为 `(DocId, Revision, SourceByte, Before/After affinity)`；
同一插入边界 Before 留在插入前，After 留在插入后；被删除区间内部锚点收敛到
替换区起点/终点的显式 affinity 结果，不能随意跳到活动视图。
独立拥有是逻辑归属，不表示各 UI 线程可任意写权威锚点：视图意图也经过同一写者
协调，内容提交与相关锚点映射对读者同批可见；平台像素/焦点观测可作本地缓存。

正文提交后所有视图映射到新 revision；只对发起视图应用明确的 selection_after。
其它视图位置独立映射，不复制发起者的选区/滚动。共享的是内容和 dirty，不是位置。
渲染器持有只读快照；两个窗口仍是一个 DocumentStore/一条 history。

**C06 纯模式切换。** PlainText/MarkdownSource/MarkdownWysiwyg 为视图模式。
能力未提供、映射不明确或准备失败时返回 NotSupported/MappingUnavailable，
保留原模式/位置，不拿只读预览充当已实现 WYSIWYG。

`request_mode(view,target)` 在 IME Idle 时仅产生视图请求；如有 composition，
返回 CompositionActive，要求调用者先通过系统输入上下文单独完成 commit/cancel。
不能清空 overlay 冒充取消，不能把隐含 IME 提交算作模式切换的文本操作。
IME commit 如发生，是独立可追踪内容事务；下述不变量从其完成后的切换基线计算。

投影准备期间保留原视图。结果须匹配 DocId、Revision、ViewId、ViewEpoch、JobId，
才可交换派生视图并映射位置；过期或取消则丢弃/重新请求最新版本，不覆盖新位置。
模式成功切换、失败和取消都必须保持：**0 文本事务、0 undo 增量、revision/token/
dirty 不变、source/EOL/provenance 不变**。不能通过 serializer/setContent 完成切换。
这些是模式操作本身的不变量；并发的其它视图编辑须有独立 TxnId/receipt，不能为了
制造“0事务”而冻结、丢弃对方已接受的输入，也不能把对方提交误归因于模式切换。

## 5. 事务、提交和事件

以下为伪码，不是产品已有 API：

```text
snapshot(DocId) -> immutable DocumentSnapshot
submit(EditTxn{doc, base_revision, origin, boundary_policy,
               source_edits[], eol_delta, provenance_plan,
               selection_before/after, kind, history_group})
    -> CommitReceipt | TxnError
undo(doc, invoking_view) / redo(doc, invoking_view) -> CommitReceipt | Error
subscribe(doc, after_event_seq) -> ordered DocEvents | Lagged
```

**C07 全部验证、一次发布。** 编辑范围均以同一 base_revision 为坐标基准。
输入 planner 先确定多光标重复/相交选区的合并意图；submit 再权威验证有序、
不重叠、不含歧义的 edits。相同偏移多个不同插入、交叠 patch 不自行猜先后顺序；
返回 InvalidOverlap，不能先应用其中有效部分。各 edits 的坐标不是前一 edit 后的位置。

单写者处理顺序：

1. 校验存活 DocId、origin 的 View/服务身份、当前 base_revision、取消状态与能力。
2. 校验全部坐标、边界策略、文本/EOL/provenance 一致性和完整 selection 计划。
3. 在未发布候选上准备正/逆 delta、history、全部视图锚点映射、事件批次和资源。
4. 发布前再次确认取消/代次；原子交换完整 DocumentState、history 与关联映射，
   revision 前进一次、生成/恢复正确 token。内存分配/验证失败不发布。
5. 发布 CommitReceipt 和有序事件，然后调度派生任务。不能在订阅回调中重入写者；
   新意图重新排队，不依赖多个 mutex 的偶然获取顺序。

InvalidDoc/View、NotReady、StaleRevision、InvalidBoundary/Overlap/EolPlan、
ReadOnly、ResourceUnavailable、Cancelled 均不改变正文/EOL/provenance/revision/
token/history/已发布选区，不发送成功 ContentCommitted。只记录拒绝原因和身份，
不默认把用户正文塞入诊断日志。提交点后取消返回 AlreadyCommitted，不能报告“未发生”。

严格无效检查不做隐式 rebase。交互意图可在最新快照重新规划；Replace All 必须
重新检索/确认对应结果，不重复提交旧 patch。全状态无变化的 NoOp 不加 revision、
token 或 undo；此规则不把不同序列化元数据变化误判为空操作。

**C08 事件可区分。** 每 DocId 的 EventSeq 单调有序。ContentCommitted 包含
prev/new revision、TxnId、token、增量、history/dirty 变化和对应视图映射。
SavedBaselineChanged/SaveStatusChanged 不自带文本 delta，不增加内容 revision；
ViewChanged/ModeChanged 使用视图事件，不制造内容事务。
订阅方检测重复序号并忽略；发现缺口/Lagged 必须拉取新快照，不能在漏 delta 后继续套用。
提交点是内存权威状态的一次发布，不保证多个 HWND 同一物理时刻绘制，也不等于磁盘持久化。

## 6. 唯一 history、状态 token 和 dirty

**C09 共享历史。** 所有输入、IME 一次确认、富编辑步骤、粘贴、Replace All、
显式编码/EOL 变更进入同一内容 history。View 选择、滚动、主题和模式不进入它。
Undo/Redo 始终作用于文档最近可撤销/重做组，不限发起窗口或模式。
调用视图定位到作用区，其他视图只映射已有锚点；不得同时调用另一个控件的私有 undo。

Undo/Redo 各自是一次原子内容发布，revision 继续递增，恢复目标 StateToken；
它们不是“将 revision 调回旧数”。Undo 后新内容编辑丢弃 redo 分支，但不得删掉
活动保存快照或 saved token。历史裁剪保留 token 判定依据，不拆开一个事务。
历史预算、分段文件和输入合并时间窗口在 T03 锁定，不将工程建议当作已验性能保证。

合并只允许同视图/相容输入意图。跨视图、导航、模式、获接纳的保存请求、IME commit、结构命令
结束均关闭当前合并组。关闭合并组本身不是内容事务，不增加 undo 组数；
保存捕获前先冻结合并边界，避免保存点落在不可单步返回的未结束组内部。

**C10 dirty 定义。** `dirty = current_state_token != clean_baseline.state_token`。
clean_baseline 是已严格打开/重载的磁盘状态或已确认成功保存的捕获状态。
新建空文档可以有内存 InitialEmpty baseline（本候选选择，待审）；它可 dirty=false，
但 backed_by_disk=false/needs_target=true，不得显示为“已写入磁盘”。

token 是状态身份，不是内容 hash：Undo/Redo 回到旧状态必须复用正确 token；
独立编辑出碰巧相同字节不自动复用 token，仍可 dirty。此选择避免逐键全文相等扫描，
不影响“Undo 回保存状态 clean”的必需行为。只读快照、来源缓存刷新不改变 dirty。

disk_conflict、save_outcome_unknown 和 needs_target 是独立字段，不能靠 dirty 一位
表达所有文件状态。即使 dirty=false，磁盘结果不确定也不得显示保存成功或继续无检查覆盖。

## 7. 保存操作与取消状态机

**C11 快照保存。** begin_save 由写者捕获 `(SaveOpId, DocId, Revision N,
StateToken S, immutable snapshot, BindingEpoch, TargetKey, fingerprint guard)`。
同一 DocId 和 TargetKey 同时最多一个 active operation；跨 DocId 也协调目的文件。
TargetKey 的别名/不存在文件解析必须由 T07 定义，不能只用未规范路径字符串。

已有 active save 时新 begin_save 返回 Busy、无 IO/状态副作用；它不是已接受的保存。
调用层必须明确保留待重试意图或反馈 Busy，不得静默丢用户请求或伪造完成。
不允许多个写者乱序写同一目标，再靠丢弃旧 UI 回执掩盖磁盘旧版本覆盖。

ReadyToReplace 到 Replacing 需要写者发放该操作的一次性 CommitPermit；取消与
permit 发放在同一写者顺序中裁决。已确认 Cancelled 的 worker 不得再取得 permit，
只检查一个可能竞态的 bool 不够；permit 发出后取消保守答 TooLate，即使 OS 调用尚未返回。

| 状态 | 可进行操作 / 转移 |
|---|---|
| Idle/终态后 | begin_save 捕获并获取目标 lease，进入 Encoding |
| Encoding | 严格编码捕获快照；不可表示则 Failed，不写目标；取消可到 Cancelled |
| WritingTemp | 写隔离临时文件；取消可清理或保留有说明的临时文件，目标未替换 |
| ReadyToReplace | 复核 fingerprint/授权/取消；外变到 Conflict；取消到 Cancelled |
| Replacing | 已进入不可撤回提交区；取消答 TooLate，继续核实实际结果，不谎报未写 |
| Verifying | 核实文件身份/内容/备份结果；成功到 Succeeded；不确定到 OutcomeUnknown/Conflict/Failed |
| Succeeded | 发持久化回执，写者一次更新 baseline/磁盘绑定/指纹，释放 lease |
| Failed/Conflict/Cancelled/OutcomeUnknown | 保留当前内存正文和历史；释放/封存 lease与临时/备份处理由实际磁盘结果决定，不默认删除唯一可恢复版本 |

lease 不阻止其它进程写文件，不是文件系统 compare-and-swap。原子替换、备份、
事后冲突检查和失效路径属于 T07；本契约不宣称仅指纹预检消除了所有竞态。

**C12 回执接纳。** 只有匹配 active SaveOpId、存活 DocId、捕获的 BindingEpoch、
TargetKey/guard、未消费回执，并有明确成功持久化结果，才更新 clean_baseline。
Save As 的路径绑定只在成功后更新；编码变化若有，必须在捕获前作为内容事务。
保存不清空 undo，不给当前正文 apply 任何旧 source delta。

接纳成功回执时允许 `current_revision > captured_revision`：只将捕获 token S 标为
磁盘保存基线，重新比较当前 token。当前已编辑 N+1 则继续 dirty；若 Undo 恰好回 S，
即使 revision 更大也可 clean。这是保存回执与普通派生结果严格 revision 校验的区别。

重复、已被取代的操作、关闭/重开后的旧 DocId、旧 binding 回执不更新现文档。
若 IO 已越过替换点，拒绝回写内存不等于撤销了磁盘副作用；保留该操作的结果和冲突提示。
文档关闭/重新绑定必须先与 active save 协调，不能偷偷复用 ID 或释放目标 lease。
Failed/Cancelled 不伪造 saved token；OutcomeUnknown 单独标注并阻止假“已同步”。

## 8. 派生异步、重载和后续实现边界

**C13 旧结果拒绝。** 高亮/Markdown 投影/搜索/布局等 job 捕获 DocId、Revision、
JobId、取消代次；涉及视图另带 ViewId/ViewEpoch。结果必须同时匹配才可发布。
相同文本或 token 不豁免 revision 检查（Undo 后的 ABA 仍拒绝旧结果）。
失效结果只丢弃/重新规划，不修改正文、历史、模式或较新的位置。取消不等于 worker
已停止，发布前代次检查仍必需。需要提交文本的结果转为正常 submit，再作版本验证。

clean 外部重载按配置先取得允许，准备完整快照后用 DocumentReset 一次换基线、映射
位置并清旧 history，revision 前进、旧派生结果失效；不是可撤销的普通编辑。
dirty 重载必须先完成明确保存/放弃/取消决定，不能靠后台 IO 覆盖。编码重开等亦同。
重载/关闭授权和磁盘别名细节不在此实现，由 T07/T11 落实。

composition/preedit 不写权威正文；确认经一个内容事务，取消无内容 delta。
跨视图编辑使 composition 基准失效时，发 RequiresCompositionResolution；
不得用旧选区提交、重复提交或直接清 overlay 冒充系统取消。T12 适配必须补齐系统输入路径。

## 9. 审核闸口

Rusty 审查上述约束和 EXAMPLES 预期后，才可推进实现。需特别审定的本候选选择：
InitialEmpty 的内存 clean baseline、token 身份而非任意字节相等判 dirty、Busy 的
调用方责任、保存取消不可撤回边界及最终链接以外未实现内容的边界。
这些选择已明确写出行为，不以含糊 TBD 隐藏竞态。

T03 才固定缓冲/坐标库/历史预算；T04 证明 NSH 外部正文和历史适配；T05 验证局部
patch/SourceMap；T07 实现编码/路径身份/实际写盘；本文件不替代这些实现与实测。
当前产品仍是窗口基线，以上协议及其应用行为全部尚未实现/NOT_RUN。
