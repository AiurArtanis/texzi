# T07：当前保存和显式另存为的前置凭据

本步仅加固 Windows owned staging/新目标发布组件，未接应用 UI、未启用已有目标最终替换，不等价完整 Save/SaveAs 或 CAS。

FileBinding 可带私有磁盘凭据：完整 FileVersion（实际 file-ID/security/mtime/bytes）、捕获解析路径、binding epoch、持属性句柄的 OpenAnchor。只有 verified-open Imported 或 PublishedNew 经 owner 成功接受后创建。普通 LoadedBytes::attach 仍没有磁盘凭据。

verified-open 在原强读 File/parent 保护下复制属性句柄，复制失败不进入 Store；Existing 不产生 FileBinding。Imported 的 Store 和 binding 各持一份属性句柄。关闭仅释放 Store 份额，旧 binding 不能因此恢复已关闭 Doc。PublishedNew::accept 在原 exclusive File 上重读完整 version、核属性句柄同 ID/path，所有返回式失败均在 Store 接受前；失败保留原 capture/Guard/恢复资源，不先推进 baseline。属性句柄只有 READ_ATTRIBUTES，share READ|WRITE|DELETE；Windows 共享兼容性仍待运行，不宣称永久锁住内容或路径。

## 入口

- prepare_current 必须有磁盘凭据。先核 Doc 存在、baseline token/binding epoch/current target key 相符及原对象身份见证，再核目标完整观察版本和解析路径。旧代次、外变、同 bytes 换 ID、另一目标均在编码/预约/temp 创建前拒绝。
- prepare_save_as 是显式换目标。有凭据时仍核旧 owner 代次/基线/身份，但不拿 source 指纹描述 destination。同 canonical 源路径（包括已换 ID）或同 file-ID 硬链接转 Current 全查；不同硬链接路径保守拒绝。已存在 leaf 通过实际 handle/full version 解析 canonical 路径，不能只比较输入拼写。真不同目标沿既有独立观察、双域预约和 owned worker 检查；未启用已有 destination 的最终替换。
- prepare_bound 兼容入口：有磁盘凭据时按 current 处理；无凭据只允许未绑定/epoch0/backed=false、初始内存基线及原 token 相符的历史 fixture。Doc 已发布/绑定后，旧 None binding 不得再走 bound 或 save_as；须使用新成功结果的磁盘 binding。所有入口先 snapshot 验 Doc 存在再读 metadata，close 后返回错误。公开内存 FileBinding::prepare 也不升级为真实发布凭据。

当前路径精确比较，另一硬链接/别名不会自动当作原目的路径；保守拒绝不等于完整路径兼容。payload 仍来自被捕获的唯一 Store snapshot/token/format，保存 N 期间 N+1 编辑不改变 N 的 payload。磁盘绑定的 source 属性句柄在预约前复制，并贯穿 BoundPrepared/BoundWork/BoundStaged/LockedStaged/发布及恢复资源；调用方丢弃 binding 不会提前结束该 worker 的身份见证。复制失败不取得预约。

新 fixture 均 NOT_RUN：6项 current/外变/另存为/代次前检，加1项真实 rename 后属性句柄转交失败（测试修改预期 handoff 路径，经真实属性 open 的 NotFound，保持 baseline/N+1/恢复资源）。保存回执模拟只测内存协议，不证明写盘；正常发布原 fixture 增加实际 post-version/path/epoch 凭据断言。

同目录非 ASCII/非 Unicode 名称若不能证明是已知 Current 路径或原对象，无论目标已存在还是不存在，都暂返回 InvalidPath。判定采用捕获的实际 parent file-ID 或相同历史 parent 路径，避免只依赖一种目录字符串关系；该 ID 是前检分类信息，不充当最终文件 identity/namespace 锁。新对象的 canonical 拼写不同不能证明它与历史目录项独立，不用 Unicode lower/upper 或猜测折叠授权。已证明的 Current 仍做完整版本核查；ASCII 同源拼写保守转 Current，大小写敏感目录可能多拒绝；真正不同父目录的新目标保留。该名称等价核验能力与完整 Unicode SaveAs 仍待补，不据保守拒绝宣布功能完成。

新增 existing_unicode_case_equivalent_new_object_is_not_an_independent_save_as 隔离 fixture 先实际确认 é/É 在生成目录里同 ID，随后保留旧对象并创建大写拼写的新 ID，核历史/新 canonical 不同、旧拼写仍能寻址新对象。拒绝前后完整 state/cursor/目录 bytes 保持，随后不同 parent 的合法准备号不跳号。前提不满足使用明确 PRECONDITION_NOT_MET 失败标记，不返回伪 PASS，也不改系统大小写设置；本 fixture 尚未运行。

新增防绕过 fixture 全部 NOT_RUN：同源 bytes/ID 外变、硬链接、真实发布后旧 None binding、closed Doc，以及大小写 alias/名称歧义边界。大小写 alias fixture 明确要求默认大小写不敏感临时目录，不改系统设置；不扩为全卷/全文件系统验收。

来源复核不是 CAS，known_fingerprint 投影仍 None。Store 的保存后对象见证转交、恢复解锁/崩溃日志、外部竞争版本保全、普通覆盖、完整权限策略、严格 GB 标准和完整 T07 尚未完成。

## 历史父目录被替换时的关系检查

有磁盘凭据的独立 SaveAs 在编码/预约前重新观察完整历史源路径。父目录/读取/权限/reparse/path 错误直接返回，不以错误当作“独立目标”；源 leaf 不存在但父目录仍可验证时保留明确的 version=None。新的 HistoricalSource 只用于命名关系：双方重检后，当前实际 file-ID 相同，或当前实际 parent-ID 相同且 leaf 精确/ASCII 保守相等，则转 Current 全查原 disk.version/path。历史源当前 parent 与目的 parent 相同也纳入非 ASCII 歧义拒绝。没有 Unicode lower/upper 推断，不把历史 parent ID 不同当独立目的地证明。

历史路径解析到的外部新对象及 bytes 不更新旧凭据、正文或 baseline。真正独立 SaveAs 仍编码唯一 Store snapshot；该关系观察按值随 Prepared/Work/Staged/Locked/发布与恢复资源持有，在写前后、发布前后及 owner accept 前重检。它临时 pin 当前解析到的源 parent，不是 Store 永久目录锁或 CAS。普通前检失败释放局部观察；消费 Work 返回 BoundFailure 时只保既有 capture/temp 路径，历史观察随 Work 释放；LockFailure/PublishFailure/registry 沿既有 owned 资源保留。

如果历史 parent 已无法读取/解析，或刚捕获的历史观察随后变化，独立 SaveAs 也会保守失败；这不是完整恢复 UI。额外全文件读取、缓冲及操作期目录 pin 的资源成本未测。新增 parent_replacement 子模块 fixture 只针对父目录 éParent→ÉParent 换 ID、ASCII source.txt 的反例，明确前提失败标记；所有 Windows 行为与 Rust 执行仍 NOT_RUN。
