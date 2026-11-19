# T07 returned-error 正式条件对应与补证

正式接口方向见 `TX-07-rusty-returned-error-resource-interface.md`（SHA256 `001779F9E3E34706EB30478F26DB6357694B81BBF1AC5F73E80F41D6F1060214`）。该审阅对照054提案；实际资源持有实现已在69ecde2，当前父候选b79f224保留该实现。本步仅补private lib边缘用例与本记录，不重复实现，不把方向PASS当实现接受。

## 六项不变量的已有实现

| 正式条件 | 当前源码对应与限制 |
| --- | --- |
| 首次metadata前已有持有 | staging的create_new成功后即入私有Arc<CreatedTemp>，首hook/identity/security/payload在其后；创建前不能构造临时能力。unknown metadata不会阻止返回原File。 |
| 私有capture权威 | CreatedTemp和BoundRecovery存原capture/expected/目标context，公开capture/path仅显示副本，不参与重绑私有资源。没有公开File/raw handle/写权限setter；原handle test getter仅cfg(test/windows)。 |
| 意图与事实分列 | expected为N计划；bytes/current_path/current_version从同原handle实际读，各自Result且不改原issue。current_version是identity/security/mtime/bytes成组Result，不是逐项独立getter；不缓存或伪造未知历史metadata。creation/payload security历史值仅在成功StagedSave字段记录，不能把错误回执的当前security称为历史预期。 |
| Arc/Mutex连续 | 原File始终FILE_SHARE_NONE，显式Err保同Arc，成功只释放guard不关File。bytes读取先rewind，poison仅检查不clear，不放开发布；Debug不读/打印正文或descriptor。最后一个持有者才关句柄，不删temp。 |
| Bound上下文 | begin/早期recheck/内层Err/成功staging后外围Err均持原ObservedTarget parent、HistoricalSource、source_anchor和format；temp不存在明确None。LockFailure原完整staged语义不改。 |
| Store独立 | recovery构造/getter/Clone/Drop不finish/cancel、不写终态/epoch/baseline、不给permit或registry ack。旧capture仍按完整身份验证，不能认领新预约。 |

裸TemporaryRecovery持N Snapshot及profile/预期bytes，但没有单独保存PreparedSave.format字段；显式format的保证限定BoundRecovery。成组version观察的失败不会更新原issue，路径/bytes仍可各自查询。错误回执不提供creation/payload security历史缓存；它们可能已成功读过但没有随该回执保留，当前getter不能当历史预期。本轮不新增历史或单项identity/security API，不把现有能力描述为逐项独立观察；该分组表述与边界交原Rusty对照正式条件审定。

## 四项补充源码（全部NOT_RUN）

1. **无temp早期错误**：真实外改target触发BoundWork开始的Changed，creation hook计数0；目录path+bytes除受控外改外不变、temporary/path提示None，原context/capture/format及N+1/历史等仍持有。
2. **首creation hook真实读权限错误**：只先记录原raw/path，不查询temp metadata；成功DuplicateHandle为READ_CONTROL后真实read一字节，预期OS5，将真实Err返回。工厂初ID/security与ACL复制尚未到达，copy计数0。实际0/expectedN分开；修改公开capture/path显示副本后，私有权威与原File不变。后续getters各返回独立观察Result，不覆盖原OS5。
3. **真实前缀与期望分开**：完成真实staging后，通过测试私有同进程RW duplicate截短为NA并sync，随后drop该duplicate，再让生产外围核验因actual!=expected返回Changed。核actual前缀/expected NA CRLF、重复读的rewind、原issue不变。不能称普通外部写者越过SHARE_NONE，也不冒称生产write_all部分写/flush/read系统调用失败。
4. **最后一个Arc才释放**：真实outerChanged保fullN；clone两个BoundRecovery，丢原failure和一个clone时reader仍拒绝，最后clone释放后路径可读完整N且temp未删除，外部target版本保留。全过程Store/cursor/history/dirty/epoch/active WritingTemp不因getter/Drop变化。

四项均在捕获N后再编辑N+1并冻结状态；复用69真实生成fixture及已有私有hooks，不修改生产、不新增setter/unsafe。旧外围错误/旧取消worker新预约用例不重做。

## 覆盖不应扩大

首hook的ReadFile权限错误不是getter自身失败，也不是factory初identity/security查询失败的实际注入；生产结构已保失败Result，但这些专门故障仍NOT_RUN。截短测试命中成功staging后的外围读取/身份验证错误，不证明write_all/sync_all/readback系统调用的错误分支已运行。类型、OS5/32/ACL/IO、线程和GUI全部NOT_RUN，不能拿源码断言当实测。

只补显式Err持有；丢worker/panic/abort、丢全部回执、进程退出、预算、许可前registry、Unknown解除、普通覆盖/实际victim备份、strictGB/完整SaveAs/UI仍未完成。既有BeforeReplace ack/registry ack/close规则未改，所有历史接受、失败与未跟踪文件保留。
