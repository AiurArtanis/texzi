# T07：已有目标的只读句柄捕获

本步为已有目标安全保存提供 read-only 中间组件，仍停在 WritingTemp。类型检查、测试及实际 OS/IO/ACL/GUI 均 NOT_RUN；不是安全替换、victim 备份或完整 T07 接受。

## 接口与所有权

`LockedStaged::begin_existing_read(self, &Store)` 只消费已有 bound/locked 对象，不收外部新 path、expected version 或任意 File。仅做内存预检：原完整 capture 在唯一 Store 中仍活动且阶段 WritingTemp；原观察 absent 返回 NoExistingTarget。返回自有资源的 ExistingReadWork，不将 Store 引用持有到 IO。`run(self)` 不借 Store，进行磁盘读取并返回 ExistingReadCandidate；`candidate.accept(self, &Store)` 只再做完整 owner 内存核验，合法才交付 ExistingReadCapture，不写保存回执。两类失败均按值交回完整 LockedStaged，保原 temp File、capture N、parent/source witnesses；局部新 read File 未通过捕获或 owner 接收时关闭，不把它制造为新磁盘凭据。

目标从同 ObservedTarget 的原版本/路径/parent 派生，read(true)、FILE_SHARE_READ、FILE_FLAG_OPEN_REPARSE_POINT 打开；check_kind 拒绝不支持末级对象。原句柄完整版本、实际 volume path 与 parent 前后复核均成功才返回同一个 File。没有解码、attach、观察结果刷新原基线或宽共享 fallback。worker 在 IO 前后查原 staged/source/parent，owner 在开始和接收时验证；所有返回式错误仍保原 staging 资源。接收只核 owner，不声称 IO 后的 ACL 等元数据再未改变。

成功类型 ExistingReadCapture 私有拥有目标 Mutex<File>、原 LockedStaged、冻结的目标旧 FileVersion/path。expected_target_version 是原目标版本；payload/read_staged_bytes 是待保存 N，二者不得混同。current_target_version/current_target_path 从所持目标原 File 实读，单个 getter 不表示 owner 仍有效。阻塞诊断用 recheck(&Store) 在读取前后核完整 owner，核 staged/source/parent 和目标实际完整版本/路径；不改 Store，不推进 stage 或发布事件，不接 UI 关键路径。

`into_staged(self)` 显式释放目标 read File，返回原 staging；此后该 read 捕获已失效，任何后续动作须重核。Drop 同样只结束本对象持有，不取消保存、删除 temp、更新 baseline 或释放逻辑 held。must_use 只提醒调用方，不构成遗失/崩溃资源保证。资源若全部遗失，原许可前 registry 缺口仍在。

## 共享及保护范围

目标 read File 的共享仅允许普通 reader；不允许普通 write/delete 打开，也须拒绝与现存普通 writer 冲突的打开。不得失败后改用更宽共享或按路径“最后再试”。共享、错误码及控制组是否实际成立仍需合法 Windows harness，当前没有执行证据。

这不是 ACL 冻结、mapped writer 排除、文件系统 CAS 或全局目录规范器。保留实际完整 version/parent/source 检查，无法证明时明确失败，不用新观察重定义 expected，不更新旧 binding。原 Unicode 路径保守拒绝及其它 preflight 限制没有放宽。

两个新 private child module 均 forbid unsafe_code；复用原 Windows identity 适配器的安全封装，不新增 unsafe 或 allow 入口。原699静态清单继续只绑定699，不能冒充当前源码清单。

## 明确尚未提供的能力

ExistingReadCapture 没有 NewReady/NewPublication 转换，不给 Ready/permit/accept，不形成最终 FileBinding。旧 new-only 发布对已有目标仍 ExistingTargetUnsupported；显式返回 staging 也不会改变这一点。N+1 编辑允许继续，捕获/检查不会清 dirty、移动保存点或改 history。

真正 existing 替换必须先具体审核 native 访问位/共享位及句柄交接、实际被替换对象/竞态外部版本的保全、专用 owner existing permit/最终实际ID/epoch、registry 与失败/Unknown资源。当前 read 锁和原 temp FILE_SHARE_NONE 不能直接套路径型 ReplaceFileW；不能关句柄后按路径覆盖、放宽共享或以旧 observation 的 bytes 代替真实 victim 保全。

完整SaveAs/覆盖/strict GB/恢复/UI、性能及硬预算仍待完成。无构建/typecheck/linker/GH/CI/push/GUI/剪贴板/远端/权限发布或新独测派发；历史失败及原接受保留。
