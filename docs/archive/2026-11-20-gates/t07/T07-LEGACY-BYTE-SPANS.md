# T07：显式 Web 编码的局部原字节保留

这是本地纯编码增量；源码和测试均未类型检查或运行，实际磁盘保存及完整 T07 仍 PARTIAL。

## 行为差异

原来 `GB18030WebEncodingRs0844` 可读取非规范别名字节 `A3 A0` 为 U+3000，未修改 token/Body 时整份透传；编辑相邻 ASCII 后因整份重编码会规范化该别名而返回 `NeedsLegacyByteMap`。本步为同格式的两个显式 Web 选项建立保存调用内的来源映射，保留仍有原字节证明的片段，只严格编码新增片段。UTF-8/UTF-16/严格1252走原路径。

这不是严格 `Gbk936` 或 `Gb18030_2022` 的实现，也未核定/补全相应标准映射；二者继续 `UnsupportedStandard`，原定范围不删，Web 名称不作标准版本替代。没有新增/升级依赖，仍调用锁定 `encoding_rs =0.8.44` 的无替换 decoder/严格回读 encoder。

## 不变量与拒绝边界

- 不改变公开 API、Body/OriginSpan/坐标、唯一 Store/history、token 或保存状态机。Body origin 仍是解码 UTF-8 偏移，绝不是原 legacy 文件字节位置。
- `DocId`、profile/format、精确 token+Body 透传资格沿原入口先校验。同格式 Web 的其它快照即使当前磁盘基线恰好 canonical，也必须检查来源覆盖；不能用一次 canonical 检查跳过跨保存 Undo 的未知来源。
- 原文件按无状态编码的 1/2/4 字节单位切分，每个单位调用已选 decoder 验证为单个 scalar，拼接必须精确等于 imported Body 的带 EOL UTF-8 序列化。未知单位、边界或来源无法证明时拒绝，不替换字符。
- 由 imported Body 的来源 Arc 身份和 raw UTF-8 scalar 起点对应原 encoded range；同 key 若存在互异原字节则拒绝。地址只作调用内身份，不解引用，两个不可变 Body 持续拥有对应 Arc。不用相同文字推断相同来源。
- imported 可以是保存后的 edited Body：既有 Some 保其实际原 bytes；已保存 None 段只有实际 raw 等于严格 canonical 编码才可重建，不能猜测其别名。
- 当前 Some 必须有同来源/raw 覆盖，且当前带 EOL 片段精确等于该 origin 的原片段；逐 scalar 复制原 byte range。当前 None 只严格编码。最终再无替换解码整体输出，与当前 Body 序列化全串比较。
- 编码/映射失败发生在 `begin_save`/reservation/fence/temp 前，不产生部分输出或 Store 修改。更改格式仍走既有明确全量转码，不拿旧格式 bytes 混入新格式。

## 仍未完成

映射只属于当前 FileBinding 的原字节范围，不是贯穿所有保存代次的持久 byte catalog。跨保存 Undo 若恢复当前绑定已不再持有的旧 Some 来源，返回 `NeedsLegacyByteMap`；不静默规范化旧别名，也不把保守拒绝称功能完成。private lib fixture 用内存构造下一绑定，只覆盖编码边界，不是原生保存成功、磁盘回执或 baseline/epoch 迁移证据。

映射、来源及全串序列化有额外文件级内存/扫描和树索引开销；没有性能、硬预算或 OOM 恢复证明。类型/fixture/native IO/ACL/GUI 全部 NOT_RUN；原覆盖、SaveAs、严格GB、恢复/UI 等要求及旧失败保留，不扩构建/linker/GH/CI/push/GUI/剪贴板/远端/安装/权限/发布范围。
