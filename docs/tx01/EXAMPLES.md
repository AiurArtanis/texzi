# TX-01 / T01 行为示例与预期

这是 [CONTRACT.md](CONTRACT.md) 的审核用输入/预期，**不是产品测试结果**。
当前窗口程序没有实现 DocumentStore；本表应用行为一律 NOT_RUN。
R 表示 Revision，S 表示 StateToken，B 表示 clean baseline token，H 表示可 Undo 组数。
各例独立初始化，不把不同 DocId 的 R/S 做大小或相等比较。

## EX-01：共享正文，独立位置（C01/C05）

初态 D1：`cat\nsecond\nthird`，R0/S0/B=S0，dirty=false。
V1 与 V2 属于两个窗口，同引 D1；V1 Source caret=0、scroll=0/bias0；
V2 Wysiwyg caret=16、scroll anchor=4/bias24（均为 SourceByte）。

V1 在 byte0 插入 `X`，提交一次有效事务：

| 字段 | 预期 |
|---|---|
| 权威内容 | `Xcat\nsecond\nthird`，R1/S1，H1，dirty=true |
| V1 | caret=1（显式 selection_after），scroll anchor仍0/Before、bias0 |
| V2 | caret=17、scroll anchor=5、bias24；不是复制 V1 的 caret/scroll |
| 两个投影 | 消费同一 R1 delta；不得各自写一遍再形成两条 undo |

V2 的投影暂未完成时允许显示明确准备状态，但不得将旧投影重新序列化覆盖 R1。
改变 V2 的选择/滚动不改变 D1 的 R/S/H/dirty，也不改变 V1 的位置。
删除区间内的其它视图锚点按 affinity 收敛，不承诺它的全部 UI 历史由内容 Undo 恢复。

## EX-02：跨窗口、跨模式的唯一 Undo（C06/C09/C10）

D2 Markdown 逻辑原文 `hi`，R0/S0/B=S0/H0；V1 Source、V2 Wysiwyg。

| 操作 | 正文 | R / S | H / redo组 | dirty |
|---|---|---|---|---|
| V1 输入 `!`，G1 | `hi!` | R1/S1 | 1/0 | true |
| V2 映射到最新R1后富编辑插入 `?`，G2 | `hi!?` | R2/S2 | 2/0 | true |
| V1 Undo | `hi!` | R3/S1 | 1/1 | true |
| V1 切为 Wysiwyg | `hi!` | R3/S1 | 1/1 | true |
| V1 再 Undo | `hi` | R4/S0 | 0/2 | false |
| V2 Redo | `hi!` | R5/S1 | 1/1 | true |

Undo 第一回撤的是 V2 最近的 G2，不是 V1 自己的 G1；Undo 调用视图定位到作用区，
其他视图只映射锚点。模式切换没有增加历史；R4 不等于 R0，但 S0 相同所以 clean。
若此时在 R5 新输入而非 Redo，redo 分支清空，saved baseline 仍 S0，不被剪掉。

## EX-03：无改动模式往返（C04/C06）

原字节对应 `**a**\r\n\r\nx  \n<!--unknown-->\r`，编码/BOM保持打开时设置。
逻辑 LF source 与 EOL 表分开，provenance 保留原跨度。R7/S7/B=S7/H3。

IME Idle，Source→Wysiwyg→Source 为一轮，共100轮（200次成功模式切换）。
每次投影只读当前快照；预期 source/EOL/编码/BOM/provenance 不变、R7/S7/H3
不变、dirty=false；文本事务计数增量0、undo组增量0。另一个 View 的模式不变。
后续未改保存应 byte-exact；这项实际 IO 由 T07 验证，本候选没有执行保存。

准备失败、NotSupported、MappingUnavailable 或用户取消：保持原模式与位置；
不得用 setContent/全篇 serializer 作为回退，也不悄悄变成只读预览。

## EX-04：保存 N 时继续输入 N+1，完成后 Undo 回保存点（C10/C11/C12）

独立 D4：当前 R10/S10=`draft`，旧 baseline=S9，dirty=true。保存请求冻结输入合并组。

| 步骤 | 当前内存 | baseline | 保存状态 / 预期 |
|---|---|---|---|
| begin_save O1 | R10/S10 | S9 | 捕获不可变 R10/S10，Encoding |
| 用户输入 `!` | R11/S11=`draft!` | S9 | 新输入组，不修改 O1 快照；dirty=true |
| O1 完成并核实磁盘 | 仍 R11/S11 | **S10** | 磁盘是 `draft`；接纳当前 active O1 的成功回执，不 apply旧正文；dirty仍true |
| Undo 输入 `!` | R12/**S10**=`draft` | S10 | dirty=false；不是把revision退到10 |
| Redo | R13/S11=`draft!` | S10 | dirty=true |

保存不增加内容 history、不清空 Undo、不以 `R11 != saved_revision10` 永久判脏。
若回执到达前已 Undo 回 S10，成功回执同样可让当前更大 revision 的 S10 clean。

## EX-05：Undo ABA 不允许旧派生结果回写（C03/C13）

D5 初态 R0/S0=`a`；输入 `b` 后 R1/S1=`ab`。J1 捕获 `(D5,R1,job=1)`。
再输入 `c` 为 R2/S2=`abc`，Undo 后为 R3/S1=`ab`。

J1 此时返回：尽管 token 和内容回到 S1，R1 != R3，必须拒绝发布；
不能覆盖新视图位置/高亮/搜索结果。正文、history、dirty 不因拒绝而改变。
新任务 J2 必须基于 R3。保存回执的例外只适用于 EX-04 那样仍合法 active 的
SaveOp 与已核实的磁盘结果，不能借“都是异步”把 J1 当保存回执接纳。

## EX-06：多编辑事务全部有效才提交（C07/C08）

D6 初态 `猫😀`，R4/S4/B=S4/H2。两个 edits 均以 R4 为准：
`[0,3) -> 狗` 合法；`[4,5) -> x` 落在 emoji UTF-8 编码内部。

预期整个事务 InvalidBoundary：仍 `猫😀`、R4/S4/H2、dirty=false，
EOL/provenance、所有已发布选区不变，没有 `ContentCommitted`，不出现先改成狗的中间状态。

另一个独立 `abcd` 事务同时替换 `[0,2)` 与 `[1,3)`：即使均是 ASCII 边界，
也必须 InvalidOverlap，不能只取其中一个。两个不同文本的同点插入也不默猜顺序。
base_revision 过期、发布前取消、资源准备失败均有同一“零部分提交”要求。
若已过提交点才取消，答 AlreadyCommitted 并给已存在 receipt，不谎报事务没发生。

合法多光标例：`abcd` 的 `[0,1)->A` 与 `[3,4)->D` 同 base，结果一次发布 `AbcD`，
R前进1、一个history组、一次Undo还原整次操作；不是将第二个范围按第一次修改后重算。

## EX-07：Unicode 边界与严格转换（C03）

fixture 原样为 `A😀é👩🏽‍💻🇨🇳B`，其中 `é` 明确是 U+0065 U+0301，不能归一化。
下列六段为预期的扩展 grapheme；库/Unicode 版本将在 T03 固定，不能用本表冒充实现测试。

| 边界 | SourceByte | Utf16Offset | GraphemeOffset |
|---|---:|---:|---:|
| A之前 | 0 | 0 | 0 |
| 😀之前 | 1 | 1 | 1 |
| e+组合重音之前 | 5 | 3 | 2 |
| 👩🏽‍💻之前 | 8 | 5 | 3 |
| 🇨🇳之前 | 23 | 12 | 4 |
| B之前 | 31 | 16 | 5 |
| EOF | 32 | 17 | 6 |

- SourceByte2：在😀的 UTF-8 内部，Exact 转换/编辑 InvalidBoundary。
- Utf16Offset2：在😀的 surrogate pair 中间，InvalidBoundary。
- SourceByte6 / Utf16Offset4：在 e 与组合重音之间，标量边界合法；
  Exact Grapheme 转换 InsideGrapheme。UI只能显式吸附至 byte5 或 byte8，不能静默误报 Exact。
- Grapheme3 精确映射 byte8/UTF16=5。默认删除该 cluster 不能只删肤色、ZWJ 或 laptop。
- 将旧 revision 的 byte8 带到新正文：先映射 anchor 或返回 StaleRevision，不沿用旧数值。

行列取规范 LF，不把视觉软折行当新行；tab 显示宽度不是字节或 UTF-16 列宽。

## EX-08：混合 EOL、原始字节偏移和来源（C04）

原文件无 BOM，字节对应 `one\r\ntwo\nthree\rfour`；没有末尾换行。
规范 source=`one\ntwo\nthree\nfour`，EOL表 `[CRLF,LF,CR]`，LF的 SourceByte 为3/7/13。
原文件 `two` 的 OriginalByteOffset=5，规范 SourceByte=4，二者不能混用。

把 source `[4,7)` 的 `two` 改为 `TWO`：只失效该段来源；原有三个 EOL 不变。
预期序列化 `one\r\nTWO\nthree\rfour`，不统一换行、不补末尾 LF。Undo 恢复
旧 token、原跨度和原字节，而非重新编码整篇“看起来一样”的文本。

在此状态的 source byte7 插入一个逻辑 LF：原三种换行计数平票，采用首个 CRLF。
EOL 表变为 `[CRLF,CRLF,LF,CR]`，原 LF/CR 不改；预期写回
`one\r\nTWO\r\n\nthree\rfour`。一次 Undo 删除该新 LF 和对应表项。
如果 caller 只插 LF 却遗漏表项，整个事务 InvalidEolPlan，不能提交一半。

## EX-09：模式任务取消/位置代次与 IME（C06/C13）

V9 当前 Source，D9 R8；请求 Wysiwyg 的任务 M1 捕获 ViewEpoch5/R8。
用户取消该模式请求或产生使位置计划过期的新请求，ViewEpoch前进。
M1 晚到：必须拒绝，不把模式改成 Wysiwyg、不把光标恢复到旧位置；
D9 R/S/history/dirty 始终不因取消/晚到而改变。

正在 IME preedit 时直接 request_mode：返回 CompositionActive、0文本事务。
系统取消组合后，正文/revision/dirty/undo不变；再模式切换仍0事务。
系统确认组合则先作为一次独立IME事务，随后以新基线切换模式，模式本身仍0事务。
不能把后台清空 preedit、丢弃未确认输入或隐含提交伪称“模式切换成功且0事务”。

## EX-10：保存失败、取消与不可撤回阶段（C11/C12）

每行独立初始化 dirty=true、旧 baseline=S0、当前S1。

| 场景 | 预期内存和 IO 语义 |
|---|---|
| 捕获快照含😀、严格GBK编码 | Encoding阶段 Unencodable，目标不替换，不写问号；S1/dirty/history不变，baseline仍S0 |
| WritingTemp时取消，写者先确认取消、未发CommitPermit | Cancelled，目标未替换；临时文件按有记录策略处理，晚到worker不能再拿permit；不伪成功 |
| CommitPermit先发出，随后取消 | TooLate；继续核实替换结果，不假报Cancelled/目标未写；最终真实失败或成功分别处理 |
| 复核发现外部修改 | Conflict，保留内存/恢复线索，不自行覆盖；后续另存/覆盖由明确选择和新操作处理 |
| 已替换后校验失败或结果不确定 | OutcomeUnknown或明确失败，不能标保存成功；保留可恢复副本和实际磁盘状态，不能假称原文件绝对没动 |
| clean文档保存时发生OutcomeUnknown | dirty可仍false，但disk/save attention状态必须显示不确定，不显示“已同步” |
| 同Doc或同TargetKey有active保存 | begin_save返回Busy，不接受第二个并发写；调用者不得将它当已完成保存 |

“已确认取消且目标没替换”依赖 CommitPermit 协议，不是丢弃 UI 回执就能实现。
若 worker 违反协议已写入，拒绝晚到结果不能抹掉磁盘副作用，必须报告冲突/不确定。

## EX-11：重复/过期保存回执不能回退基线（C01/C12）

O1 成功保存 S1，回执已消费并释放lease；后来 O2 成功保存 S2，当前也是 S2/clean。
此时重复到达 O1：拒绝 AlreadyConsumed/InactiveSave，baseline保持S2、dirty=false，
正文/history/revision不变；不能把baseline降回S1。接收回执本身不触发再次写盘。

文档关闭后以新 DocId 打开同一路径，旧 DocId 的回执同样不得作用于新文档。
关闭/绑定协调与实际 IO 副作用分别核实，不把身份拒绝当作磁盘动作已经撤回。

## EX-12：序列化元数据与纯元数据（C02/C04/C10）

R0/S0/B=S0 的逻辑 `x\n`，EOL=CRLF；显式转换为LF是一个内容事务：
R1/S1、dirty=true，即使屏幕文字相同。Undo 为R2/S0、dirty=false，EOL恢复CRLF。
编码/BOM显式变更亦同。来源缓存刷新或成功Save As路径绑定不凭空增加内容Undo。

新建空文档 InitialEmpty baseline可dirty=false，但无磁盘目标；
显式保存仍需要目标选择，不能以dirty=false直接返回“文件已保存”。
取消目标选择不创建SaveOp、不写盘。独立删除再输入同字节不自动变成saved token，
但Undo回保存节点必须clean；若要改变此token语义，必须先审改契约与预期。

## EX-13：初次观察与内容提交交错（C02/C07/C08）

下文observe均指原子的 `observe_and_subscribe`，last_complete均指
`last_complete_cursor`；W/C为完整批次水位，g为SubscriptionId。所有未列出的域也必须包含在
ObserverSnapshot中；不能用简写表格省掉实际接口应返回的baseline/save/history/View状态。

服务器D13在W20：R10/S10=`abc`，baseline=S9，dirty=true，H1/redo0，
save=Idle、attention无冲突/不确定、needs_target=false，scope View V1 caret3。

| 顺序 | 一致观察返回 / 流 | 消费后的确定结果 |
|---|---|---|
| observe先于输入提交线性化 | snapshot水位20，完整状态如上；g1已原子注册从21开始 | 一次安装全部域，last_complete=20 |
| 随后V1插入`!`，发布完整B21，发生在observe返回值传输期间也同样 | g1必须交付B21，含R11/S11=`abc!`、H2、caret4等变化 | 安装snapshot20后整批应用21，last_complete=21，不漏插入 |
| 反过来，输入先发布B21再observe | snapshot水位21已含`abc!`、H2、caret4；g2从22开始 | 不再把21的delta应用一遍，不能得到`abc!!` |

不允许取得snapshot20之后另取最新游标21，再注册22；这种组合正是会漏B21的非法接口。
若B21早于snapshot安装到达，先暂存；若期间队列溢出则按EX-16重同步，不在旧缓存上套它。

## EX-14：同revision保存变化与重连/实时衔接（C08/C10/C12）

D14客户端已有完整缓存C30：R10/S10=`abc`、baseline=S9、dirty=true，
active O1捕获S10且正在Verifying，H1/can_undo=true、redo0，V1 caret1。
订阅连接中断，但发布端继续发布以下批次；保存结果不修改正文revision。

| 批次/步骤 | 服务器变化 | 恢复要求 |
|---|---|---|
| B31 | O1成功：baseline=S10、dirty=false、active=None/终态Succeeded、指纹更新；R10/S10不变 | baseline/dirty/save等作为一个完整批次，不能只看到成功而仍留旧baseline |
| B32 | 历史预算裁剪后H0、can_undo=false；R10/S10不变 | history可用性也是被恢复域，不从正文revision猜它没变 |
| resume(C30)在head32线性化，日志31/32完整 | 新g3原子接好重放31/32与实时33起 | 从完整缓存30开始，按31→32应用，last_complete=32；不是从32直接开始 |
| resume返回途中发布B33 | V1 caret变2/ViewEpoch前进；R10/S10仍不变 | g3排在重放后交付33，最终caret2、last_complete=33，无交接空隙 |

最终缓存R10/S10、baseline=S10、dirty=false、H0、save成功终态、V1 caret2，
各字段对应同一完整前缀。不能以“revision没变”跳过31/32/33。

Rusty反例的初订阅变体：observe若在状态30、B31之前线性化，返回包含旧baseline的snapshot30
和已注册g4，B31必进入g4；若在B31之后、B32之前线性化，返回的ObserverSnapshot31本身就含
新baseline/Succeeded/dirty=false，g5从32起。两种顺序均不需要、也不允许另取最新游标。

## EX-15：半批次、重复投递与旧句柄（C08）

客户端g6已完整安装C40：R4/S4正文`a`、baseline=S4/dirty=false、H0/redo0、
can_undo=false、save=Idle/attention无异常，scope只消费V1，caret1。
B41含内容追加`x`及相关View/history/dirty子事件：目标为R5/S5=`ax`、baseline仍S4、
dirty=true、H1/can_undo=true、V1 caret2，其余域不变；后有完整批次结束标志。
这些不是可独立提交的观察状态。

| 输入 | 预期缓存 / 游标 |
|---|---|
| 只收到41的内容部分，没有BatchEnd | 暂存，不公开`ax`，仍完整C40状态；last_complete仍40 |
| 收齐41各部分及结束，验证成功 | 一次换为上述完整R5/S5、`ax`、H1、dirty=true、caret2状态，last_complete=41 |
| 同g再次投递完整41 | 整批忽略，不重复append，正文仍`ax`，不是`axx` |
| 同一待完成批次的同身份分片重传 | 去重；发现同序号内容/边界冲突则协议错误并重同步，不挑一个继续 |
| B42仅修改未被scope消费的V2选区 | scope仍接收42的空投影完整批次标记；数据不变、last_complete=42，不误报缺号 |

若随后observe得到新g7/snapshot45，旧g6即使晚到seq46（大于45）也不能应用。
必须按SubscriptionId拒绝g6，不能仅用“seq大于当前”判断可接受。
新scope若增加V2，旧scope的cursor42不能resume；必须重新observe补齐V2状态及新身份。

## EX-16：旧游标与恢复后再次Lagged（C08）

客户端断线缓存C50：R10/S10=`abc`，baseline=S9、dirty=true，O1捕获S10且Verifying，
H3、V1 caret0。服务器51–54只改变视图等状态；至head54时V1 caret2，正文revision仍10。
有界重放日志最早保留L52；本例订阅待消费队列容量为1个完整批次，仅用于推演溢出，
不是冻结产品预算或声称已实现队列。

| 步骤 | 必须发生的动作及确定结果 |
|---|---|
| resume(cursor50)，head54/L52 | 所需51已丢，返回CursorTooOld；不把cursor改为51/54继续，不因R10相同跳过View状态 |
| 重新observe，g8/snapshot54 | 一次装入完整R10/S10、baselineS9/dirtytrue、O1 Verifying、H3、caret2；last_complete=54，g8从55起 |
| 先B55保存成功，再B56 caret3；消费者尚未读55 | 容量1不足，g8报告Lagged；不能只接56或先半改baseline；本地完整游标仍54，缓存标stale |
| 丢弃g8未完成/暂存批次，再observe | g9/snapshot56一次包含baselineS10/dirtyfalse、O1完成、H3、caret3，R10仍不变；装完后last_complete=56 |
| 旧g8迟到55/56 | 按旧SubscriptionId全部忽略，不能覆盖g9状态 |
| g9收到下一完整B57：追加`!` | 一次成为R11/S11=`abc!`、baselineS10/dirtytrue、H4、caret4，last_complete=57 |

第二次Lagged不能答“已经取过快照，直接到最新”。若g8在snapshot54安装前已明确Lagged，
不将该快照装成当前同步状态，直接重新observe；若已安装则如表保留stale显示后整体替换。
再发生溢出就再次完整恢复；无法赶上时保持明确stale，不承诺必然在有限轮数追上，
也不丢已接受的正文事务来制造追上。

## 要求到示例的映射

| 固定要求 | 契约 | 示例 / 既有设计验收关联 |
|---|---|---|
| 身份、唯一正文/独立位置 | C01/C02/C05 | EX-01/11；AC-013/016/017/065 |
| UTF-8/UTF-16/grapheme/边界 | C03 | EX-06/07；F01/F09、T03 |
| EOL/provenance/未改片段 | C04 | EX-03/08/12；AC-049/050/072/077 |
| 原子事务/事件/取消 | C07/C08 | EX-06/10；AC-025/043 |
| 唯一history/跨模式Undo | C09 | EX-02；AC-013/035/036/065 |
| saved token/异步保存 | C10/C11/C12 | EX-04/10/11/12；AC-036/078/082 |
| 模式0事务/0undo/dirty不变 | C06 | EX-03/09；AC-049/065 |
| 旧任务/ABA拒绝 | C13 | EX-05/09/11；T01/T16 |
| 一致观察/订阅/恢复与同revision状态 | C02/C07/C08 | EX-13/14；R4初订阅/重连竞态 |
| 完整批次、去重、旧游标/再次溢出 | C08 | EX-15/16；R4恢复安全性 |

仅Unicode长度和混合EOL示例的算术可以用静态脚本核对；
其余行为留待契约审查、后续实现和独立测试，不用文档表格的“预期”填产品PASS。
