# T07：确认失败记录时的事件计数边界

依据原接口审阅 `evidence/TX-07-rusty-pre-replace-ack-interface-274039f.md`，`d15a5db9d05ddc3ccacb6f350309865325fd5bf9` 的 owner 确认路径已经区分首次与重复，并覆盖后来 Busy/Unknown 拒绝、旧失败在新成功保存点之后确认等公共内存场景。本增量只补事件计数耗尽的两个 lib unit fixture，不复制这些已存在源码用例，不改生产确认接口或通用观察协议。

`observe/failure_ack_tests.rs` 作为 observe 的私有测试子模块，仅在 `cfg(test)` 下直接设置隔离 Store 的 journal head。没有新的公共 setter、生产计数注入、全局静态修改或真实 IO。注入 head 的行为不代表实际生成了接近 2^64 个事件，也不证明完整续传矩阵。

首次确认仍须经过 `observed_write` 的 `EventSeq + 2` 预检；在 `u64::MAX - 1` 和 `u64::MAX` 处应返回 `CounterExhausted`，原 flag、文档/恢复快照、保存点/history、cursor、journal 和订阅队列不发生部分修改。

合法已确认的重复请求先核原记录/快照和当前安全资格，再从只读路径返回 `newly_acknowledged = false`。同样两处计数边界不得进入通用写入预检，不增加事件、改变水位或替换原 N。仍不允许绕过后来 active save 或 Unknown/冲突；这些精确场景沿用 d15a 的六个公共内存 fixture。

两个新 fixture 属于 **lib unit harness**，不能用 `contracts.exe` 证明已执行。后续恢复合法可构建环境时，需单列 `cargo test -p texzi-document --lib --locked --target x86_64-pc-windows-msvc observe::failure_ack_tests` 或对应取得的真实 lib harness；该命令本轮 **未执行**，不是本轮新增构建或运行要求。

本轮仅格式、差异与固定源码字节/命名核对。类型检查、两个 lib unit、旧六个内存用例、probe、真实应用/磁盘 IO 及人工验收仍 **NOT_RUN**。不重跑 CURRENT、旧窗口或其它已闭路径，不改变既有 CI/GH/linker/push/GUI/剪贴板/远端/权限限制。完整安全保存、Unknown 核实与恢复解锁、strict GB、完整 SaveAs/UI 仍按原 T07 范围未完成。
