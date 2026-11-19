# T07 无已有目标时的新临时文件保护

继已审659创建独占边界继续本地实现。没有已有目标security时，temp在任何payload之前设置protected DACL，仅授予Windows实际文件Owner Rights和SYSTEM完整访问，保原创建owner/group。不是Everyone/Creator Owner继承占位；OWNER_RIGHTS对应S-1-3-4。[Microsoft定义](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-dtyp/81d92bba-d22b-4a8c-908a-554ab29148ab)。

通过ConvertStringSecurityDescriptorToSecurityDescriptorW解析固定D:P(A;;FA;;;OW)(A;;FA;;;SY)，LocalFree释放；SetSecurityInfo只作用新temp的DACL。读回核protected位及实际DACL字节与模板一致后才写入，不能只依赖setter返回成功。文件自create_new起FILE_SHARE_NONE，无宽继承阶段外部读句柄窗口；权限不足失败不提权或换入口。Owner是实际文件owner，可能是Windows所有者组，不冒称任意环境都只等于当前登录SID。

无目标不需要复制owner/group，故不请求WRITE_OWNER；仍需原句柄read/write/DELETE/WRITE_DAC供受控后续操作。有已有目标继续请求WRITE_OWNER并复制目标描述符，不将此私有策略替换现有文件的权限保留要求。

这也是取消/失败后保留孤立temp的基本限制，Drop只关闭handle不删除文件。新目标组件暂只提供显式owner/SYSTEM策略；目录继承/协作权限与完整SaveAs政策仍待实现和审核，不作为初版完整权限支持或静默产品默认。没有UI接线/用户文件动作，所有Windows行为NOT_RUN。

[SDDL转换及LocalFree依据](https://learn.microsoft.com/en-us/windows/win32/api/sddl/nf-sddl-convertstringsecuritydescriptortosecuritydescriptorw)。未读GitHub/CI、不安装工具或启特权。
