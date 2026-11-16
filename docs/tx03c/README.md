# T03c / C08–R4 内存观察协议

基于已接受7aaa6b7的C08.1–3/EX13–16，在T03b之上提供可运行的内存发布/接收组件。
这不是T03/G1完成或T04/T05/NSH/IO放行；原5a/da检查点及历史失败保留。
新probe操作见[PROTOCOL.md](PROTOCOL.md)，原JSON-lines v1操作继续可用。

## 单一发布点与状态域

公开正文/视图/保存/历史写入口统一走`observed_write`；内部嵌套（如保存捕获中的fence）
合并为一次发布，先验证事务和视图，返回前发布同点状态、完整批次和EventSeq。
单写者独占借用内没有回调重入或异步写者；`read_document`仍不是恢复凭据。
EventSeq每Doc从0起，只随可观察状态发布增长，与正文revision独立；NoOp无状态变化时不增长。
正文提交包含PublicationId（StoreEpoch/DocId/EventSeq）及稳定HistoryGroupId、正逆delta、
新全局状态和scope内View映射。纯save/history/View同样可发布，正文revision可以不变。

`observe_and_subscribe(doc,scope,capacity)`在一个独占调用中捕获所有域于W，
注册新SubscriptionId、从W+1接流，再成对返回ObserverSnapshot/SubscriptionHandle。
初订阅前后的内容提交各自落入快照或后续流；没有“读取内容后再取最新游标”的API。

ObserverScope=`DocGlobal + 显式已存在/已关闭ViewIds + schema1`，ViewIds排序去重后成为身份。
原始非规范scope拒绝；增加View或改变schema必须重新observe。Snapshot必含：

| 域 | 当前返回值 |
|---|---|
| 正文 | 不可变Snapshot根：DocId/Revision/StateToken、UTF8 Rope、EOL/profile/BOM/provenance |
| baseline | token/kind/backed_by_disk和dirty；支持InitialEmpty、测试用MemoryImport、ExternalLoadClaim、SavedByCaller |
| 文件/attention | needs_target、epoch0、当前内存TargetKey、未提供fingerprint(None)、conflict/outcome_unknown明确NotImplemented、当前显示错误 |
| save | 当前op/捕获revision-token/target/guard/阶段/cancellable，以及已发生的SucceededByCaller/Cancelled终态 |
| history | undo/redo可用性、组数/最近组身份、当前合并边界及last_fence |
| View | 完整已实现逻辑ModeTag、SelectionSet/primary、scroll/affinity/bias、epoch/snaps；或Closed墓碑 |
| 其余View域 | folds/focus/composition/pending_mode明确NotImplemented，没有伪造空overlay或系统焦点 |
| 观察身份 | StoreEpoch、DocId、scope/schema、完整批次水位W和SubscriptionId |

MemoryImport/ExternalLoadClaim是此前组件的生成fixture/可信调用方入口，不是新文件产品策略或IO证明。
SavedByCaller仍依赖可信IO回执；probe只模拟该边界。未知冲突等不是false/“无错误”。
schema1显式冻结当前可观察域及NotImplemented标记；将来实现这些可变域时必须升级schema/
经契约审查，不能偷偷复用旧decoder忽略新状态。不对当前没有API可改变的域虚构事件。

StoreEpoch包含进程随机化nonce（标准库RandomState混合进程ID/启动时刻）和进程内不复用序号；
同进程重建改变序号，新进程重启改变nonce，旧cursor拒绝。随机nonce有通常的有限碰撞风险，
不是磁盘持久化计数器；不写用户文件。身份溢出停止创建/提交，不绕回。数字故障注入仍待后续。

## 完整批次、scope投影和容量

Record在发布点保存Global/Content/各View变化；ObserverSnapshot及事件共享不可变根，
默认库不逐键序列化或全文hash。probe的JSON正文/来源物化仅为显式验证出口，不是UI适配路径。
只变更scope外View且global不变时，仍发同水位的**空事件批次**，不跳号。
若该View动作切了全局history合并边界，全局域确有变化，不能假装空批次。

工程初值：每Doc完整日志64批次；每订阅默认调用方选择1–64队列批次；Store最多1024个订阅。
可在开Doc前设置日志0–4096、queue最大1–4096、订阅最大1–4096；每scope最多1024个View。
这些是批次数/入口保护，不是bytes/Private Bytes硬上限或正式性能达标。
大delta/root、tombstone和外部快照仍可持有额外内存，硬内存/回收仍NOT_RUN。

订阅队列装满后下一次发布终止该流、清队列并标Lagged；正文提交仍成功。
poll返回包含注册水位、当前L/H及capacity的Lagged，释放该订阅；下一次poll为StaleSubscription。
poll/pop只是交付，不当作客户端已安装的ack；诊断L/H不替代客户端last_complete_cursor。
主动unsubscribe释放资源。未poll/unsubscribe的旧订阅仍计上限，达到上限明确拒绝，不能假同步。
逻辑Doc关闭拒绝active save，销毁内存owner并令订阅得到带身份DocGone；不是UI关闭授权/磁盘流程。

## 连续resume

`resume(doc,scope,cached_complete_cursor,capacity)`先检查Store/Doc/scope/schema，
再验证C在完整批次范围、C+1..H连续且全部保留；一次注册新SubscriptionId，将重放接到H+1实时流。
L-1<=C<=H只是必要条件，还验证完整连续区间。C>H为CursorAhead；C<L-1为CursorTooOld。
日志为空取L=H+1：C=H允许空重放，C<H不允许。重放批次数大于新队列容量直接返回Lagged，
不截断、不分配成功句柄。失败不改正文或缓存；scope/schema改变须observe而非跳最新。

调用方必须已有与C匹配的**完整状态**，不是只拿cursor。若外部持久化，状态和cursor须同次落盘；
本步未实现真实持久化。内置cache只有installed/非stale/无半批次时能取resume cursor。
新句柄必须显式rebind；旧句柄晚到数据即便序号更大也忽略。Lagged后的cache不允许resume，必须observe。

## 分片接收缓存

EventBatch可展开为Begin(parts)、Part(index,event)、End(parts)。每片含完整cursor身份和SubscriptionId。
`ObserverCache::prepare`持有待安装Snapshot；安装前到达帧只缓冲，state仍None；收到Lagged即stale且不安装。
一次install替换全部域及W，再按序处理缓冲；本地缓冲可设1–65536帧，满时显式BufferOverflow/stale。

安装后只接W+1批次：所有Part暂存，直到End验证完整计数、前置revision、Content/Global/View一致性、
scope与View epoch后，一次替换全部状态和cursor。Global使用共享不可变根；delta供增量消费/独立oracle，
缓存不先对旧正文apply一半内容。未知schema/事件、缺片、缺批次、冲突片或错误身份均标stale、丢半批次。
重复同片去重；已完成水位及更旧批次不再应用，最近完成批次仍核已知帧冲突。
快照覆盖/已不保留的更早批次仅丢弃，不从重复载荷恢复数据或认证它的内容。

StoreEpoch/Subscription注册身份先于序号检查；非当前句柄忽略。同句柄错误Doc/scope/schema不能被旧序号掩盖。
第二次Lagged重复完整observe恢复，不盲跳头部。缓存“同步至W”不承诺等于尚未知的服务器实时头部。
实际绘制/present、真实IO和系统IME不在此发布保证中；自定义客户端需遵守相同协议。

## 验证边界

新增EX13–16交错/完整批次/去重/空投影/身份/resume/两次Lagged、关闭墓碑/DocGone等组件测试；
新增外部probe驱动按独立round保留原始字节stdout、stderr、解析trace、进程exit及exe hash。
实现者提供的驱动不是Steamy独立编写测试，原Steamy可替换客户端/构造自己的帧和oracle。
旧suite/外部byte-string oracle继续回归；未测试不补成PASS。

初始本机测试代码所有权写法E0382和probe辅助模块被Cargo当独立bin的检查失败已修复；
默认库check0，启用serde probe依旧依靠现有CI编译，不重试本机已知缺linker路径。
最终候选必须绑定新commit/run/artifact/hash，下载包再复验；不把构建成功作GUI/IO验收。
