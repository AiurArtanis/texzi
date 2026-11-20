# T07 权限不匹配零payload及匹配正控制fixture

对应Rusty对a416的未测条件。a416的严格预检查在23629d8已演进为真实SetSecurityInfo复制后的校验，因此本轮测试当前实际post-copy/pre-payload安全边界，不人为恢复旧产品分支或以假错误入口代替。

新增两个Windows library unit fixture，经真实ObservedTarget→FileBinding→唯一Store/scoped capture→BoundWork.write；只创建独占生成目录，操作其新temp/原fixture文件，不读取或改用户文件ACL。SecurityDescriptor从OS读取，测试用Get/SetSecurityDescriptorControl只在其自有拷贝中翻转DACL protected位，保持有效结构而非伪造字节。

cfg(all(test,windows))私有钩子位于真实复制成功后、payload_security读回之前，只有新create_new File参数。负控制用真实SetSecurityInfo把已复制权限改为不同protected状态，核OS读回不同，再进入产品原SecurityMismatch判断；正控制重新应用匹配描述符。两者断言钩子调用一次、当时len0；不mock IO/API或直接返回预制错误。release/probe构建无此字段/方法/钩子，无环境变量或普通用户故障开关。

负控制断言回传原capture/op/epoch/token/Body Arc、恢复文件len0/空字节，原capture可finish_save_failure(BeforeReplace)进入Failed且保其恢复快照。正控制生成N固定payload/format，可核owner/锁但不permit，随后显式cancel。两例都在worker执行前编辑N+1，核baseline、N+1/body/history/dirty不变；完整原fixture fileID/security/bytes/mtime不变，失败/取消终态属于原operation。

这两项为lib unit目标（非contracts可执行文件），26既有integration/组件命名项加2共28，全部NOT_RUN。未来合法Windows构建须单列library unit可执行入口/日志，不能只复跑旧document-contracts.exe就称覆盖两例。本轮不构建或重试缺linker/CI，不操作桌面/剪贴板/远端。测试钩子及用例源码不是实际权限变更或运行PASS，完整ACL矩阵/安全保存仍PARTIAL。
