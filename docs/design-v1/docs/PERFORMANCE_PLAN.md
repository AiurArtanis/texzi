# texzi 性能、响应与资源验证计划 v1

状态：**测量方案与目标，尚无 texzi Windows 实测结果**。日期：2026-11-15。技术路径见 [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md)，执行任务见 [HANDOFF.md](HANDOFF.md)，产品验收见 [ACCEPTANCE.md](ACCEPTANCE.md)。

本文件将“用户已接受的建议指标”与“为实施提出的其他工程预算”分开。所有目标必须附硬件、软件版本、模式和样本；不能以 NSH 官方 benchmark、单次最快结果、开发机肉眼感受或交互原型表现代替产品证据。

## 1. 指标与判定口径

### 1.1 已接受的目标数值与建议测量口径

用户已接受下表的目标数值。具体测试机、S0/S1 会话、冷启动逐轮判定、普通 Markdown WYSIWYG 的样本范围及其他场景矩阵，是本方案为实施提出的展开建议，并非用户已经逐项确认的细节。实施按这些清楚标识的建议建立基线，不把场景展开的来源改写为用户原话。

| 指标 | 已接受的目标数值 | 建议的测量边界与场景范围 |
|---|---|---|
| 热启动 | p95 ≤800 ms | 从启动器即将调用 CreateProcess 到活动编辑区首次完成可见呈现且可以接受输入。无 texzi 常驻进程；系统文件缓存可热。S0/S1 分开计算。 |
| 冷启动 | ≤2,000 ms | 重启后首次启动，同一 ready 边界。用户未指定百分位，建议按每轮有效测量≤2 s 判定，同时报告 p50/p95/max；不偷偷放宽成仅 p95 达标。 |
| 1–10 MiB 输入 | p95 ≤33 ms，p99 ≤50 ms | 从应用收到输入/IME 正式 commit 到包含该事务的第一帧 present；1/10 MiB 纯文本、源码、普通 Markdown WYSIWYG 分开评估。真实 IME 另有端到端核验。 |
| 60 Hz 滚动 | p95 帧间隔 ≤20 ms | 持续主动滚动期间的实际呈现帧间隔；每个指定文件规模/模式分别计算，不把静止时间或未绘制帧删掉美化结果。 |
| 100 MiB 普通文本首屏 | ≤2,000 ms | 已运行且空闲的应用收到打开文件请求，到实际正文首屏完成呈现；加载提示或占位骨架不算首屏。 |
| 100 MiB 普通文本可编辑 | ≤5,000 ms | 同一打开请求起点，到整个文档模型可接受真实编辑、撤销、正确保存；不能只加载前缀便标 ready。 |

本方案建议将已接受的 1–10 MiB 输入目标延用到 MD-1/MD-10 普通 Markdown WYSIWYG，并单独测量；这属于场景展开，用户未逐项指定其样本细节。该场景必须使用真正的富编辑器。**100 MiB 的打开目标以纯文本及 Markdown 源码为性能基线，不扩展成任意 100 MiB WYSIWYG、巨表、单行或复杂嵌套文档的承诺。** 复杂文档仍有响应、取消和正确性验收；若不能富编辑，不得把只读预览算成功。

“输入至 present”尚不包含键盘硬件传输和屏幕像素响应。必须同时保留输入注入→应用接收、提交→布局、布局→present 的分段结果；抽样用高速摄像验证可见响应，不能把 handler 执行时间命名为完整输入延迟。

### 1.2 新增的工程预算建议

以下数值帮助实施团队定位问题，**不是用户已单独确定的要求**；在 G0 性能机锁定后记录为项目预算，若调整，写清理由，不修改上一表已接受目标。

| 场景 | 建议预算 | 失败表现与处理 |
|---|---|---|
| 一般菜单、标签、设置点击 | 100 ms 内给出可见反馈 | 超过则显示正在处理；不冻结输入线程。 |
| 可取消后台工作 | 取消 p95 ≤100 ms，max ≤250 ms | 以界面不再显示旧结果/工作进程被终止为终点；未提交事务保持不变。 |
| 10 MiB 字面查找 | 首结果 p95 ≤100 ms；完整扫描 p95 ≤500 ms | 如结果很多，分页显示并异步计数，不用“10,000+”冒充完整结果。 |
| 100 MiB 字面查找 | 首结果 p95 ≤250 ms；完整扫描 p95 ≤3 s | 阈值后显示进度；取消始终可用。 |
| regex 查询 | 250 ms 显示仍在计算；默认硬预算 2 s | 超时显示明确错误/重试操作，不显示 0 匹配；模式编译也计入预算。 |
| Replace All | 100 ms 内反馈；10 MiB 常规样本 1 s 内规划并提交 | 复杂/大量替换可继续后台处理并取消；只在完整规划后一次提交。 |
| 1 MiB 富编辑/源码切换 | p95 ≤150 ms | 已有正确投影时测量；无文本/undo 变化。 |
| 10 MiB 富编辑/源码切换 | p95 ≤500 ms | 未完成投影时可见加载且可取消，单列 cold projection 时间，不混入缓存命中结果。 |
| 常规 Markdown 首次富编辑 ready | 1 MiB p95 ≤1 s；10 MiB p95 ≤3 s | 以实际可编辑正文为终点，不能仅出现窗口壳。 |
| 100 MiB 输入 | p95 ≤50 ms、p99 ≤100 ms（探索预算） | 仍记录已接受 1–10 MiB 标准作对照；不把探索值改写为用户承诺。 |
| 100 MiB 本地保存 | 可见反馈≤100 ms，保存完成 p95 ≤3 s | 包含编码、flush、替换及必要核验；磁盘状态异常单列，不合并进正常结果。 |
| 正常空闲 CPU | 60 s 窗口内≤单核 1% | 同时报告按全部逻辑核归一的数值，避免不同口径混比。 |
| 未保存内容持久化 | 通常≤2 s 的恢复窗口 | 故障注入验证最后已 flush 的事务；不是断电零丢失保证。 |

### 1.3 内存与包体预算建议

| 场景 | 建议预算（Private Bytes） |
|---|---|
| 空白窗口稳态 | ≤180 MiB |
| 单个 10 MiB 文档常规编辑 | ≤400 MiB；纯文本和富编辑各测 |
| 单个 100 MiB 纯文本/源码稳态 | ≤1.2 GiB |
| 同时搜索/保存的 100 MiB 峰值 | 主进程+任务子进程合计≤1.6 GiB |
| 同一 10 MiB 文档增加 3 个视图 | 增量≤100 MiB；必须解释缓存与视图成本，不能复制 3 份文档历史 |
| 连续打开/关闭同文档 50 次 | 最后稳态相对第 5 次增长≤20 MiB 或≤5%，取较大者；同时分析可回收缓存 |

包体不设置人为硬上限。每个 Release 报告安装器压缩体积、安装后总字节、可执行文件、字体、语法资源、第三方库各占比，以及相对上版变化。不要把删除恢复功能/中文字体/可访问性当成减包优化。上述内存数值仍需 G0/G1 校准；同时报告 Working Set、Private Working Set、峰值提交量与 GPU 内存，不拿一种指标替代另一种。

## 2. 测试环境与版本锁定

### 2.1 建议主性能机

| 项目 | 建议配置；实施前填入实际值 |
|---|---|
| CPU | Intel Core i5-12400，6 核 12 线程；记录 BIOS、频率、电源模式与温度。 |
| 内存 | 16 GiB 双通道 DDR4；记录速度及开测可用内存。 |
| 磁盘 | 本地 NVMe SSD，NTFS，保留≥20% 空间；记录型号、固件及 BitLocker 状态。 |
| GPU/显示器 | Intel UHD 730 或明确记录的等效集显；1920×1080、60 Hz、100% DPI。 |
| OS | Windows 10 22H2 x64，完整 build/补丁号记录。Win10 1809 x64 单独做最低基线兼容性测试，不混合统计。 |
| 应用 | texzi Release，准确 commit、NSH `224d9a7…`、工具链、Cargo.lock hash、features、优化参数、字体版本。 |
| 系统状态 | 接交流电，默认平衡电源，关闭节能；Defender 保持通常配置；无正在运行的构建/下载/同步基准样本。 |
| 文件位置 | 同一本地卷、固定路径；未压缩、非网络盘、非云占位文件。 |

这是一台**建议参考机，不是用户已确认的电脑**。次级探索机建议四核 CPU、8 GiB RAM、SATA SSD；用户实际设备结果另表报告。Win11 x64 可做兼容性 smoke，但不取代 Win10 验收，也不新增其他平台承诺。远程桌面、虚拟机、核显/独显、省电模式、125/150/200% DPI、120/144 Hz 屏幕都应分别标注，不能混进 60 Hz 主数据。

执行前记录 `environment.json`：OS、硬件、驱动、显示刷新率/DPI、电源、应用/依赖版本、字体、杀毒状态、locale、输入法版本、样本 hash。测试运行期间记录 CPU 温度/频率及后台总 CPU/磁盘活动；预定义干扰阈值。若异常需要剔除，保留原始行、剔除理由和完整重复测量，不在看到慢结果后随意删点。

### 2.2 构建与运行状态

必须用优化 Release 包直接运行 `texzi.exe`；不开 dev server、热重载、调试器和详细正文日志。性能事件采样成本须测开/关对照，建议开销≤2%。NSH `--fps`/`NSH_FPS=1` 可辅助定位 tick/paint/present 阶段，但其每秒统计、平均绘制耗时及空闲 FPS 都不等于输入端到端延迟。[NSH Windows 性能监视说明](https://nsh.dev/docs/windows/)、[官方性能报告及环境](https://nsh.dev/docs/performance-report/)。

## 3. 可重复样本生成与 manifest

随包提供可运行的 [generate_fixtures.py](../tools/bench/generate_fixtures.py)，仅用 Python 3.10+ 标准库，不访问网络。它使用明确的 LCG32：`state=(1664525*state+1013904223) mod 2^32`，默认 seed `20261115`，编码与换行显式指定。每份 manifest 写入磁盘字节数、SHA-256、编码/BOM、逻辑行数、最大行字节数/Unicode scalar 数、EOL 计数、生成器版本/代码 hash、适用的结构元数据。MiB 始终为 1,048,576 bytes。故意无效编码样本的语义行统计为 null，并明确要求严格解码失败；不编造解码后行数。

生成器 `smoke` 为默认：TXT-1、MD-1 及小型长行/匹配/regex/编码/EOL/Unicode/原文保真语料，共约 2.2 MiB；`standard` 增加 TXT-10/100、ASCII-10/100、MD-10/MD-100-SOURCE、LONG-1M、MATCH-HIGH-10/100，共约 443 MiB；`stress` 再增加 LONG-100M 与 MIXED-LONG-100M，共约 643 MiB。样本写在 manifest 相邻的 `texzi-fixtures/`，不要放入最终设计交付 ZIP。

**下表是完整测试计划，不是生成器已全部覆盖的声明。** 当前未实现 MD-COMPLEX 深嵌套/巨表、HUGE-1G、THRESHOLD 精确边界组及图片/网络资源样本，manifest 的 `coverage.not_implemented` 同步列明。Markdown 的块数/表格信息来自确定模板；真实解析器节点数、实际最大布局块必须由未来原生测试补充，不用模板估计冒充已经解析验证。生成器不启动 Windows 应用，不测 IME、帧率或任何产品性能。

生成算法采用确定性记录模板循环，而不是只重复单一字符：按模板编号与固定随机数产生文本、数字、标点，逐完整 Unicode scalar 和完整 EOL 写入；接近目标长度时使用 ASCII 尾段补齐字节数，禁止截断 UTF-8/UTF-16 序列。源样本与编码变体的**磁盘尺寸可能不同**，必须分别标明，不能都叫“10 MiB”却只按字符数估算。

| 样本 ID | 内容与规模 | 作用与目标归属 |
|---|---|---|
| TXT-1 / TXT-10 / TXT-100 | 精确 1/10/100 MiB UTF-8 无 BOM；中英混合，约 60% 内容为汉字；每行约 40–160 字符，最大≤4 KiB，CRLF；包含标点/tab/空行。 | 主普通文本样本；启动、打开、输入、滚动、保存。 |
| ASCII-10 / ASCII-100 | 同规模英文/数字日志，平均行长约 120 bytes，LF；非单一重复文本。 | 排除只优化中文或极高压缩比样本。 |
| MD-1 / MD-10 | 1/10 MiB；标题、段落、引用、任务列表、代码块、简单表格、链接；<50,000 块、单块<64 KiB、单表<1,000 cells；无网络资源。 | 正常 Markdown 源码与真正 WYSIWYG 的 P0 输入/滚动；同一文件分别两模式。 |
| MD-100-SOURCE | 100 MiB Markdown，普通行长，首尾均有有效结构。 | 源码 100 MiB 开启目标；富编辑为探索，不作为无条件同值承诺。 |
| MD-COMPLEX | 1/10/25 MiB，各含深层列表、长引用、20,000-cell 表格、引用式链接、未闭合 fence 等独立子样本。 | 找出结构复杂度瓶颈；局部编辑、取消、模式切换、保真。 |
| LONG-100K / LONG-1M / LONG-100M | 单行 100 KiB/1 MiB/100 MiB；另有 100 MiB 文件中混一条 1 MiB 行。 | 长行布局、高亮退化、横向滚动；不能与“普通行长”合并报达标。 |
| MATCH-HIGH | 10/100 MiB，每 8–32 字节有一次匹配；包含相邻和零宽匹配场景。 | 匹配分页、总计数、装饰上限与 Replace All 资源预算。 |
| REGEX-ADV | `(a+)+$` 对长 `a…ab`、高回溯分支、长 lookbehind、无效 regex、跨行模式等固定集。 | 超时/取消/错误分类；不要求病态模式 2 s 内完成成功检索。 |
| ENCODING | 同语义文本的 UTF-8 BOM、UTF-16 LE/BE、GB18030/GBK/1252；含不可编码字符与无效字节变体。 | 严格解码、提示、无损保存；不把不同编码结果合成一个平均。 |
| THRESHOLD | 20 MiB 与 200 MiB 各生成阈值前1 byte、恰好阈值、阈值后1 byte；长行分别19,999/20,000/20,001 Unicode scalar。 | 证明大文件/超大文件从≥20/200 MiB触发，长行保护从≥20,000 Unicode scalar触发；不混用字节、UTF-16单位和字符数。 |
| EOL-MIXED | CRLF/LF/CR 按固定序列混合；有无末尾换行；非空最后行。 | 原字节保留、局部编辑、转换换行、撤销。 |
| UNICODE | emoji ZWJ、肤色、组合重音、代理对对应字符、CJK 扩展汉字、制表符。 | 字节/UTF-16/grapheme 映射、光标及 IME 正确性。 |
| HUGE-1G | 1 GiB 普通行长、独立超长行变体。 | 探索资源极限/失败路径，不承诺 100 MiB 相同打开时间。 |

主样本中的普通文本不含足以使机器进入换页风暴的其他进程负载；压力负载是另一组实验。Markdown 本地图片样本固定尺寸/压缩格式并预先生成；远程图片另测加载失败/慢响应，不用网络波动解释纯文本输入超标。

## 4. 时间戳、Trace 与统计计算

### 4.1 事件契约

使用同机高精度单调时钟 `QueryPerformanceCounter`，启动器与应用记录频率并核验时间轴。跨系统 wall clock 不用于微秒/毫秒差值。[微软 QPC 指南](https://learn.microsoft.com/en-us/windows/win32/sysinfo/acquiring-high-resolution-time-stamps)。

| 事件 | 准确定义 |
|---|---|
| `launch.request` | 测试启动器调用 CreateProcess 前；带 launch_id。 |
| `app.init` | 应用入口第一条可记录语句；与上一事件的差值是进程装载部分。 |
| `window.first_present` | 窗口首帧呈现，可能尚无正文；不能替代 ready。 |
| `editor.ready` | 活动文档模型与输入路由已就绪，正确正文帧已 present；注入编辑探针验证成功。 |
| `document.open_request` | 应用收到已解析路径/编码意图的打开命令；文件选择对话框的人为等待不计入。 |
| `document.first_text_present` | 首屏真实文本、行号/块布局正确，内容 hash/文本探针与 fixture 相符。 |
| `input.received` | WM_CHAR、按键命令或 TSF commit 到达 texzi 输入适配的最早位置。 |
| `transaction.committed` | DocumentStore 发布该版本及 delta。 |
| `revision.presented` | 第一个包含该 revision 相关编辑范围的可见帧实际提交呈现；不能只用 request_repaint。 |
| `search.request/first/complete/cancelled` | 同 query_id/revision，依次为请求、首可用结果、扫描完、取消已生效。 |
| `save.request/flushed/replaced/verified` | 捕获快照、临时文件flush、替换完成、版本核验完成。 |
| `recovery.queued/flushed` | journal 入队及系统 flush 返回，带最后完整事务号。 |

应用层事件用 TraceLogging/ETW 或低开销二进制环形缓冲写出；不记录正文、搜索词或真实用户文件路径，只写测试 fixture ID/长度/版本/范围长度。这个统一最小日志策略不依赖内容识别。关联 `frame_id`、`revision`、`view_id`，避免把另一窗口的 present 当作当前输入已显示。

用 PresentMon/ETW 校准真实 present 时间并保留原始采集；固定工具版本和捕获参数。不同渲染路径是否能精确关联 frame_id 要在 T01 先证实，无法精确时明确列为“应用标记近似值”，不得用 CPU paint 时间代替。对 Microsoft Pinyin 和普通键入各录至少 30 次 240 fps 视频抽样，报告视频 4.17 ms 的时间分辨率及测量误差。[PresentMon 官方仓库](https://github.com/GameTechDev/PresentMon)。

### 4.2 重复次数与百分位

| 试验 | 最少样本数 |
|---|---|
| 热启动 | 5 次预热不计，随后 100 次；至少分 3 批。 |
| 冷启动 | 30 次独立重启后的首启；每次记录启动后稳定等待条件。 |
| 文件打开/切换/保存 | 每个模式/规模/缓存条件 30 次。 |
| 普通输入 | 每个场景 5 轮，每轮≥1,000 个独立输入事件；主速率 10 次/秒，另测 30 次/秒压力输入。 |
| 滚动 | 每个场景 5 轮、每轮 60 s；包含慢速、快速和往返段，分别汇总。 |
| 搜索/替换 | 每个 query/样本至少 30 次；病态 regex 每项至少 10 次 timeout/cancel。 |
| 内存泄漏/恢复 | 50 次文档开关；故障点各至少 10 次，断电类用可重置测试环境单列。 |

百分位采用 nearest-rank：将有效值升序排列，`p95 = a[ceil(0.95*N)-1]`，`p99 = a[ceil(0.99*N)-1]`，单位统一 ms。每轮先计算，再给出合并事件值；**每轮均应通过**，不能用其他快轮掩盖某一轮卡顿。报告 p50/p95/p99/max/N、超阈值比例，启动可附原始散点；小样本 p99 不稳定时明确说明，不通过插值制造精度。

崩溃、冻结、任务未完成、无对应 present 都记为失败，不从延迟分布悄悄消失。超时可记 censor 标记与下界，不当成刚好 2,000 ms 的成功结果。UI 正确性失败即该场景失败，即使时间更快。不得把纯文本、WYSIWYG、不同大小/硬件/DPI 的样本池化后只报一个 p95。

## 5. 启动、打开与恢复场景

### 5.1 启动场景

- **S0 空白会话**：一个窗口、一个空临时文档、默认主题，设置和字体均已按发行包就位。
- **S1 日常恢复**：一个窗口、两个编辑组、共 5 个标签；活动文档为 1 MiB 普通 Markdown WYSIWYG，其余为 1 MiB 文本/代码；侧栏一个 1,000 项文件夹树。非活动正文允许延迟载入，标签和恢复顺序须正确。
- **S2 重会话**：两个窗口、8 视图，其中含 10 MiB 与 100 MiB 文档；作为额外压力场景，显示恢复进度，各文件打开目标另计，不把 S0 目标无条件套用。
- **S3 崩溃恢复**：含至少 2 个未命名文档、1 个外部已修改的文件、混合换行与中文未保存文本；先恢复活动文档，时间与数据正确性同时报告。

建议已接受启动目标同时用于 S0 和 S1，作为具体化的首版验收范围；S2/S3 要给完整数据及可交互进度，不能用简化空窗口成绩代表所有会话。

热启动前确认主进程与后台 worker 已退出，保留 OS 文件缓存；关闭后 2 s 等待进程清理结束即可，不用常驻托盘作弊。冷启动使用真实重启后的首次启动、默认预取机制和相同开机后稳定等待规则；例如 CPU/磁盘背景负载连续 10 s 低于记录阈值后开测。重启不保证每个页绝对未缓存，因此报告名称为“重启后首次启动”。只清应用状态/只结束进程的测量叫“新进程、缓存热”，不得标成冷启动。虚拟机快照冷启需要记录宿主缓存局限，不能替代主物理机结果。

### 5.2 100 MiB 打开探针

依次测已缓存和首次读取两组；从空闲应用发出打开命令。首屏后验证首行、中间行以及尾部探针，5 s ready 前不得把输入吞掉或永久丢弃。ready 后立刻在文档中段插入 `测试🙂`，Undo，再 Save As 到新测试文件，与原始 fixture 比较原字节；随后导航到末尾、查找跨 chunk 文本。若前缀可见但全文搜索/保存无法正确完成，ready 事件不合格。

从 Explorer/命令行启动并打开 100 MiB 的总体时长额外报告，起点为 CreateProcess；它包含启动及文件打开，不能和已运行应用的 open 指标混为一谈。

## 6. 输入、滚动与多视图实验

### 6.1 输入矩阵

每个 TXT-1/TXT-10、MD-1/MD-10 的两模式在文件前部、中部、尾部执行：普通 ASCII/汉字提交、Backspace/Delete、选区替换、回车、1 KiB/100 KiB/1 MiB 粘贴、Undo/Redo。粘贴是独立任务延迟，不并入单字 p95。分别测高亮开启/关闭定位成本，**正式达标使用正常默认设置**，不能关掉产品默认能力才通过。

源码/纯文本增加 10/100/1,000 光标实验；1,000 光标属于压力场景，报告事务成本，防止每光标触发一次全文 onChange。正式 IME 至少覆盖 Pinyin 中英混输、长拼音串、候选选择、Esc 取消、表格 cell 和代码块、组合中切窗口/模式。通过 OS IME 实际组合验证“preedit 不落正文，commit 一次 history”；输入已组成的 Unicode 字符只能测试提交路径。

建立同 DocId 的 4 个视图、分布在 2 个窗口，分别滚到不同位置；在视图 A 输入，验证其他视图只有相关区域更新且选区/滚动不被抢走。再次测 active input 延迟和总内存，验证共享文档真正避免全量复制。

### 6.2 滚动协议

60 Hz 固定显示器下，按预先保存的滚轮/触控板/滚动条轨迹回放：慢滚 15 s、快速连续 15 s、反向 15 s、拖动到远处并继续滚 15 s。设置相同窗口尺寸、字体、行高、自动换行和缩放。分别测 TXT-1/10/100、MD-1/10 富编辑及源码；长行和巨表单列。

在连续视觉移动区间统计相邻 present 的时间差，包含跳帧形成的 33.3/50 ms 间隔。若渲染合并跳过中间帧，也必须保留用户看到的较长间隔，不能只统计成功绘制的一帧内部耗时。报告 p95、p99、>33.3 ms 比例、最大冻结时间、布局/paint/文本 shaping 时间、输入队列等待；只在预定义的停顿区间不计滚动分布。

每轮核验内容与命中：行号不漂移、没有空白裂缝/重复行、选择高亮跟随、回到原锚点误差≤1 个可见行；富编辑图片高度变化时 scroll anchor 保持。滚动过快而少画内容导致时间漂亮仍判失败。

## 7. 搜索、替换、超时与取消

查询集包含：文件头/中/尾唯一命中、无命中、大小写变化、中文全字、跨 Rope chunk、跨行 regex、捕获组替换、零宽、MATCH-HIGH、大量不存在的 named group、未参与组、非法模式。普通正文 projection 检索和 Markdown 源码检索分别测，切范围后旧结果必须立即失效。

建议 PCRE2 初始保护参数为 match limit 1,000,000、depth limit 1,000、heap limit 32 MiB；这些为待 Gate 校准的引擎预算，不保证对应固定时间，也不等于 subject 总内存。专用进程总内存建议 512 MiB；引擎版本/JIT、输入大小、编译期和匹配期都需测试。超限错误分清“格式错误”“匹配预算”“内存不足”“取消”“截止时间”，不统称无结果。[PCRE2 限制和 callout API](https://www.pcre.org/current/doc/html/pcre2api.html)。

从 query 发出到 UI 反馈、首结果、完整结果分别计时；从 Esc/取消按钮事件到所有旧结果不再更新及 worker 停止计取消时间。硬预算由父进程 watchdog 保证；不能只指望正在回溯的同线程检查取消标记。查询编辑连续输入时，旧任务取消、版本 token 失效，新查询不会被旧结果覆盖。

Replace All 必须测 0、1、1 万、100 万匹配。预先核验替换数与完整候选版本，再单事务提交；在 10%、50%、90% 规划进度取消，原文 hash/dirty/undo 均不变。并行在另一窗口修改同一文档，旧计划必须被拒绝并重新检索，不能覆盖新修改。结果过大时测磁盘暂存及资源错误，不允许部分替换后声称取消成功。

## 8. 保存、恢复、内存及长时稳定性

保存对 UTF-8、UTF-16、GB18030、混合 EOL 分别测，输出 hash 与预期字节比较。保存总时间包含 flush/替换/核验；若只统计 write 系统调用返回，要标为分段指标。测试过程中模拟磁盘满、只读、共享锁、外部竞争写入、临时文件写完但尚未 replace、replace 后尚未更新恢复清单等故障点。

恢复测试先记录已接受事务序号与最后 flush 序号，在每个故障点终止进程，重新启动比对内容、编码/EOL、未保存标记、各视图模式/位置。已 flush 的完整事务不得丢失；最后一个不完整 journal 记录不能让整个文档恢复失败。关机断电模拟、进程 kill 与正常退出是不同故障类型，结果分开报告。

正常关窗的静默保留另测 `session.restoreOnStartup`、`session.restoreUnsaved`、`snapshotFlushSucceeded` 的组合：仅三项同时为 true 可不显示未保存确认，其余组合必须走保存/放弃/取消；显式关闭最后一个 dirty 标签始终提示。外部变更分别测试默认 `reloadClean` 和 `ask`：后者即使 clean 也必须先提示，不能因自动重载更快就绕过用户设置。

内存采样建议每 100 ms 取进程 Private Bytes/Working Set/Private Working Set，每 1 s 取系统可用内存、提交量、硬页错误和 GPU 占用。打开后 10 s、编辑 60 s、执行搜索/保存峰值、关闭文档后 30 s 各取点；所有 texzi worker 纳入总和。为 OS 页面缓存、allocator 保留区和 UI 缓存分别解释，不能凭 Working Set 暂时下降宣称内存已释放。

长时试验运行 2 h，循环编辑、切模式、多视图、搜索、保存与外部修改；每 10 min 记录稳态资源，最后保存并校验内容。另测 50 次开关文档和窗口、显示器/DPI 改变、睡眠唤醒。只在发现具体增长/卡顿风险后延长到 8 h，不进行没有判定价值的无限 soak。

## 9. 输出证据与回归门槛

每次正式运行至少交付：

1. `environment.json`、fixture manifest、二进制及依赖 hash。
2. `events.jsonl`/ETW 原始事件、PresentMon 输出、各 trial 的成功/失败/剔除原因。
3. 按场景分组的 `metrics.csv`、p50/p95/p99/max、样本数、内存/CPU/磁盘数据。
4. `report.md`：目标与结果并排；标注未测/失败/通过，不把“未测”显示成绿色。
5. IME、混合 DPI、100 MiB 编辑探针的短录屏；故障恢复前后 hash/事务序号。

建议事件行格式如下；示例数字仅说明格式，不是测量结果：

```json
{"schema":1,"run_id":"example-only","trial":1,"fixture":"TXT-10","mode":"source","event":"revision.presented","doc_revision":42,"view_id":"v1","qpc_ticks":123456789,"success":true}
```

CI 在 Windows x64 Release 上运行文档/编码/Markdown/搜索正确性测试及小样本性能 smoke。正式主性能机每个候选 Release 执行完整计划；日常提交比较固定样本，若 p95 退化>10% 且绝对增加>3 ms，或内存>10%，进入调查，不自动用统计波动阻断所有工作。数据丢失、共享历史错误、IME 重复/丢字、严重 UI 冻结和任一已接受指标失败阻断发布；仅新增建议预算失败需记录原因、影响及修复计划，不伪装为用户重新批准了较差目标。

## 10. 已提供工具与未来实施接口

以下生成命令已由本包提供，可直接运行；建议 manifest 放在交付包目录之外：

```powershell
python tools/bench/generate_fixtures.py --profile smoke --manifest ../texzi-bench-data/smoke/manifest.json --seed 20261115
python tools/bench/generate_fixtures.py --profile standard --manifest ../texzi-bench-data/standard/manifest.json --seed 20261115
```

已对 smoke 执行两次同 seed 生成，核验清单与文件 hash 可复现、尺寸/行统计一致、有效样本严格解码成功、故意无效样本按预期拒绝：18 文件、2,298,632 bytes。standard 已完成生成和独立校验：27 文件、464,720,648 bytes；stress 与 standard 共用的27份记录完全相同，额外两份100MiB长行样本也通过尺寸/hash/严格解码检查，总计29文件、674,435,848 bytes。这里只证明生成工具行为，不是 texzi 原生性能实测。

下列 Windows runner/report 入口**尚未提供**，属于实施团队需要创建的接口约定：

```powershell
cargo run -p xtask --release -- bench --scenario startup-warm --trials 100 --output artifacts/perf
cargo run -p xtask --release -- bench --scenario open-100m --trials 30 --output artifacts/perf
cargo run -p xtask --release -- bench --scenario input-scroll --manifest fixtures/manifest.json --output artifacts/perf
cargo run -p xtask --release -- bench --scenario search-cancel --output artifacts/perf
cargo run -p xtask --release -- bench --scenario recovery-faults --output artifacts/perf
python tools/bench/report.py artifacts/perf --targets tools/bench/targets.json
```

冷启动需要跨重启调度和结果累积，不提供一个会未经说明自动重启用户电脑的默认命令；测试脚本只在专用测试机显式启用该模式。报告器必须按本文件 nearest-rank 计算，保留失败 trial，输出目标适用的模式/样本范围。脚本、样本生成器及参数与产品代码一起版本化，避免“换了样本就变快”而无人察觉。
