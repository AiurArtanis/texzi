# T05/T06 原生富文本局部回写第一步

此阶段是未验收候选，不代表完整T05/T06或G2通过。首版原范围保留，未实现项不能改成只读预览后宣布完成。

- 启用固定NSH的rich-editor，与原源码Editor同一便携程序。工具栏“源码/富文本”切换同一正文；独立逻辑View共用唯一Store/history，RichEditor starter history明确关闭，Undo/Redo回Store。
- Markdown1.0.0及serde_json1.0.154原已在锁文件，新增直接依赖关系；未升级原包版本。实际features/全部依赖/SBOM/许可继续由候选CI记录。
- 按mdast顶层源byte区间建立投影，保留区间外空白/EOL；单text leaf且原文匹配唯一时做Unicode标量边界最小替换，结构编辑只序列化改变的连续块，不整篇serialize。补丁先重解析并与目标rich树一致，选择边界可严格映射，才提交唯一事务；不支持的操作显示错误，正文保持不变。
- HTML、定义和脚注等无法无损表达的语法作为可见codeBlock源码局部编辑，原件不丢弃；禁止把含opaque的跨块修改偷偷序列化。此局部源码fallback不是所有结构WYSIWYG已完成。
- 每次富文本写入校验doc revision和View epoch，选择更新不增加正文事务。投影仅缓存，原生回调通过UI线程弱注册表访问Owner，无unsafe Send/Sync。
- RichEditor自己的Windows隐藏sink也调整剩余视口宽度；不能继承源码IME测试为rich IME已通过。

## 覆盖与待补

组件新增：混合EOL/未知块旁的标题emoji局部修改、UTF16代理对内严格拒绝、opaque局部原字编辑，以及100次纯投影重建不写Store/token/history/dirty。组件测试的100次不是100次真实GUI切换证明。

真实GUI必须用本轮CI下载的新exe验证输入→切源码→Undo/Redo；旧2d不作新hash运行证明。实际GUI/IME、选区/滚动映射、100轮切换、全部指定结构、位置/性能/失败矩阵均待本轮及后续分项证据，不先写PASS。

当前仍有明确限制：投影每次富写重解析，未做增量CST、稳定BlockId/虚拟化/大文档预算；纯文本leaf以原文唯一匹配定位，复杂转义/重复文本可能拒绝操作；富视图恢复位置与多选能力未完成。已存在源码多选与逻辑独立View保留。上述为PARTIAL，不取消首版要求。T07多编码/EOL/安全写/冲突原范围保留，尚未接磁盘IO。旧剪贴板恢复未证明，本轮严格无剪贴板访问/快捷键C/X/V。

## 80df467 源码预审后的收口

原80df467构建成功也不代表可接受：原Rusty源码预审指出opaque整块回写会归一未改EOL、纯选区映射失败仍发布本地选择，以及模式准备/组合保护不对称。原记录和白盒FAIL保留，不运行中间GUI冒充修复。

修订将语义块补丁再拆为grapheme差分，匹配的原文/换行不进普通输入替换，保留Store已有EOL/provenance；仅新增换行用原输入策略。若现有多行块内的换行身份无法确定，则提交前明确拒绝，原文/token/history/两逻辑View不变。该限制保留为未完成要求，不宣称完整结构操作已支持。imara-diff0.2.0与unicode-segmentation1.13.3均为原锁文件已有版本，本次仅增加直接关系。

纯选区两端必须都能映射；失败不发布Rich state。未同步选区的输入/history也拒绝；没有源变化却改变富树的不可表达结构不得只提交到本地。

模式由ModeGate先准备完整RichOwner，再发布模式；失败保留原可编辑源码区。原生source组合经ExternalDocument临时状态、rich组合经session监听更新两个根级信号，组合期间禁用按钮且请求入口再次拒绝；这些不是正文事务。TEXZI_T05_FAIL_PREPARE=1为显式隔离测试故障，作用于真实投影准备后/发布前边界，默认不启用、不写用户文档。

新增真实Owner/Store组件路径反例覆盖opaque混合EOL单字、普通多行加粗/来源/UndoRedo、无法映射选择及后续输入/history、普通选择同步、多行换行不支持时原子拒绝、模式双向组合/准备失败、不可表达结构无本地提交。组件仍不能替代真实TSF和GUI；新候选CI与本机实测后另记真实结果。

简单可逆的源位置在重建时恢复；无法映射的初始位置会在发布模式前拒绝，保留源码区。输入/history还核对富选择与权威View是否一致。复杂位置/滚动和Markdown无法表达的空段落仍未完成，相关结构/换行操作可能明确拒绝；不能称完整Enter/结构编辑或G2已通过。

3a0bcc9作者实际下载运行13/13组件通过；首GUI轮实际富输入和加粗可回写，手动聚焦源码后跨模式Undo/Redo正确，但切源码后自动焦点未恢复，直接快捷键无响应。原图/元数据/owned app0/无强清记录保留，不作整体PASS。后续将source auto_focus与原RichEditor一致延至挂载提交后，并提供必要标题/marks/代码/表格语义类；不是完整视觉或结构矩阵通过。新exe须重新验证键盘焦点、样式、组合和失败边界。

## 构建和产物

后续段落候选：顶层空段落映射与具体 Enter/合并规则见 `../decisions/2026-11-17-rich-empty-paragraphs.md`。新增真实 Owner 测试覆盖段首/中/尾、空正文、Unicode 标量边界、连续空段继续输入及 Backspace 合并。提交前仅 fmt/diff 静态核对，组件及 GUI 结果须绑定新的 CI 身份，不能继承 18cc55e 的运行结论。

README现有cargo build --release --locked --target x86_64-pc-windows-msvc命令不变。CI额外编译执行app单元测试，将真实rich-projection.exe与原双流/exit随包交独测；失败不打成功候选。原文档组件/窗口失败路径证据保留。

补充：固定12语料每例100次无修改投影、跨逻辑View撤销/重做与原混合EOL字节恢复均为组件检查；真实GUI次数另计。显式长度诊断下的TX05_VIEW记录revision/token/history/dirty供核模式切换0文本事务，默认关闭，不输出正文。MIT移植许可补齐与blob来源见LICENSE-SOURCES.json；原NSH未注明移植所用PM/Tiptap revision，不自行编造。
