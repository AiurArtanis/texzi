# texzi 参考产品核实：quiet.md

核实日期：2026-11-15。对象是 **s1dashu/quiet-md 的 0.1.12**，固定 commit `874763d2c9fd19204db727ddc0bcc6f5742e8970`；不混用同名项目或后续分支。本文记录一手文档与源码核实，未声称完成 Windows 真机试用。[版本源码][Q0]

## 1. 实际能力及 Windows 边界

这是 Rust + GPUI 的原生富文本应用，正文以可编辑块呈现，包含标题、列表、任务、表格和代码等结构，能够作为所见即所得交互参考。0.1.12 已支持直接打开本地 `.md`、`.markdown`、`.mdown` 并写回原位置；“导入到文档库”会另建副本，“从列表中移除”不删除原文件，不能把打开与导入描述成同一动作。[README][Q1]、[本地文件入口][Q2]

该版本列出的 Windows 包是 **x64**，要求 Windows 10 2004 或更新版本；没有据此确认 x86 支持。texzi 已选择 x64，可以参考其窗口及文件体验，但它的支持声明不能证明 texzi 的 Windows 10 1809 工程基线可用。上述源码亦不足以确认一个满足 texzi 契约的“源码/富编辑双向切换、同步滚动、跨窗口共享撤销”现成组件；这些能力仍由 texzi 自己实现验收。[Q1]

## 2. 文档模型与可复用范围

内存模型是 `Document / Block / RichText` 块树，文档库使用真实文件夹，保存 `.xml` 或 `.md`。不能只依据架构文档开头的 DocxXML 描述，误称新版只保存 XML。编辑 `Core` 持有文档、选区和历史；`Snapshot` 保存文档及选区，撤销/重做恢复两者。视图调用编辑内核，按操作决定追随光标或保持视口，这种职责区分值得借鉴。[Q1]、[架构][Q3]、[编辑内核][Q4]

`crates/core` 的包名为 `fdl_core`，确实不依赖 GPUI，可独立测试；但它提供的是该产品的块模型、格式和编辑内核，**不是 NSH 控件**。窗口、绘制、焦点和输入桥接仍在 GPUI 应用层。移植需适配 texzi 的源文本权威模型、事务、每视图选区、IME 和布局，不能因为同用 Rust 就直接接入。整篇文档快照历史的时间与内存成本也不能直接套用到 100 MiB 文本。[核心包声明][Q5]、[Q3]、[Q4]

## 3. Markdown 保真的具体风险

`crates/core/src/markdown.rs::write_file_with` 先产生规范化结果 `canonical`；存在原文时，依次尝试较宽松及严格的 `preserve`，保留可对应的旧块和间距，再用 `reads_as` 检验解析语义。两次失败便回退到完整 `canonical`。这是真实的原貌保留努力，却不是“任何情况下未改字节都不变”的保证。[Markdown 写回实现][Q6]

因此不能把该函数直接当作 texzi 模式切换器：仅切换视图就导出再导入，可能重写空白、标记或邻接结构。texzi 应继续以源文本为唯一权威，保留未知语法的原样片段；富编辑仅提交必要源区间的补丁，无法安全映射时提供源码入口。可以借鉴其“候选写回后重新解析验证”的思路，但不能照搬全篇规范化回退作为成功路径。此段是基于源码的设计判断。[Q6]

## 4. 性能数字的适用范围

仓库性能报告采用 **Apple M5 Pro、48 GiB、macOS 26.5**，1280×800 pt、2×缩放，测试源码为 `0e92b51`，不是本次固定版本的 Windows 结果。样本为 1,001,307 字符、549 章；连续输入、退格、滚动各 100 次，帧处理 p95 分别为 5.78、5.55、4.14 ms。测量同步调用 `window.draw` 的应用内处理时间，明确不含屏幕呈现延迟。[性能报告][Q7]

这些数据支持研究块虚拟化和局部重测量，不能作为 texzi 的启动、10 MiB 中文输入或 100 MiB 打开验收；字符数也不能直接当 MiB。texzi 必须按自身 Windows 环境和 input-to-present 边界重新测量。

## 5. 许可与最终采用建议

代码根许可为 MIT，复用代码需保留适用声明并检查第三方依赖。README 明确排除 **67 个 Nucleo 图标、Quiet.md 名称与应用图标**；图标目录另有商业许可约束，不能随代码许可一起打包为 texzi 素材。[代码许可][Q8]、[图标许可][Q9]、[Q1]

建议采用的是直接编辑正文、轻量上下文操作、选区与撤销一起恢复、局部更新和明确区分打开/导入的经验。首版不把 quiet.md 整体引入为运行时依赖；如复用算法，须先用独立适配实验验证原文保真、多视图与中文输入，再纳入依赖清单。Typora、Obsidian 在本方案仅作为用户指定的交互参照，本附件不追加未经核实的功能或集成声明。

## 固定一手来源

[Q0]: https://github.com/s1dashu/quiet-md/tree/874763d2c9fd19204db727ddc0bcc6f5742e8970 "quiet.md 0.1.12 固定源码"
[Q1]: https://github.com/s1dashu/quiet-md/blob/874763d2c9fd19204db727ddc0bcc6f5742e8970/README.md "实际功能、平台、版本记录与许可例外"
[Q2]: https://github.com/s1dashu/quiet-md/blob/874763d2c9fd19204db727ddc0bcc6f5742e8970/src/local_files.rs "本地Markdown打开、导入与移除语义"
[Q3]: https://github.com/s1dashu/quiet-md/blob/874763d2c9fd19204db727ddc0bcc6f5742e8970/docs/ARCHITECTURE.md "块模型、Core、历史与虚拟化"
[Q4]: https://github.com/s1dashu/quiet-md/blob/874763d2c9fd19204db727ddc0bcc6f5742e8970/crates/core/src/editor/core.rs "Snapshot、checkpoint、undo、redo"
[Q5]: https://github.com/s1dashu/quiet-md/blob/874763d2c9fd19204db727ddc0bcc6f5742e8970/crates/core/Cargo.toml "fdl_core版本与依赖声明"
[Q6]: https://github.com/s1dashu/quiet-md/blob/874763d2c9fd19204db727ddc0bcc6f5742e8970/crates/core/src/markdown.rs "write_file_with、preserve、reads_as及canonical回退"
[Q7]: https://github.com/s1dashu/quiet-md/blob/874763d2c9fd19204db727ddc0bcc6f5742e8970/docs/PERFORMANCE.md "原始性能环境、版本、指标边界与结果"
[Q8]: https://github.com/s1dashu/quiet-md/blob/874763d2c9fd19204db727ddc0bcc6f5742e8970/LICENSE "MIT代码许可"
[Q9]: https://github.com/s1dashu/quiet-md/blob/874763d2c9fd19204db727ddc0bcc6f5742e8970/assets/icons/LICENSE-nucleo "Nucleo资源许可"
