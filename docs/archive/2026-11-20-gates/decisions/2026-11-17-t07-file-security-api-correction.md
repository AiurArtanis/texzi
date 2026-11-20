# T07 文件权限 API 勘误（保留46df历史）

46df06d新增了SetKernelObjectSecurity，但核对Microsoft文档后确认该API不应给文件系统对象设置描述符；不得将该提交fmt/metadata结果称文件ACL支持。本地未执行该调用，没有修改用户文件/权限。保留46df及其静态记录，以新提交修正，不改写历史。

当前以GetSecurityInfo/SetSecurityInfo，SE_FILE_OBJECT，配合精确windows-sys0.61.2 Win32_Security_Authorization feature。读取的系统分配描述符通过LocalFree释放；复制到对齐自有存储前核有效性/self-relative/长度，拒绝未知表示。设置前取owner/group/DACL及protected位；缺owner/group或DACL absent时拒绝，避免把“无可解析DACL”无声改成NULL DACL。明确present的NULL DACL才按来源保留。API返回的DWORD错误直接转io error，不误用GetLastError。

设置对象仍仅内部create_new临时File，且在payload之前，原文档不写ACL。无SeRestore/SeTakeOwnership/SeSecurity等特权启用或全局策略变化。API失败/读回不精确一致保空temp和原capture责任；原继承security与payload security分记录。自定义ACL继承/不同owner/SACL等仍缺实际运行，不将推荐API选择当功能验收或类型检查通过。

官方依据：
- [SetKernelObjectSecurity：文件系统对象应使用其他文件安全API](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-setkernelobjectsecurity)
- [SetSecurityInfo：按handle设置，返回DWORD，NULL DACL语义](https://learn.microsoft.com/en-us/windows/win32/api/aclapi/nf-aclapi-setsecurityinfo)
- [GetSecurityInfo：handle读取、LocalFree资源、READ_CONTROL及竞态边界](https://learn.microsoft.com/en-us/windows/win32/api/aclapi/nf-aclapi-getsecurityinfo)

本次只有上述Microsoft文档阅读，未读取GitHub/CI或更换受拒工具入口；代码API签名同步核本机缓存windows-sys源。26命名组件均NOT_RUN，原失败历史及完整T07范围保留。
