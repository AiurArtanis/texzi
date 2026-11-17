# T07：当前保存和显式另存为的前置凭据

本步仅加固 Windows owned staging/新目标发布组件，未接应用 UI、未启用已有目标最终替换，不等价完整 Save/SaveAs 或 CAS。

FileBinding 可带私有磁盘凭据：完整 FileVersion（实际 file-ID/security/mtime/bytes）、捕获解析路径、binding epoch、持属性句柄的 OpenAnchor。只有 verified-open Imported 或 PublishedNew 经 owner 成功接受后创建。普通 LoadedBytes::attach 仍没有磁盘凭据。

verified-open 在原强读 File/parent 保护下复制属性句柄，复制失败不进入 Store；Existing 不产生 FileBinding。Imported 的 Store 和 binding 各持一份属性句柄。关闭仅释放 Store 份额，旧 binding 不能因此恢复已关闭 Doc。PublishedNew::accept 在原 exclusive File 上重读完整 version、核属性句柄同 ID/path，所有返回式失败均在 Store 接受前；失败保留原 capture/Guard/恢复资源，不先推进 baseline。属性句柄只有 READ_ATTRIBUTES，share READ|WRITE|DELETE；Windows 共享兼容性仍待运行，不宣称永久锁住内容或路径。

## 入口

- prepare_current 必须有磁盘凭据。先核 Doc 存在、baseline token/binding epoch/current target key 相符及原对象身份见证，再核目标完整观察版本和解析路径。旧代次、外变、同 bytes 换 ID、另一目标均在编码/预约/temp 创建前拒绝。
- prepare_save_as 是显式换目标。有凭据时仍核旧 owner 代次/基线/身份，但不拿 source 指纹描述 destination。新目标沿既有独立观察、双域预约和 owned worker 写前后检查；允许把正文另存新目标而不覆盖外变 source，未启用已有 destination 的最终替换。
- prepare_bound 兼容入口：有磁盘凭据时按 current 处理；无凭据历史内存 fixture 保原目的地 staging 语义，不是普通 current-save API。公开内存 FileBinding::prepare 也不升级为真实发布凭据。应用须按动作选择 current/save_as。

当前路径精确比较，另一硬链接/别名不会自动当作原目的路径；保守拒绝不等于完整路径兼容。payload 仍来自被捕获的唯一 Store snapshot/token/format，保存 N 期间 N+1 编辑不改变 N 的 payload。磁盘绑定的 source 属性句柄在预约前复制，并贯穿 BoundPrepared/BoundWork/BoundStaged/LockedStaged/发布及恢复资源；调用方丢弃 binding 不会提前结束该 worker 的身份见证。复制失败不取得预约。

新 fixture 均 NOT_RUN：6项 current/外变/另存为/代次前检，加1项真实 rename 后属性句柄转交失败（测试修改预期 handoff 路径，经真实属性 open 的 NotFound，保持 baseline/N+1/恢复资源）。保存回执模拟只测内存协议，不证明写盘；正常发布原 fixture 增加实际 post-version/path/epoch 凭据断言。

来源复核不是 CAS，known_fingerprint 投影仍 None。Store 的保存后对象见证转交、恢复解锁/崩溃日志、外部竞争版本保全、普通覆盖、完整权限策略、严格 GB 标准和完整 T07 尚未完成。
