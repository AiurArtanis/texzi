# T07 保存失败终态及未决闩锁

沿原Rusty已审接口条件继续。新增finish_save_failure经原observed_write发布；校验活动capture的operation/doc/target/epoch/guard/revision/token与同一Body Arc，拒绝错租约、错阶段和重复回执，不能释放另一预约。

失败记录包含实际阶段、OS错误、消息、原生PathBuf恢复路径及捕获revision/token；捕获Snapshot独立保留供恢复读取。原始N+1、baseline/history不变，不发Succeeded；单次失败一次完整发布，新的observe快照含恢复记录。probe仅在存在恢复记录时追加字段，Windows路径另有UTF16 code units，不用有损display作唯一恢复路径。

结果未知、替换后失败或已知冲突时，释放活动预约的同时保留doc闩锁及全局逻辑TargetKey保护。下一次同doc或同逻辑目标保存返回UnresolvedConflict，不自动重试。没有清闩锁捷径；真实身份协调与核实结果的解除/归档回收API尚未实现。存在恢复记录时不自动移除文档，防止删除唯一内存恢复快照。

这是内存协议源码，不是Windows磁盘故障实测。恢复归档尚无内存硬预算/重启持久化证明，TargetKey仍需真实FileID/epoch解析，不能声称路径别名已经统一。文件身份及SaveAs/ACL协调后，才接真实ReplaceFileW/无覆盖发布、核验和binding原子刷新。当前temporary staging止于WritingTemp，不发commit permit。

新增2个Rust用例覆盖各阶段失败、N+1不变、捕获恢复、一次发布/完整快照、重复/错operation/epoch/target/body和阶段拒绝、释放旧预约不放开Unknown目标。运行NOT_RUN；fmt/metadata不替代Rust编译或实际IO。
