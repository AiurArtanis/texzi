# T07 真实重命名成功后的失败回归源码

承接748有限源码审核指出的缺口：调用前目标碰撞不能代替rename成功后失败。本步不重做旧碰撞用例，新增2个Windows lib unit，均走原ObservedTarget/Store/owner-only new-ready/permit/native rename链。

1. post_rename_flush_failure_keeps_resources_baseline_and_held_scope：只有rename_new返回成功后才执行test-only flush-handle钩子。钩子核原File当前名为saved.txt，真实DuplicateHandle成功降为READ_CONTROL后才增加计数，再由原run的sync_all调用触发真实权限不足。DuplicateHandle失败不计为flush故障，回归会失败。预期ACCESS_DENIED5仍需实际Windows运行；不是磁盘满/硬件故障覆盖。[FlushFileBuffers要求GENERIC_WRITE](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers)。
2. post_rename_verification_failure_keeps_resources_baseline_and_held_scope：真实rename/原File flush/实际读回后，test-only格式预期override使原核验谓词返回VerificationMismatch；不改物理N数据，不直接返回预制错误，不声称磁盘已实际损坏。

两例核实际handle当前路径已到目标、原temp路径不存在、独立读仍被共享约束拒绝、owner原句柄读回完整UTF16LE+BOM/EOL N字节。记录原capture后只有一个元数据批次，终态Unknown、active释放；baseline/原逻辑绑定/epoch、N+1 token/body/history/dirty不改变，捕获N恢复快照保留，候选恢复路径含实际新路径。

另以同目录、不同实际file-ID key的existing-other.txt经真实scoped prepare检查UnresolvedConflict，避免仅旧new-in-directory key恰相等造成假覆盖。重复终态拒绝；Drop资源后实际目标字节保留，held parent scope仍不释放。资源生命周期只覆盖“已拿到失败对象并明确记录后Drop”，不解决工作线程中断/丢结果/进程崩溃/未record就Drop，must_use仍不是恢复日志。

两个故障入口只cfg(test)，普通release/probe构建没有字段、setter或环境开关；正常flush仍原创建File，正常格式仍同一捕获格式。未运行任何fixture/ACL/rename或修改用户文件。本次35新增T07命名项=26原integration/组件+9Windows lib unit，全部NOT_RUN。旧contracts.exe不能覆盖lib unit，不复用旧hash；无新CI/构建/GUI/远端/剪贴板/权限变化或发布。完整SaveAs/普通替换/恢复生命周期及SEC01-B仍未完成。
