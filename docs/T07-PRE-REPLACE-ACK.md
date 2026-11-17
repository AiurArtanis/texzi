# T07：替换前非冲突失败的显式处理

状态：本地源码增量，基于 `274039f287b92219ef09862f5bce3f92a537537e`。具体接口和不变量已先交原审核链；本文随实现交固定源码继续审核，不表示类型检查、运行或功能接受通过。

## 要解决的生命周期缺口

当前 `finish_save_failure` 对 `BeforeReplace && !conflict` 保留失败记录和捕获 N，释放该操作预约，允许重试。`close_document` 却因任何失败记录而永久拒绝，后续成功也不会移除旧记录。需要由单一 owner 显式承接这条已结束失败的恢复责任；禁止以自动清记录、自动关闭或伪造保存成功解决。

本步不处理 `Unknown` 的核实或解除冲突，不清理磁盘文件，不接 UI，不宣称完整安全保存或恢复完成。

## 拟议接口和状态

新增 `Store::acknowledge_pre_replace_failure(doc, operation) -> Result<PreReplaceAcknowledgement, Error>`。这是 owner 的显式意图，不是 worker 异步结果或磁盘成功凭据；不得由通用错误处理自动调用。

`SaveFailureRecord` 追加 `acknowledged_before_replace: bool`，初始为 `false`。返回的 `PreReplaceAcknowledgement` 包含：

- 完整失败记录（只把该标记置为 `true`）；
- 该操作原捕获 N 的 `Snapshot`，保持相同 token、Body Arc 和格式；
- `newly_acknowledged: bool`，首次 `true`，重复确认 `false`。

返回拥有的记录和快照，使调用方在后续显式关闭文档之前能保留恢复责任。该返回值不拥有原生 File、parent handle 或 identity witness，也不转移或延续原生资源的持有责任。`must_use` 只能提示误丢弃，不能替代实际 UI 的保留或明确丢弃决策。Store 仍保留原记录及快照直到正常关闭；不从临时文件存在或字节相等推断恢复完成。

入口须在任何修改前检查：

1. doc 存在，operation 属于该 doc 的终态失败且有对应恢复快照；否则 `InvalidDoc` 或 `InvalidSave`。
2. outcome 恰为 `BeforeReplace`、`conflict == false`，记录阶段为 `Encoding / WritingTemp / ReadyToReplace`，doc 没有未决冲突闩锁；否则 `UnresolvedConflict`。
3. 同 doc 没有新的活动保存；否则 `Busy`。新的预约或其它 doc 的保存不被取消或释放。
4. 首次确认才进入现有 `observed_write` 预检事件计数能力，闭包再次检查同一资格；计数不足不发生部分修改。重复确认仍先通过上述资格检查，随后只读返回已拥有的数据，不消耗事件水位，也不因首次确认所需的计数额度耗尽而丢失幂等读取能力。

首次仅修改该记录的标记，并通过现有观察协议发布一个 Global metadata 批次；revision、StateToken、正文、EOL/provenance、baseline、backed、dirty、binding/epoch、唯一 history、视图、save terminal 和所有预约/held scope 均不变。不生成 Content 或 View 事件。重复确认返回相同恢复数据，不增加事件水位。

`close_document` 继续先拒绝活动保存；存在 `save_blocked` 或任一未确认失败时仍 `UnresolvedConflict`。确认入口与关闭聚合检查共用资格谓词，必须所有记录仍符合 BeforeReplace、nonconflict、许可前阶段且均已确认，才允许正常移除内存文档；true 标记本身不是任意失败的关闭凭据。这不自动确认当前 dirty 内容，也不代替既有关闭 UI 的保存/丢弃选择；本步没有 UI 自动调用或自动关闭。

与 `acknowledge_publication_recovery(operation)` 严格分开：后者只释放 registry 的那一份原生资源，不能确认失败记录；本接口不调用后者，也不解除任何逻辑 target/parent/object 保护。`ReplaceFailed / OutcomeUnknown / conflict=true` 一律不能走本入口。保留旧失败事件、诊断、恢复路径和历史证据。

## 最小源码回归计划

全部仅内存协议，不写用户或 fixture 文件；未执行时逐项 `NOT_RUN`。

- 覆盖三个许可前阶段：捕获 N 后编辑 N+1，记非冲突失败，关闭先拒绝；显式确认只发布一次同 revision Global 批次，返回 N，所有内容/保存点/history/视图/租约不变；重复确认零事件，持有返回快照后关闭仍能读 N。
- 未知 doc/operation、另一 doc 的 operation、缺少恢复数据（若仅私有破坏才能构造则不冒充公共可达）、新活动保存均拒绝且无状态/水位变化；新的保存仍可正常取消。
- `BeforeReplace+conflict`、`ReplaceFailed`、`OutcomeUnknown` 各拒绝；doc/同 target/同 parent 或 object 的 held 保护保留，其它独立 operation 不受影响。
- 非冲突失败后重试成功，不自动确认旧记录；捕获 N 成功不得覆盖或清除 N+1，随后显式确认旧记录不回滚新的 baseline/binding/epoch。
- 两条失败只确认一条仍不能关闭；同一 doc 后续出现未决冲突时不能借旧确认解锁。

probe 扩展只暴露相同内存 owner 入口与恢复标记；不得把 probe 回执当真实文件保存、ACL、共享行为或崩溃恢复证据。

## 组件入口（尚未执行）

沿用 `document-probe` 的 JSON-lines 协议 1，新增命令为增量扩展；旧候选不支持这些命令。`FileAttention.recoveries` 保留原字段、顺序和原路径编码，追加 `acknowledged_before_replace`，由 snapshot、batch/frame 和产品 cache 共用同一序列化函数。没有修改 `OBSERVER_SCHEMA` 或旧字段含义；本步未证明跨二进制版本兼容。

| 命令 | 输入 | 实际 owner 入口 |
| --- | --- | --- |
| `save_advance_simulated` | `doc / save / stage`，stage 为 WritingTemp、ReadyToReplace、Replacing、Verifying | 原 `advance_save` 或 `commit_permit`，一次请求只前进一步 |
| `save_fail_simulated` | `doc / save / outcome / conflict`，outcome 为三种原失败结果 | 完整解析后调用原 `finish_save_failure`，生成明确的模拟诊断，无真实恢复路径 |
| `save_ack_pre_replace` | `doc / operation` | 新 `acknowledge_pre_replace_failure`，返回原 N 及完整记录，不隐藏拒绝错误 |

前两者的成功响应附 `simulated_save: true` 和内存 scope。确认响应中的 `acknowledgement` 包含 failure、snapshot、newly_acknowledged 和 scope；不把它列为磁盘成功。stage/outcome/阶段不匹配、跨 doc alias 或缺少必需参数不会先推进阶段。`obs_close_document` 和观察批次仍走原入口。

下面只说明一个待执行的最小序列，不是运行回执；N/N+1 和拒绝的更完整断言见新增 Rust 内存 fixture：

```json
{"op":"open","doc":"d","text":"A"}
{"op":"save_begin","doc":"d","save":"s","target":"memory-only"}
{"op":"save_fail_simulated","doc":"d","save":"s","outcome":"BeforeReplace","conflict":false}
{"op":"obs_observe","doc":"d","views":[],"snapshot":"before","handle":"h","capacity":8}
```

从 `before` 的 recoveries 读取实际 operation，不能跨候选假设其值。然后发送 `save_ack_pre_replace`：首次应返回原 N、true 标记，并使 `obs_poll` 取得同 revision 的 Global-only 批；重复确认应 false，下一次 poll Pending。关闭后 poll 可直接 DocGone，不保证消费者一定先看到排队的确认批。所有这些运行状态目前均 `NOT_RUN`。

## 保留缺口

真实覆盖、安全替换与外部版恢复、完整 SaveAs 权限策略、严格 `Gbk936 / Gb18030_2022`、恢复 UI、许可前对象遗失/abort/崩溃、资源预算及 SEC01-B 仍按原范围待办。本步不解除构建/CI/GH/push/GUI/剪贴板/远端等限制，不重跑旧已闭反例。
