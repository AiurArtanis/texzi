# T07：新目标发布后的创建默认 DACL

基于 `d398dd6cbd48402f4f9611d5592a3fc25c3881f9` 的下一个本地实现增量；先交具体不变量，随后交固定源码审核。所有新行为尚未编译/运行，不表示完整 SaveAs、权限继承或安全保存已通过。

## 目的与边界

目前无目标暂存会把新 File 保护为 owner/SYSTEM-only，最终只能显式按该策略发布。直接跳过暂存保护会使取消/失败后遗留的非空 temp 暴露给目录其他读者，因此本步不改 staging，也不在 temp 名称下放宽 DACL。

增加显式 `NewFileSecurity::CreationDefault`，含义为恢复同一个 create_new File 在写 payload 前已捕获的创建默认 DACL。它可能含目录继承 ACE，但不推断任意目录或之后 ACL 变化的语义。旧 `OwnerOnly` 和 `prepare_new_owner_only` 保持独立显式能力；普通覆盖/已存在 destination 仍拒绝，不借本步缩减完整 T07 范围。

本文 descriptor/完整读回仅指现有 `SecurityDescriptor::read` 请求的 owner/group/DACL，不包含 SACL、完整性标签或 EFS；也不把前后检查当权限或命名空间 CAS。

## 具体接口与约束

- `LockedStaged::prepare_new_creation_default` 与旧 owner-only 方法共用新目标/名称/身份/格式预检；只接受原观察 absent，提前验证捕获的 creation descriptor 有可恢复的非 NULL DACL。无法表达的 descriptor 在许可前明确拒绝，不静默选另一策略。
- `NewReady` 在 permit 前固定私有 typed policy。策略随 NewPublication、PublishedNew、PublishFailure 按值移动；外部不可拼装、不能在许可后改选。结果的公开 getter 和 CommittedNew 说明实际选择。
- 原 publication registry 同步保留该同 operation 的策略意图，丢结果后的恢复回执仍能读取；`expected_security()` 复用同一冻结映射（OwnerOnly 为原 private payload，CreationDefault 为原创建 descriptor），`current_version()` 从保留原 File 实际重读。两者分开，不以枚举推断 helper 执行，不把 actual 重新定义成 expected。检查 poison 句柄不清 poison 或恢复发布资格。原 Drop/drain/ack/held 规则不变。
- 本增量不在 staging、取消、许可前错误或未成功 rename 的路径调用 DACL 恢复。对象仍被拥有时沿原规则保持私有 descriptor/独占 File；本次没有新增 close/reopen、按路径改权限、删除或重试。既存早期 staging 错误会关闭局部 File，Ready/许可前失败对象 Drop 也未纳入 registry，原句柄持续保留与 owner 收尾缺口仍未解决。
- worker 在原 File 上完成 `ReplaceIfExists=false` 的相对名称 rename，随后重读同 File 的 version，先核实际 final path、原创建 ID、原 payload/捕获 format、私有 security、parent 及历史源；这些均成功后才在原独占 File 上恢复创建 DACL。不能只依赖 rename 前的暂存读回。DACL-only helper 不写 owner/group，不要求新 WRITE_OWNER 权限，不复用仅适用于 pre-payload 的旧复制接口。
- 已成功 rename 后 DACL 恢复、flush、身份/内容/格式/安全描述符读回或 owner 接受前核查失败，沿原 Attempted→OutcomeUnknown，保留原 capture、N、File/parent 和恢复资源；不把 owner-only fallback 当成功。
- 成功必须 full descriptor 实读严格等于按 policy 选择的 expected：OwnerOnly 选原私有 payload security，CreationDefault 选创建时 descriptor；owner/group 未被主动写入，仍由该读回相等条件核查。owner accept 在创建属性 anchor 和更新 Store 前再次重检目的 parent、历史源及实际版本；所有返回式失败仍在 Store 接受前。最终 actual version/raw/path/file-ID 与同次策略进入既有结果凭据；Store 接受 N 不回滚或清除 N+1。

Unknown 不表示 ACL 回滚或保密撤回。恢复已经成功但后续 flush、验证、owner 接受失败或结果丢失时，最终名称可能已具有创建默认 DACL；API 错误本身也不能证明权限绝对未改变。本步不自动再私有化、删除文件、回退 OwnerOnly 成功或解除 held。registry ack 只释放其资源份额；原 File 的最后持有者结束后，文件按实际 DACL 暴露，须实际观察并另行恢复。

策略通过现有不可外部拼装的结果所有权链绑定同次 capture/permit，暂不新增 Store 公共 setter 或改变一般内存 save 协议。若此证明链存在具体可绕过入口，须在固定源码审核中退回，不以文档代替。

## 回归源码计划

仅新增隔离生成文件的 Windows lib unit 源码，不实际运行或修改用户 ACL：创建默认策略正常新目标、实际 descriptor/原句柄/N+1/最终 binding；准备失败或已有 target 不得到覆盖能力；rename 前目标竞争不覆盖且 temp 仍为私有；rename 后 DACL helper 的受控错误走真实 Unknown 路径、保持 N/N+1/held。测试只用该原创建句柄和同目录生成对照，不修改目录或全局权限。

接口补充回归另列在 creation_security/contracts、unsupported_creation 和 windows_security/creation_policy_tests：两策略正控制、后置验证失败、两策略丢 worker/result 的冻结 expected/actual、恢复前 payload 故障及竞争；absent、NULL、present 空 ACL 分别验证。三种 descriptor 只在测试 owned buffer 中构造，经实际验证 API 检查；真实 prepare 拒绝路径使用私有 cfg(test) credential override，不把该注入称为 OS 创建了 NULL/空 ACL 文件。空 ACL 只验证表示/准备，不执行空 ACL 发布；不新增生产 setter 或 probe 开关。

另补 CreationDefault 恢复成功后的 native flush 拒绝用例：后置私有 test hook 先核原 File 的最终路径、ID、N bytes 及真实 creation descriptor，再返回 READ_CONTROL-only DuplicateHandle，由生产路径的真实 `sync_all` 产生预期 OS error 5，不预制 Err、不注入 format 错误。断言 Unknown/N/N+1/baseline/history/epoch、保留原资源与 held，重复 record 及 Drop/drain 不新增终态，actual 仍为 creation DACL。此用例与 setter 未成功的拒绝及恢复后 format 校验失败分列；只交源码，预期错误 5 和共享行为仍未运行证实。

后续真实 harness 要与 contracts 分列；当前所有新用例 NOT_RUN。SetSecurityInfo 对 inherited ACE 的重新继承/规范化与 full descriptor 相等能否满足、权限/共享及崩溃边界尚无运行证明，严格失败谓词不等于功能可用。SACL/标签/EFS/多主体及完整权限策略未闭；本步不改 CI/GH/linker/push/GUI/剪贴板/远端/权限/发布限制。

## 既存 Rust lint 边界修正

本轮源码检查发现根 `forbid(unsafe_code)` 会同时禁止三个已经存在的 Windows FFI 适配模块。这是确定的 lint 配置冲突，尚未运行编译，不等价已恢复可构建。

改为 crate 默认 `deny(unsafe_code)`，十个纯文档模块以及 staging 各显式 `forbid(unsafe_code)`；只有 `cfg(windows)` 的 windows_identity、windows_save、windows_security 三个既有适配根使用局部 `allow(unsafe_code)`。各调用仍须保留安全性注释，新增例外须单独代码审查；静态清单审计限定这三个入口。本变更不改宿主权限、审批、模型或 OS 策略，不放宽正文/坐标/事务/history/观察/视图内核的禁止 unsafe 边界。
