# T03a 正文、Unicode 与历史组件候选

本步只增加 `texzi-document`，不接 NSH、不提供编辑器 UI，不宣称 T03/G1 完成。
原生窗口仍是 TX-00；新 crate 尚未成为应用依赖。

## 实现与边界

- 单一 `Store` 以 `&mut self` 串行提交；不可变快照共享 Rope/来源树。
  Ropey 1.6.1 关闭全部默认 features（只把 LF 当行界），底层 UTF-16 聚合度量可直接查询。
- 字节/UTF-16/grapheme 类型分别携带 DocId/Revision，严格拒绝 continuation byte、
  surrogate 中间和旧 revision；不规范化 Unicode。
- `unicode-segmentation =1.13.3`，UAX29 扩展 grapheme/Unicode 17.0.0。
  本地边界检查经 GraphemeCursor 处理 Rope chunk/pre-context。
  全局 grapheme ordinal 使用显式 `GraphemeIndex`，每快照一次线性构建，可重复查表；
  它不是逐键/UI hit-test 默认入口。跨编辑增量全局 ordinal 索引、超长行 UI 降级尚未实现。
- 正文与 EOL 是持久化 Rope；来源是独立不可变 AVL span 索引，按逻辑 UTF-8 字节计长。
  原始跨度每叶约 1024 raw bytes（不切 scalar/CRLF）；拆分只扫本叶。
  编辑只复制受影响路径；未改原始跨度保留 Arc 源及 raw range，Undo 恢复旧树。
- 导入严格 UTF-8 `&str`、可选 BOM、CRLF/LF/CR 无损；T07 其它编码/坏字节/磁盘身份仍 NOT_RUN。
  插入文本单独规范化 CRLF/CR，采用提交前主导 EOL；粘贴 U+FEFF 保留为正文。
- 全部 patch 同一 base revision、按序不重叠，先验证所有 range/EOL/边界再准备并一次交换；
  正逆 delta 以各自完整版本为坐标。取消、旧版本、非法 patch、预算拒绝不部分发布。
  NoOp 不增 revision/token/history；元数据修改独立可撤销。
- 一条 Undo/Redo，revision 单调前进、token 恢复；新编辑清 redo，保存不清历史。
  目前每个事务一个组，无自动连续输入合并（没有冒充已实现 750ms 合并窗口）。
  NSH/跨模式调用与 View ownership 待 T04/T05；独立 anchor 映射函数不是窗口实测。
- 保存接口仅内存协调：Doc/Target lease、Busy、阶段顺序、一次 permit、TooLate、
  校验 operation/binding/target/guard/捕获 revision-token、baseline=N 时 N+1 仍 dirty。
  `accept_verified_save` 是 T07 IO owner 的可信边界；组件测试模拟成功回执，绝非写盘证据。
  binding epoch 当前固定0、TargetKey 由调用方提供；别名/重绑定/失败冲突/unknown/关闭协调待 T07。

## 预算与依赖

初始历史 accounting ceiling：每文档64 MiB、每Store256 MiB。每组计
`4096 + 8*(删除UTF-8 bytes + 插入UTF-8 bytes)`；元数据组计 `4096 + 2*正文bytes`。
预算将超限时拒绝**整个新事务**，不悄悄截断旧组或半撤销。该计费是工程保护，
不是 allocator/process Private Bytes 上限证明；外部快照、原始来源、Rope共享节点等
可额外保留内存。硬内存上限、显式历史回收/段文件、跨Store进程总量协调仍待实现。
这不是已测量的性能达标或锁定最终预算。

直接：ropey1.6.1（MIT）、unicode-segmentation1.13.3（MIT OR Apache-2.0）。
Ropey传递：smallvec1.16.5（MIT OR Apache-2.0）、str_indices0.4.4（MIT OR Apache-2.0）；
后两者版本以 Cargo.lock 为准。无测试第三方依赖；随机oracle使用固定seed xorshift。
registry源码的Cargo.toml、rope.rs（clone/UTF16）、grapheme.rs（chunk/context）及
tables.rs Unicode版本已实际审读。CI metadata/SBOM/根许可收集覆盖新crate；无字体或运行时资源新增。

## 可复现组件验证

```powershell
cargo fmt --all -- --check
cargo check -p texzi-document --tests --locked --target x86_64-pc-windows-msvc
cargo test -p texzi-document --release --locked --target x86_64-pc-windows-msvc --test contracts -- --include-ignored --nocapture --test-threads=1
```

百万序列固定seed `0x20261116`：一百万次实际内容编辑（相同替换换成不同文本），记录revision与计费，
每32步整批Undo/Redo对照独立字符串oracle；持续检查END锚点及UTF16位置，
没有把一百万次全部留在历史中。固定语料另有chunk seam/长RI/ZWJ/Indic案例。
大正文用9600000 bytes样本，末尾一次编辑后比较未触及chunk指针，旧快照不变；
这是结构共享组件证据，不是10MiB输入延迟/100MiB打开/present或正式内存测量。

CI先编译并执行完整contracts测试，再构建应用；失败阻止成功包并保留native两路日志/真实exit。
包内 `component-tests/document-contracts.exe` 供在PALADIN独立重复执行同一固定版本测试。
本机无MSVC链接器，cargo check不是执行测试；CI结果与下载包复验另附evidence。

## 后续依赖闸口

T03a **没有提供** C08 ObserverSnapshot/subscribe/resume 接口，不能拿正文快照冒充恢复快照。
R4观察协议的确定交错/缺口/二次Lagged运行测试仍 NOT_RUN，后续实现该接口时必须补齐。
多选区意图规范化/selection_after、ViewId/模式和IME、history合并、派生任务取消也是后续组件范围。
当前只有已校验的patch计划和纯anchor映射，不能接入UI时绕过这些缺口。
后续在T03完整接口及R4运行协议落实后再放行T04/T05集成；本步交Rusty和原Steamy按组件范围审测。
