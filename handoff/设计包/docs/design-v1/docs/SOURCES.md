# texzi 事实来源与核实口径

核实日期：2026-11-15。以官方项目、官方文档、固定版本源码和微软Windows API文档为依据。产品行为与数值建议属于本方案设计；不能借某一来源把自己的推断写成上游保证。

## 1. 固定版本

| 项目 | 固定版本 / commit | 用途 |
|---|---|---|
| NSH | v0.5.1 / 224d9a795c7ac9d10d75750de9351adac5549675 | Windows后端、Editor、RichEditor、Dock与Window API审读 |
| VS Code | 1.146.0 / 69047191fc427f2f92828279aae1d90d8eeda7ab | Windows默认快捷键、上下文和冲突对照 |
| quiet.md | v0.1.12 / 874763d2c9fd19204db727ddc0bcc6f5742e8970 | 原生富编辑、本地Markdown与集成边界参考 |

## 2. 关键结论的直接来源

| 结论 | 一手来源 | 证据限度 |
|---|---|---|
| NSH的Windows后端是Win32 + Direct2D + DirectWrite；要求Win10 1809+ | [NSH Windows文档](https://nsh.dev/docs/windows/)、[固定后端说明](https://github.com/longbridge/nsh/blob/v0.5.1/docs/dev/windows.md) | 文档支持范围，不是texzi真机测试 |
| v0.5.1为本轮固定基线 | [官方Release](https://github.com/longbridge/nsh/releases/tag/v0.5.1)、[完整commit](https://github.com/longbridge/nsh/commit/224d9a795c7ac9d10d75750de9351adac5549675) | 固定证据，未来版本需重新审核 |
| Editor有Rope、可见行和编辑功能；受控回调与搜索存在边界 | [Editor文档](https://nsh.dev/elements/editor/)、[固定设计说明](https://github.com/longbridge/nsh/blob/v0.5.1/docs/dev/editor.md)、[state.rs](https://github.com/longbridge/nsh/blob/v0.5.1/crates/nsh-base/src/editor/state.rs) | 不能自动推出100MiB、完整Unicode或所有性能目标达标 |
| RichEditor已有原生富编辑模型/事务/Markdown能力 | [RichEditor文档](https://nsh.dev/elements/rich-editor/)、[固定设计说明](https://github.com/longbridge/nsh/blob/v0.5.1/docs/dev/rich-editor.md) | 不是只读TextView；但仍需产品适配 |
| Markdown格式转换不保证无损 | [固定MarkdownFormat实现](https://github.com/longbridge/nsh/blob/v0.5.1/crates/nsh-base/src/rich_editor/formats/markdown.rs) | is_lossless_for返回false；局部源码桥接是texzi设计方案 |
| NSH Windows输入桥接需进一步真机验收 | [text_input.rs](https://github.com/longbridge/nsh/blob/v0.5.1/crates/nsh-platform-windows/src/text_input.rs) | 源码存在不等于微软拼音/搜狗/高DPI已通过 |
| quiet.md确为Rust+GPUI富编辑，有Windows x64发布 | [项目官方说明](https://github.com/s1dashu/quiet-md)、[v0.1.12发布](https://github.com/s1dashu/quiet-md/releases/tag/v0.1.12) | 是完整应用，不是NSH可直接集成组件 |
| quiet.md性能结果不能代替texzi测试 | [官方性能说明](https://github.com/s1dashu/quiet-md/blob/v0.1.12/docs/PERFORMANCE.md) | 测试机为macOS设备；帧处理时间不等于Windows端到端输入延迟 |
| VS Code参照版本为1.146.0、2026-11-11发布 | [官方Release](https://code.visualstudio.com/updates/v1_146)、[官方tag API](https://api.github.com/repos/microsoft/vscode/git/ref/tags/1.146.0) | 固定命令源码与动态快捷键PDF区分使用 |
| PCRE2支持的语法、限制与callout接口 | [pattern](https://www.pcre.org/current/doc/html/pcre2pattern.html)、[API](https://www.pcre.org/current/doc/html/pcre2api.html) | 实施前锁实际版本；match/depth limit不等于时间保证 |
| Windows DPI与保存API的行为 | [高DPI开发指南](https://learn.microsoft.com/en-us/windows/win32/hidpi/high-dpi-desktop-application-development-on-windows)、[ReplaceFileW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-replacefilew) | 文件系统、权限、链接和失败竞争仍要故障注入 |

## 3. 快捷键证据

COMMANDS.md与commands.json包含37条来源，优先用固定commit下的命令注册与平台条件，动态官方快捷键表/PDF仅作交叉核对。表内VS Code默认键已完成源码核实；无默认绑定写Unassigned，texzi自己新增/改键标suggested。

源码核实不等于Windows运行时导出、不同键盘布局和IME组合测试。实施Gate仍要在固定VS Code二进制导出默认键，比较文本/Markdown/输入框/菜单上下文。

## 4. 推断与选型的标记

quiet.md 的详细源码核实与采用判断见 [REFERENCE_RESEARCH.md](REFERENCE_RESEARCH.md)。它区分固定版本文档、动态主页和实际测量环境，记录原文保留失败后的规范化回退风险。

以下均是设计判断：Rust应用路线、CST/SourceMap局部patch、独立DocumentStore、PCRE2搜索服务、20/200MiB策略、恢复周期/预算、最多8组和性能参考机。依据上游能力提出可行路径，但尚无texzi生产实现测量结果。

上游文档会变化。固定commit链接优于main；在线文档若与固定源码有差异，以固定源码审读记录并在技术Gate澄清，不把差异藏起来。data/source-index.json收集文档中的来源链接及命令来源元数据，方便实施团队继续核对。

## 5. 许可与素材边界

NSH官方仓库为Apache-2.0；实施团队仍需保留适用声明与依赖许可证。quiet.md代码许可与商业图标/品牌不是同一授权，不复制其品牌或专有图标；本包没有打包quiet.md代码或图标。[quiet.md许可说明](https://github.com/s1dashu/quiet-md#license)

概念图作为本项目视觉参考放入concepts。原型图标为本包内的简单线条绘制。手册构建使用随包Marked，其许可证保留在tools/vendor/marked-LICENSE.md；该构建工具不决定未来原生应用依赖。
