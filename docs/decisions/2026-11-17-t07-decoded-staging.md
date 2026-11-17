# T07 decoded import、格式历史和替换前 staging

承接原Rusty `TX-07-rusty-source-review-a37d758.md` 可实施条件。此为下一本地源码检查点，Rust运行NOT_RUN；不是完整安全保存接受。

- 保留open_utf8/Profile v1语义；新增open_decoded及Body共享导入实现，绝不二次strip正文FEFF。decode_exact接受显式BOM策略以消除签名/正文歧义。手写UTF8/UTF16原字节golden覆盖正文FEFF、双FEFF、无BOM、空文件及BOM-only，普通字符修改与Undo。
- Profile增加可选file_format；v1必须None，v2必须Some且legacy utf8_bom=false。v2的encoding/BOM通过原set_serialization/唯一history分配StateToken、单调Revision并发布，无变化不新建历史。Body.to_utf8仍为内部UTF8/EOL视图，不冒充磁盘目标字节。旧probe v1字段内容保持；v2 profile输出真实选项和BOM。
- file binding原字节快路径仍核DocId/token/Body身份/原格式；格式变化必须来自捕获快照，不再是Binding私下切换。非规范旧编码原件可透传；修改后若全量编码会改写未改别名字节，则NeedsLegacyByteMap明确拒绝，等待准确原文件字节映射。decoded UTF8 provenance不是原UTF16/GB偏移。
- read_only改为同一File句柄前后len/mtime及读取长度检查，不宣称抵御同元数据改写/路径替换或完成文件ID解析。未解析文件身份的import不把Store标为已绑定磁盘。
- PreparedSave::stage_temp实现绝对目标同目录create_new独占temp、捕获payload write_all/sync_all、读回相等、目标ContentGuard前后复核、Encoding→WritingTemp→ReadyToReplace。失败保留预约与已创建temp路径；不覆盖、删除或发布目标；重复staging因阶段不合法而拒绝。ContentGuard仅内容比较，不是Windows文件ID/CAS。没有Drop自动删唯一恢复副本。

新增/更新Rust fixture源码全部NOT_RUN：格式端序/BOM变化、无变化、内存模拟N保存回执后N+1格式仍dirty、Undo回保存token、不编码时无预约；temp sibling/目标原件不动、冲突仍busy、SaveAs目标只暂存不发布、只读保留原件长度/mtime。temp fixture目录独占生成并保留原始路径，不触用户文档。内存模拟回执不是磁盘保存证据。

一次离线document --lib check实际exit101：encoding_rs0.8.44无条件build-dependency rustversion/build.rs及multiversion_no_op仍需link.exe，未检查到本次产品代码。原始日志保留，不重试/安装/换target，不把fmt当类型检查。fmt/locked metadata/diff按实际命令另记。

后续仍需失败终态及Unknown冲突闩锁、真实文件ID/epoch及SaveAs唯一协调，再接ReplaceFileW+备份/新目标无覆盖发布/事后核验与binding原子刷新。不得用临时写完成或内存receipt宣布安全保存成功。严格GBK936/GB18030标准映射、未改legacy区间原字节地图、UI打开保存、完整结构和查找均在原首版队列。

后续收紧：暂存成功仍停WritingTemp；内容比较不等于身份确认，不能自行进入ReadyToReplace。未来身份协调器须核target/parent/epoch和DocId归属后才能推进。组件新增断言临时写后commit_permit仍InvalidSave。此前154f5aa源码阶段的Ready推进保留在git历史，不套成已运行失败或通过。
