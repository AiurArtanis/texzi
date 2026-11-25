# T07：明确返回错误时的原资源持有

本步仅实现 returned-error 所有权，不是许可前 Drop registry、崩溃恢复或完整安全保存。类型检查、测试和实际 IO/ACL/GUI 均 NOT_RUN；完整 T07 仍 PARTIAL。

## 原问题与新边界

此前 `StageFailure` 和 `BoundFailure` 只带候选 temp 路径。创建后出错会关闭局部原 File；staging 成功后的外围 recheck 出错还会释放原 parent/source witnesses。路径仍存在并不能证明调用方保有同一对象。这是源码所有权缺口，不是已实测泄露、数据损坏或绕过 permit。

现在 `create_new` 返回原 File 后，立即在任何 fallible hook、identity、security 或 payload 操作之前包装为私有 `Arc<CreatedTemp>`。成功的 `StagedSave` 和错误的 `TemporaryRecovery` 持该同一对象；没有复制或重开替换原 File。成功只释放 Mutex guard，原 File 继续贯穿既有 staged/locked/permit/run 链，FILE_SHARE_NONE 不变。

创建前错误没有 temp 回执。创建后的每个显式返回错误都持有 TemporaryRecovery，包含原完整 capture、N 的预期 encoded payload、temp/target 提示和原 File。预期 payload 不是 actual 写入证据；实际文件可能为空、部分写入或完整 N。未能读取 identity/security 也能保有原 File，不能因此制造一个已验证的 ID。

## 只读接口与兼容差异

- `StageFailure::recovery()` 返回借用的私有构造回执；可 Clone 显式延长其原资源寿命。capture/expected_bytes 和 temp_hint/target_hint 说明意图及路径提示；路径不是 capability，不按该路径重新打开作为恢复证明。
- read_created_bytes/current_path/current_version 从保留原 File 实际读取；poison 只允许检查，不清 poison、不恢复 worker 权限。Getter 不接收 Store、不写终态、不推进 Ready、不改 baseline/history/epoch/N+1，也不删除文件、放宽 ACL 或授权发布。
- `BoundFailure::recovery()` 还保留同次 format、原 ObservedTarget（含 parent File）、HistoricalSource 和 source_anchor。begin 的错误、write 进入前错误、内层 staging 错误、staging 成功后的 target/source 重检及 temp identity/path/security/bytes 失败均沿该 context 返回。源 anchor 只是对象身份存续，不能当永久内容锁。
- BoundRecovery 的 target_path/parent_identity 是原观察信息，recheck_parent 是实际只读复核；它不把新观察写回基线、不解除 held，也不证明原历史源路径仍指同对象。
- 新增私有 recovery 字段限制外部直接构造这两种错误结构；既有公开 issue/capture/path 仍作诊断，不能拼接或更改私有资源的真实归属。外部结构匹配须使用 `..`，仓库现有穷尽匹配已同步。

一个有意的行为变化：错误对象或其 cloned receipt 活着时，temp 仍保持 FILE_SHARE_NONE；路径重开 reader 应被拒绝。旧安全 fixture 在错误后按路径 read 内容的断言依赖原 File 已关闭，本次改为回执原句柄读取，并明确检查外部 reader 被拒；属性 metadata 查询不作为内容读取或独占的证明。最后一个持有者释放后才结束原句柄保护；不把释放等价资源清理、原文恢复或权限回滚。

## 取消、预约及终态

worker 的 write 不借用 Store。取消后旧 worker 仍可能创建或写入它自己的 temp；此回执始终只带旧 operation/capture，不查询当前保存来重认领资源。回执构造、读取、Clone/Drop 不调用 finish_save_failure/cancel 或发布观察事件。调用方若用旧 capture 结束新预约，仍由原完整 capture 校验拒绝。

Store 的 pending/active/scope/baseline/N+1 不因资源回执变化。与已有 publication registry 严格分开：本步没有自动注册、drain 或 ack；不是 Unknown 核实/解除或当前 dirty 丢弃许可。

## 尚未覆盖

只保证显式返回 Err 对象所持资源。worker 普通遗失、panic/unwind 在错误形成前、mem::forget、abort、所有回执被丢弃、Store/进程终止和预算仍没有新增存续保证；没有后台恢复或磁盘日志。最初 create_new 成功后 Arc 分配失败也无 OOM 保证。Getters 的实际 OS 共享/权限/错误码仍待固定合法 harness。

已有目标替换仍未启用。路径型 ReplaceFileW 与原 exclusive temp/目标锁不直接兼容，后续须单列真实共享交接、实际 victim/外部版本保全、existing owner permit/最终ID/epoch 与恢复资源接口审核，不能通过关闭重开、放宽权限或把检查称 CAS 绕过。严格GB、完整SaveAs/恢复/UI要求及全部旧失败保留。
