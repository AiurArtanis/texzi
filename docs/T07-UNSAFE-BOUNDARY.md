# T07 unsafe 模块边界静态清单

产品源码固定 `69967fef327bcabb2363e144847fafde9521e4b5`；本次只增加清单/工具，不改 Rust、Cargo target/feature 或编译 flags。
类型检查、build/smoke、全部组件/IO/ACL/应用运行均 **NOT_RUN**。词法和模块布局枚举不等于 Rust 编译器语义或内存安全证明。

共 46 个 Rust 文件、47 个 crate/module 归属，库树 31 文件；实际 unsafe 关键字 49 处，库中 Windows 非 test 24、test 25。

## lint 和例外

根 deny；十个纯内核声明和 staging 为 forbid。三个 cfg(windows) allow 覆盖整个子树及嵌套测试，不是三个函数。files 其它代码继续受 deny；allow 只表示 lint 许可，每块 SAFETY 不变量仍须审查。

| 声明位置 | 属性 |
| --- | --- |
| `crates/texzi-document/src/files.rs:4` | `#[forbid(unsafe_code)]` |
| `crates/texzi-document/src/files.rs:8` | `#[allow(unsafe_code)]` |
| `crates/texzi-document/src/files.rs:11` | `#[allow(unsafe_code)]` |
| `crates/texzi-document/src/files.rs:14` | `#[allow(unsafe_code)]` |
| `crates/texzi-document/src/lib.rs:4` | `#![deny(unsafe_code)]` |
| `crates/texzi-document/src/lib.rs:6` | `#[forbid(unsafe_code)]` |
| `crates/texzi-document/src/lib.rs:8` | `#[forbid(unsafe_code)]` |
| `crates/texzi-document/src/lib.rs:10` | `#[forbid(unsafe_code)]` |
| `crates/texzi-document/src/lib.rs:13` | `#[forbid(unsafe_code)]` |
| `crates/texzi-document/src/lib.rs:15` | `#[forbid(unsafe_code)]` |
| `crates/texzi-document/src/lib.rs:17` | `#[forbid(unsafe_code)]` |
| `crates/texzi-document/src/lib.rs:19` | `#[forbid(unsafe_code)]` |
| `crates/texzi-document/src/lib.rs:21` | `#[forbid(unsafe_code)]` |
| `crates/texzi-document/src/lib.rs:23` | `#[forbid(unsafe_code)]` |
| `crates/texzi-document/src/lib.rs:25` | `#[forbid(unsafe_code)]` |

三个例外的源码共享原因在 files.rs 注释；没有伪造 reason 属性。具体原因：windows_identity 封装活句柄/缓冲区的 FileId/GUID 查询；windows_save 封装原句柄 rename 及隔离 fixture 的 DuplicateHandle；windows_security 封装 descriptor/SID/DACL 的原生内存和 handle API。

## 全部实际 unsafe 位置

以下位置从掩去注释/字符串后的固定源码枚举；cfg 只记录、不求值。

| 路径 | 非 test 行号 | test 行号 |
| --- | --- | --- |
| `crates/texzi-document/src/files/windows_identity.rs` | 122, 157 | 无 |
| `crates/texzi-document/src/files/windows_save/publish.rs` | 298 | 无 |
| `crates/texzi-document/src/files/windows_save/tests.rs` | 无 | 47, 48, 62 |
| `crates/texzi-document/src/files/windows_save/tests/creation_security.rs` | 无 | 69, 75 |
| `crates/texzi-document/src/files/windows_security.rs` | 19, 45, 67, 78, 100, 102, 119, 165, 184, 185, 186, 202, 233, 258, 259, 280, 288, 311, 319, 328, 339 | 142, 145, 391, 413, 427, 438, 439, 449, 455, 464 |
| `crates/texzi-document/src/files/windows_security/creation_policy_tests.rs` | 无 | 9, 11, 17, 26, 31, 62, 71, 73, 76, 112 |

## 模块子树与独立 crate

| crate 类别 | 模块 | 源文件 | 继承的源码 lint |
| --- | --- | --- | --- |
| lib | `texzi_document` | `crates/texzi-document/src/lib.rs` | deny |
| lib | `texzi_document::anchors` | `crates/texzi-document/src/anchors.rs` | forbid |
| lib | `texzi_document::body` | `crates/texzi-document/src/body.rs` | forbid |
| lib | `texzi_document::coordinates` | `crates/texzi-document/src/coordinates.rs` | forbid |
| lib | `texzi_document::files` | `crates/texzi-document/src/files.rs` | deny |
| lib | `texzi_document::files::staging` | `crates/texzi-document/src/files/staging.rs` | forbid |
| lib | `texzi_document::files::windows_identity` | `crates/texzi-document/src/files/windows_identity.rs` | allow |
| lib | `texzi_document::files::windows_identity::open` | `crates/texzi-document/src/files/windows_identity/open.rs` | allow |
| lib | `texzi_document::files::windows_identity::open::tests` | `crates/texzi-document/src/files/windows_identity/open/tests.rs` | allow |
| lib | `texzi_document::files::windows_save` | `crates/texzi-document/src/files/windows_save.rs` | allow |
| lib | `texzi_document::files::windows_save::current_tests` | `crates/texzi-document/src/files/windows_save/current_tests.rs` | allow |
| lib | `texzi_document::files::windows_save::current_tests::parent_replacement` | `crates/texzi-document/src/files/windows_save/current_tests/parent_replacement.rs` | allow |
| lib | `texzi_document::files::windows_save::publish` | `crates/texzi-document/src/files/windows_save/publish.rs` | allow |
| lib | `texzi_document::files::windows_save::recovery` | `crates/texzi-document/src/files/windows_save/recovery.rs` | allow |
| lib | `texzi_document::files::windows_save::tests` | `crates/texzi-document/src/files/windows_save/tests.rs` | allow |
| lib | `texzi_document::files::windows_save::tests::creation_security` | `crates/texzi-document/src/files/windows_save/tests/creation_security.rs` | allow |
| lib | `texzi_document::files::windows_save::tests::creation_security::contracts` | `crates/texzi-document/src/files/windows_save/tests/creation_security/contracts.rs` | allow |
| lib | `texzi_document::files::windows_save::tests::creation_security::unsupported_creation` | `crates/texzi-document/src/files/windows_save/tests/creation_security/unsupported_creation.rs` | allow |
| lib | `texzi_document::files::windows_security` | `crates/texzi-document/src/files/windows_security.rs` | allow |
| lib | `texzi_document::files::windows_security::creation_policy_tests` | `crates/texzi-document/src/files/windows_security/creation_policy_tests.rs` | allow |
| lib | `texzi_document::history` | `crates/texzi-document/src/history.rs` | forbid |
| lib | `texzi_document::history::failure_ack` | `crates/texzi-document/src/history/failure_ack.rs` | forbid |
| lib | `texzi_document::history::verified_open` | `crates/texzi-document/src/history/verified_open.rs` | forbid |
| lib | `texzi_document::history::verified_open::tests` | `crates/texzi-document/src/history/verified_open/tests.rs` | forbid |
| lib | `texzi_document::observe` | `crates/texzi-document/src/observe.rs` | forbid |
| lib | `texzi_document::observe::failure_ack_tests` | `crates/texzi-document/src/observe/failure_ack_tests.rs` | forbid |
| lib | `texzi_document::observed_writes` | `crates/texzi-document/src/observed_writes.rs` | forbid |
| lib | `texzi_document::provenance` | `crates/texzi-document/src/provenance.rs` | forbid |
| lib | `texzi_document::ui_plan` | `crates/texzi-document/src/ui_plan.rs` | forbid |
| lib | `texzi_document::view_store` | `crates/texzi-document/src/view_store.rs` | forbid |
| lib | `texzi_document::views` | `crates/texzi-document/src/views.rs` | forbid |
| binary-layout | `bin::document-probe` | `crates/texzi-document/src/bin/document-probe.rs` | rust-default/not-inherited-from-lib |
| binary-layout | `bin::document-probe::observe_probe` | `crates/texzi-document/src/bin/document_probe/observe.rs` | rust-default/not-inherited-from-lib |
| binary-layout | `bin::document-probe::save_probe` | `crates/texzi-document/src/bin/document_probe/save.rs` | rust-default/not-inherited-from-lib |
| integration-layout | `test::contracts` | `crates/texzi-document/tests/contracts.rs` | rust-default/not-inherited-from-lib |
| integration-layout | `test::contracts::binding_epochs` | `crates/texzi-document/tests/binding_epochs/mod.rs` | rust-default/not-inherited-from-lib |
| integration-layout | `test::contracts::file_codecs` | `crates/texzi-document/tests/file_codecs/mod.rs` | rust-default/not-inherited-from-lib |
| integration-layout | `test::contracts::file_staging` | `crates/texzi-document/tests/file_staging/mod.rs` | rust-default/not-inherited-from-lib |
| integration-layout | `test::contracts::native_plans` | `crates/texzi-document/tests/native_plans.rs` | rust-default/not-inherited-from-lib |
| integration-layout | `test::contracts::observers` | `crates/texzi-document/tests/observers/mod.rs` | rust-default/not-inherited-from-lib |
| integration-layout | `test::contracts::save_failure_ack` | `crates/texzi-document/tests/save_failure_ack/mod.rs` | rust-default/not-inherited-from-lib |
| integration-layout | `test::contracts::save_failures` | `crates/texzi-document/tests/save_failures/mod.rs` | rust-default/not-inherited-from-lib |
| integration-layout | `test::contracts::save_scopes` | `crates/texzi-document/tests/save_scopes/mod.rs` | rust-default/not-inherited-from-lib |
| integration-layout | `test::contracts::views` | `crates/texzi-document/tests/views/mod.rs` | rust-default/not-inherited-from-lib |
| integration-layout | `test::contracts::windows_bound_staging` | `crates/texzi-document/tests/windows_bound_staging/mod.rs` | rust-default/not-inherited-from-lib |
| integration-layout | `test::contracts::windows_identity` | `crates/texzi-document/tests/windows_identity/mod.rs` | rust-default/not-inherited-from-lib |
| integration-layout | `test::native_plans` | `crates/texzi-document/tests/native_plans.rs` | rust-default/not-inherited-from-lib |

document-probe 的两个 path 子模块属于独立 bin；contracts.rs 的模块属于独立 integration-test。native_plans.rs 同时是 contracts 子模块及自动发现的独立 test 根候选，两种情况都不继承 lib lint。布局来自 manifest/固定目录，未执行 Cargo metadata 验证。

## path/include 与降级入口

| 位置 | 入口/类别 |
| --- | --- |
| `crates/texzi-document/src/bin/document-probe.rs:8` | `#[path = "document_probe/observe.rs"]` |
| `crates/texzi-document/src/bin/document-probe.rs:10` | `#[path = "document_probe/save.rs"]` |
| `crates/texzi-document/tests/contracts.rs:53` | `include_str!`，data |

完整属性、每个 unsafe 的 module/cfg、固定字节 SHA256、扫描过的配置和 flags 词法命中都在配套 JSON。没有修改 RUSTFLAGS、cap-lints、feature、target 或依赖。配置文本命中不是实际执行参数；用户/全局 Cargo 配置与 ambient 环境不由本源码清单证明。

本范围仅 texzi-document 库树和该包独立入口的对照，不证明 texzi-app、workspace、依赖或宏展开全部无 unsafe；当前独立入口未见 unsafe 也不等于获得库 forbid。

机器清单：`D:/Work/texzi/.cindy-worktrees/serene-hertz/docs/t07/unsafe-inventory-69967fe.json`。所有旧失败/接受及未跟踪文件保留，不构建/运行、不重试受限通道。
