# T03b：逻辑视图、选区计划与共享历史组

在已接受的5a14645 T03a有限组件上继续。本步是内存组件；不是原生编辑器或T04/T05放行。
旧T03a文档/失败/审测证据保留。验证入口见[PROTOCOL.md](PROTOCOL.md)。

## 接口和发布边界

`Store`仍为正文/历史/逻辑视图的唯一可写owner。`open_view`分配进程内不复用的ViewId；
ViewState包含DocId、epoch、独立SelectionSet/primary、scroll+affinity/pixel_bias及逻辑ModeTag。
这里的Plain/Source/Rich只是组件标签，不是NSH projection/WYSIWYG/IME能力。

`submit_view(ViewTransaction)`权威验证origin View存在、绑定Doc及expected_epoch，
base revision正确、全部patch及显式selection_after有效；默认UI计划只允许grapheme边界。
selection_after必须使用**实际候选版本**的Doc/Revision，含非空规范化SelectionSet和合法primary。
正文NoOp时仍用原revision，不能盲写base+1。无效origin/位置/选区、旧epoch、预算或取消
都不发布正文、history、视图或合并状态；不先提交正文再补视图验证。

写者在临时候选上准备正文/EOL/provenance、history与所有关联View，完成验证后在同个
独占调用里交换。`read_document`同一次只读调用取得正文快照和所有逻辑View版本。
它**不是**C08 ObserverSnapshot，没有EventSeq、subscribe或异步恢复保证。

`submit`保留为受信任文档服务入口，自动映射所有View并切断输入合并；未来视图适配必须
使用submit_view，不能绕过origin/selection_after校验。此处不是安全身份认证/线程队列实现。

内容提交只对origin应用显式selection_after，其它View各自映射选区、滚动；不复制活动View。
所有相关锚点带新revision，View epoch同时前进，使导航/切换后的旧计划被拒绝。
映射到新grapheme内部时，Before向前/Floor、After向后/Ceil，`mapping_snaps`返回requested/actual/affinity；
这属于明确的映射吸附，不冒充Exact坐标转换。原始selection_after自身严格校验，不自动修正。

## 多选区意图和事务

`normalize_selections`是显式输入planner：按range排序；合并重叠范围、重复carets、
落入或接触已选择范围的carets；相邻非空范围保持独立。保留包含原primary的组为primary，
合并组方向由primary成员决定；无primary的组按排序首成员方向，外边界Before/After。
非法标量/grapheme位置先报错。权威set_selection/submit_view拒绝未规范化的计划，
不会把两个不同文本的重叠patch猜成合法事务。

`plan_replace`从当前View的规范化SelectionSet生成同base的多段替换和候选caret；
所有段一次提交、一个undo组。若插入后的caret不在grapheme边界，Exact计划报错，
不悄悄丢字或截断。矩形选择、UI hit-test和系统IME在T04/T12继续。

## 历史合并与fence

每Doc一条history，任一逻辑View可Undo/Redo其它View的最近组。调用View恢复该组
before/after的作用选区并改绑当前revision；其它View只映射自己的位置，scroll不复制。

初始工程选择：同View、单caret、相容的连续普通Typing插入、无LF、间隔0–750ms
（含750，时钟倒退切组），合并后最多64KiB插入UTF8。这是选型值，不是实测性能保证。
每次内容提交仍增revision/生成token；合并只改变撤销粒度，正逆delta聚合成连续插入/删除，
Undo一次恢复整组前token，Redo恢复整组后token。

跨View编辑、导航/scroll、逻辑mode意图、成功接纳的保存捕获、服务事务、Undo/Redo、
非Typing的Paste/ImeCommit/Structural以及显式fence均阻止跨边界合并。
失败/Busy请求不切掉已有合法组；接受的非Typing NoOp也切组但不加文本revision/undo。
保存捕获先固定当前组尾，后续输入另起组，因此保存N期间编辑N+1后Undo能回N clean。
不允许“给最近一组打标”来抹去保存点或把跨View历史变成独立undo。

沿用T03a的64/256MiB accounting ceiling；合并组累加每次提交计费，不声称减少真实进程内存。
硬内存、历史回收/段文件、分配失败恢复、计数/提交后取消故障以及C08/R4留后续，未补成PASS。

## 验证和依赖

组件suite继续执行旧13项，并增加EX01/02/04/06逻辑行为、非法selection_after零部分提交、
跨View Undo、合并/导航/跨View/保存点、去重/primary多选区及grapheme映射吸附。
EX02只测逻辑标签下共享history；不是两HWND或真实模式投影/IME测试。
外部Python byte-string oracle通过document-probe检查128个固定seed多edit的内容、逆delta和被动anchor。
这仍是实现者的测试；Steamy可以独立更换oracle和输入，不是只能重跑固定suite。

默认库不新增依赖。可选`probe` feature精确依赖serde_json1.0.154（MIT OR Apache-2.0），
Cargo.lock已包含其传递依赖；不升级既有版本。CI记录默认库和probe各自实际feature树及许可/SBOM。
本机链接环境不安装；启用probe的本机check触发serde build-script linker缺失，记录为检查失败，
不能把默认库check成功冒充probe构建成功。完整probe/组件/程序构建由现有CI完成，再下载复验。

```powershell
cargo fmt --all -- --check
cargo check -p texzi-document --tests --locked --target x86_64-pc-windows-msvc
cargo build -p texzi-document --features probe --bin document-probe --release --locked --target x86_64-pc-windows-msvc
cargo test -p texzi-document --release --locked --target x86_64-pc-windows-msvc --test contracts -- --include-ignored --nocapture --test-threads=1
python scripts/test-document-probe.py --probe target/x86_64-pc-windows-msvc/release/document-probe.exe --trace build-evidence/probe-oracle.jsonl
```

CI与脚本命令一致，全部组件和oracle成功后才构建应用/打包；失败保留native双流/exit和失败组件exe。
新程序包供下载绑定新commit/run/hash。NSH窗口仅保留既有基线，不接真实编辑、保存或完整工作台。
