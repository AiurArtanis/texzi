# T07：verified-open owner 导入边界

`ObservedTarget → OpenedExisting::attach(&mut Store)` 是本地 Windows 组件入口，尚未接应用打开菜单或 probe 文件 IO 命令。严格解码发生在此前的 captured-open；失败不会进入 owner 导入。已有编辑内核、正文和唯一 history 沿用，不借打开行为重新生成正文。

## 凭据和结果

attach 消费私有 owned 捕获，原读 File 和 parent 持有到 owner 返回。先 recheck，再在读锁保护下开同对象的属性句柄，核 file-ID 并再次 recheck。owner 只接内部私有字段创建的 OpenAnchor，不接受外部任意 key/text/raw 组合。失败返回 AttachFailure，保留原捕获；不会改磁盘或自动重试。

- `Imported { snapshot, binding }`：新 DocId、Revision(0)、新初始 StateToken，epoch=1、backed=true、baseline=初始token、来源 VerifiedOpen、空 history/View、dirty=false。原字节 Arc 与该初始快照/token/Body 身份配对，未修改字节透传沿用既有 FileBinding 条件。没有保存成功回执。
- `Existing { snapshot }`：仅定位当前 DocId。请求的磁盘解码/format 不应用到已有文档。不会创建 FileBinding、分配 token/DocId，或改正文、格式、baseline、epoch、history、View、fence、observer cursor、保存状态和冲突。

## 身份存续

Store 为每个 verified-open 新文档保留 FILE_READ_ATTRIBUTES 属性句柄，share READ|WRITE|DELETE；它是对象存续见证，不是正文/安全描述符/路径锁。保留期间仍允许正常写入和替换，不能据此断言磁盘内容未变。新的捕获与被保留的属性句柄均须核实各自实际 ID，才可按同一对象定位 Existing；硬链接不因路径不同另建 DocId。

旧绑定没有 Store 见证、或保存回执没有向 Store 转交新对象见证时，即使 key 字符串相同也返回 ContinuityUnproven。现有成功保存接受入口使该文档的 Store 见证失效；关闭释放 Store 份额，调用方仍持 FileBinding 时其独立属性句柄可继续存续，但不能恢复已关闭 Doc。不会用 raw/mtime/路径相同弥补连续性，也不把新捕获 raw 配到旧 dirty token。返回 binding 的磁盘凭据见 T07-CURRENT-SAVE-PREFLIGHT.md；恢复/显式核验及向 Store 的保存后见证转交尚未实现。

## 错误顺序与原子性

捕获/最终身份核查在前。owner 先查同 key 的已绑定文档：证明连续性才 Existing；无见证即 ContinuityUnproven，保留原冲突。新文档按固定顺序检查：held parent/object 或 unresolved → UnresolvedConflict；其后 active parent/object 或 target 预约 → Busy；再 profile、token 和全局 DocId 计数。此顺序不依赖 HashMap 遍历。Existing 不解除 Busy/Unknown，单纯定位不当作冲突解决。

新文档先准备 Body/format、计数、身份见证和全部返回式错误，再由 &mut Store 单一 owner 插入 docs、bound_targets、身份见证与完整初始 journal。没有 open_decoded 后再 fallible bind 的半文档路径。全局 DocId 号段允许安全空洞，不回退全局原子计数；普通返回错误不改其它文档、保存预约或 cursor。OOM/任意 panic 的事务恢复未实现，不把 Result 原子性扩称分配失败可恢复。

## 尚未证明

新增 Windows lib unit 全部 NOT_RUN，没有类型检查/构建/运行证据。两项保存状态 fixture 只模拟内存可信回执，不证明 IO。计数故障仅 cfg(test) 使用独立 MAX 原子计数器，不更改生产 NEXT_DOC 或全局状态。probe 的 baseline kind 使用现有 Debug 输出，文档补 VerifiedOpen；没有为 probe 新增打开命令。

只有本地 volumeGUID、受支持的普通非末级 reparse 文件；正常共享句柄范围外的 mapping、特权写、权限竞争和 OOM/资源预算无保证。强读/属性句柄边界、硬链接行为仍待合法 Windows harness 实测。释放读锁后保存仍须重新观察并处理外变；现有 known_fingerprint 投影仍为 None，不宣称完整外变检测或 CAS。严格 GBK936/GB18030-2022 仍 UnsupportedStandard，完整打开/覆盖/SaveAs/恢复 UI 与 T07 未完成。
