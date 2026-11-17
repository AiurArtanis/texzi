# T07 原句柄不覆盖新目标：有限组件能力

本步在已审独占创建/双域/epoch基础上继续实际代码，不代表完整T07、现有文件替换或GUI保存完成。只接受捕获时不存在的目标，显式prepare_new_owner_only选择owner/SYSTEM策略，不能走已有目标覆盖或目录继承发布。现有完整范围保留。

## 绑定及状态顺序

LockedStaged.prepare_new_owner_only→NewReady（worker复核独占File/parent/格式、打开并核同一父目录的ADD_FILE/TRAVERSE能力、验证leaf）→permit（owner一次observed_write核完整capture/epoch/scoped parent且object=None/阶段/epoch上限，内部Ready后CommitPermit，完成即Replacing不可取消）→NewPublication.run（不借Store的worker）→PublishedNew.accept（owner更新保存点与最终绑定）。所有资源类型不可外部拼装，许可/结果消耗self，must_use提醒处理剩余操作责任。

实际新文件命名使用SetFileInformationByHandle(FileRenameInfo)：源是原create_new File，RootDirectory为已核身份/权限且保持不共享DELETE的父目录handle，FileName为相对leaf，ReplaceIfExists=false。不关闭独占File、不按源路径重新找文件、不用copy/delete fallback。目标最后出现也由文件系统操作拒绝覆盖，precheck不是CAS。

worker在原File上flush→rename→flush，从同一handle重新查询最终FileIdInfo/GUID路径/实际字节/有效描述符，核已捕获payload/Format及parent；与创建ID对比是完整性检查，最终身份来自事后查询，不把旧new-in-directory key或缓存tempID当成功绑定。文件保持独占至owner接受或显式恢复处理。

accept使用最终实际file-ID key迁移持续绑定，原预约capture.target只用于核旧操作与释放其双域；接受保存token N并刷新epoch及FileBinding的实际读回raw Arc/捕获Snapshot N。N+1正文/格式/history不回滚、不清dirty。新元数据在一个observed_write接受；内存旧public accept_verified_save语义保留，仍只代表调用者可信回执。

## 失败与保留限制

许可前失败按BeforeReplace；许可后尚未调用rename按ReplaceFailed；rename尝试后任何失败保守OutcomeUnknown。PublishFailure拥有原独占File，record_failure仅记录原capture终态、候选恢复路径，不消耗对象或自动删除/重试；可从原File读取当前版本/内容/路径。候选path不是恢复身份凭据，处理失败后close/生命周期、重启恢复及核实解锁仍待完整实现。Drop仍会关资源，must_use不是持久日志或自动收尾。

只提供文件flush及事后读回的源码路径，没有断电/卷metadata持久性证明、完整ACL/SACL/标签/EFS矩阵、不同owner、多主体SEC01-B、计数极限、目录权限竞争完整运行证据。新文件owner/SYSTEM策略是有限显式能力，不把它等同完整SaveAs策略。

## Fixture与接口边界

4新增Windows lib unit：独立worker等待时owner继续改变N+1编码/正文，保存N实际UTF16LE+BOM/EOL、最终key/epoch及Undo保存点；最后检查后create_new外部完整版本，实际rename不得覆盖、记录Unknown并保原capture；取消旧Ready后新预约不受旧许可/终态影响；已有目标明确拒绝且原fileID/security/bytes/mtime不变。全部NOT_RUN，不以schema/fmt当运行。

33新增T07命名项=26既有integration/组件+7 Windows lib unit（3安全+4新目标）；旧contracts.exe不能覆盖lib unit。没有运行这些方法/fixture，没有UI/GH/CI/push/远端/剪贴板/安装/全局权限变化或正式发布。必要后续是已有文件安全替换/外部版本保全、完整打开绑定、恢复处置/权限政策、严格GB标准及应用接线，不能以本分支保守拒绝标为这些能力完成。

依据：[FILE_RENAME_INFO的RootDirectory/相对名和ReplaceIfExists](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info)、[SetFileInformationByHandle](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle)。API签名/结构同时核本机windows-sys0.61.2缓存。
