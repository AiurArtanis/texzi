# T07-LEGACY-COV-01：局部断言与来源 key 可达性

本步只追加 private lib fixture 和本记录，生产映射/正文/历史/文件组件不改。类型检查与全部运行 NOT_RUN；不撤销 `0541df9` 有限源码接受，也不称编码或完整 T07 已可用。

## 新增三项源码

1. `inserted_none_newlines_preserve_alias_and_all_mixed_eols_through_undo_redo`：手写 `A3 A0 A CRLF B LF C CR D`，logical `6..6` 插入归一化 `中\n文\nX\n`，显式新 EOL `[Cr,Lf,CrLf]`。核新 None 精确 `6..16`、旧 alias/三类旧EOL不变、新换行bytes及Undo/Redo，不依赖 dominant-EOL 猜测。
2. `memory_rebound_with_none_newlines_and_multibyte_text_maps_a_later_none_edit`：将上述实际编码结果在核手写bytes后交既有 memory_binding，继续把 None 中 `10..13` 的文改为中Y。核 imported None 中多字节/CR/LF/CRLF 的 canonical 预检，以及当前 None 的精确输出、Undo/Redo。只是内存编码模型，不是磁盘保存、baseline/epoch迁移或发布。
3. `same_format_gbk_preserves_single_byte_euro_and_double_byte_origin_on_adjacent_edits`：明确选择 GbkWebEncodingRs0844，原 bytes `80 A D6 D0 B CRLF C`；0x80 是原 Euro 单字节，D6 D0 是原中双字节，分别改相邻两处ASCII并核冻结bytes。不是format转换或不能编码失败的替代测试。

0x80接受条件仅据实际锁定 `encoding_rs 0.8.44` 源码：`variant.rs:371` 的 Gbk/Gb18030 同用 Gb18030Decoder，`gb18030.rs:115–117` 将0x80解为U+20AC。现有 texzi gbk_shape 也允许该单字节。所有预期bytes手写；同库白盒依据不是独立GBK标准oracle，严格Gbk936/Gb18030_2022仍未实现，不能用Web代替。

## 同 key 重复：当前 Owner 不可达

以下是对父候选 `a45742eb75d3d09f656575ddc31b986aaf3ce63c` 的正常公开 Owner 入口所作源码不变量论证，不覆盖私有损坏态/unsafe外部篡改，也不作为删除防御检查的许可。

- 导入：Body::import_decoded 建立原 source Arc；Tree::imported 从 start=0 顺序取 start..end，再令 start=end（body.rs:122；provenance.rs:164–196）。raw区间不交叠，CRLF完整归一片，一个raw scalar起点只出现一次。
- 编辑：split 把原range分成相邻不交叠两段（provenance.rs:137–145）；replace只保left/right并插None（198–210）。Store先拒绝重叠edit再逆序replace（history.rs:710–759、789–796）。输入/复制文字携带String/EOL，不携带可写回的OriginSpan，不复制Some来源。
- Undo/Redo：history保存并整份选取before/after Arc<Body>（history.rs:85–86、952–972），不拼接两个来源集合。共享同一历史Arc不会让单份Body内重复raw位置。
- 其它入口：改EOL会清来源为None（body.rs:205–211）；verified-open仍用import_decoded（history/verified_open.rs:63）；新FileBinding保存整个捕获snapshot的克隆（files/windows_save/publish.rs:481、511–514），不把两代来源合并。
- OriginSpan字段虽公开，Body.origins私有；Body::origins/Tree::spans返回克隆列表（body.rs:138–139；provenance.rs:212–226），没有公开写回入口。地址仅在当前map调用中作身份，Body仍持有Arc，不能把跨调用地址重用当作重复key。

因此 imported Body 构图时 `(source.as_ptr(), raw scalar start)` 在正常 Owner 下只插入一次；legacy_spans.rs:149–155 的“已有key一致允许/异bytes拒绝”均保持防御分支 NOT_RUN。本轮不新增public setter，不私造用户操作、不重构生产只为覆盖，也没有执行私有损坏态fixture。将来若改变来源拼接入口，须重新审此结论。

同一文件中两个U+3000即使分别用A3 A0/A1 A1，也有不同raw起点，不是同key冲突。

## 不同 Arc 同文本：另一个边界

正常两次 LoadedBytes.attach 产生不同DocId，旧binding先WrongDocument。调用者也可用Snapshot的公开字段拼装同DocId、不同Body::import_decoded的detached snapshot：纯encode_snapshot会因新source身份不在原map中返回NeedsLegacyByteMap。这是直接API/fixture边界，不是Store当前正文的正常Owner流程，更不是同key冲突。

旧canonical rebound后Undo恢复的是原Arc中已被基线删去的raw起点，属于key缺失；既不是不同Arc同文本，也不是重复key。持久byte catalog仍未完成，保守拒绝不等价跨保存Undo功能完成。

本步三项及防御分支/直接API边界运行全NOT_RUN；没有构建、CI/GH/linker/push、窗口/剪贴板/远端/权限发布或新独测派发，旧接受、失败和未跟踪证据均保留。
