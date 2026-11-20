# T07：发布对象丢弃后的本进程资源责任

本增量只接 Windows owner/SYSTEM-only 的不存在目标发布组件；不接应用 UI，不等价普通覆盖或完整 SaveAs。

## 所有权与顺序

`NewReady::permit` 先以完整 capture 检查 WritingTemp，随后把同一 `LockedStaged` 放入 Store 的私有 registry，再申请原有 scoped permit。permit 返回错误时撤销注册，仍返回原失败对象；没有进入 native rename。获得许可后，Guard 随 `NewPublication → PublishedNew/PublishFailure` 移动。Registry 持有同一原 File/Mutex、原目录句柄、捕获快照、payload/格式及身份观察，不按路径重开、复制或删除文件。

正常 `PublishedNew::accept` 通过原 owner 校验、最终实际 file-ID/binding/epoch/保存点迁移后解除 registry 持有；N+1 的正文、history 与 dirty 仍由既有 Store 协议决定。显式 `PublishFailure::record_failure` 仅在原终态成功写入后标记 recorded。

Guard Drop 只标记 dropped，不做 IO、保存回执或 Store 回调。若未接受结果就 Drop 或 Rust unwind，原资源仍由 Store 保留。`drain_dropped_publications` 必须由单一 owner 显式调用：完整 capture 仍有效时，许可前记 BeforeReplace；进入 Replacing/Verifying 后保守记 OutcomeUnknown 并保留冲突 scope。不会从文件存在或字节相等推断保存成功。owner 校验/事件发布失败则返回 owner_error，保留资源供后续处理，不释放或覆盖其它预约。

已 recorded 的条目重复 drain 只返回句柄回执，不重复写终态事件。`retained_publication_recoveries` 可重新取得遗失的回执。返回回执自身也持有同一资源。`acknowledge_publication_recovery(operation)` 只允许 dropped 且 recorded 的本 Store 条目，释放 registry 的那一份资源；其它回执存在时句柄继续持有。该操作不删文件、不清 Unknown、不解锁逻辑 scope、不改 baseline/binding/epoch，也不是恢复完成或可继续覆盖的声明。

worker unwind 可能 poison 原 File Mutex。恢复读取可从该 Mutex 取得原句柄做只读检查，但不清 poison，不恢复发布能力，不能把读成功当作验证成功。

## 源码 fixture 与边界

新增三个 Windows lib unit（尚未执行）：

- 丢弃已许可未运行的 job；N+1 保持、Unknown 单事件、回执遗失后仍可发现、ack 前保原句柄且 ack 后冲突 scope 仍存在。
- 真实 rename/flush/核验成功后丢弃 PublishedNew，不 accept；目标仍是捕获 N，原句柄继续排除另开读取，owner 不误清 N+1，丢回执/重复 drain/显式 ack 的责任同上。
- `panic=unwind` 时在真实 rename 后测试 hook 展开；原句柄保持，poison 后只读恢复、Unknown 终态。不是进程崩溃或断电测试。

原两个 post-rename 失败 fixture 增加已记录失败对象 Drop 后的 registry 收尾，原失败谓词/字节/history/外部 scope 断言保留。

所有本增量运行都是 NOT_RUN；fmt/diff 不证明 Rust 类型正确或 Windows 共享行为。仍缺许可前普通对象遗失、mem::forget/abort、Store/进程终止的持久日志、恢复 UI/核实解锁和资源压力预算。registry 仅活于 Store 生命周期，没有后台自动 drain。完整覆盖、外部版保全、严格 GB 标准实现和完整 T07 不因此完成。历史失败记录不回写。
