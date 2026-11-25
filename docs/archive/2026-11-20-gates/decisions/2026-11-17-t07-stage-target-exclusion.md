# T07-STAGE-01 暂存创建前排除目标

原91b/f775的长临时名可恰等于不存在的目标：前检Absent通过，create_new写目标后，后检才拒绝。该白盒反例成立；旧“目标不动”表述不能作为普遍保证。原代码/报告保留，本修复不倒填为旧版本运行通过；反例尚未实际运行。

write_temp现在用纯数字8.3候选：8位十进制stem加3位十进制attempt扩展名。没有大小写变体、无需另外生成不同的DOS短别名；同一parent下创建前比较原生filename，相同即跳过。stem由PID及operation确定，既非随机秘密，也不以随机化代替排除。64次候选均占用则失败，已有候选永不改写或删除。临时文件的身份来自StagedSave/recovery路径，不再依赖.texzi-*.tmp glob。

末尾点/空格、Windows非法字符/ADS、控制字符和不能无损表示为Unicode的leaf在任何创建之前返回InvalidTarget。正常Unicode文件名仍可使用；无法可靠解释的形式保留明确拒绝，不把此临时限制称作T07全部名称支持。最终父目录身份/权限/reparse协调及任意路径别名的安全发布仍未完成。本修复仅排除确定的目标/暂存同名发布，不把内容guard当CAS，也不放行Ready/permit/成功回执。

新增真实Owner/Store链fixture：attempt0目标；attempt0已存在、attempt1目标，实际应选attempt2且保留attempt0字节；旧.texzi小写/大写反例控制。断言目标不存在、temp不同、固定N字节不跟随N+1、baseline/N+1/history/dirty保留、预约仍busy/无permit、cancel后释放且保留temp；另测试5种不可靠名称在空隔离目录中零创建。测试将只使用独占生成目录并保留，当前NOT_RUN；原缺link.exe101不重试。

Windows大小写等价以消除候选字母解决，不实现另一套Unicode fold。纯数字短文件名的真实Windows文件系统行为及全部新增fixture仍待允许环境运行；静态fmt/metadata不替代产品运行。后续发布凭据必须将operation、捕获snapshot/token、目标真实身份和staged对象一起绑定，不能靠拼capture+任意路径授权替换；本次未提供最终发布函数。
