# TX-00 环境预检（PARTIAL）

本记录不是实现候选或验收结论。当前任务要求先恢复 Rusty 完整任务单，尚未满足；未修改产品代码、分支或默认分支，未提交、推送、触发 CI、发布。

## 已核对

- 工作目录：`D:\Work\texzi`。
- HEAD：`7f84e9d7ce33ba51474e08f7a4c3c8958b79d333`；分支 `main`。
- 初始 `git status --short`：仅 `?? docs/`；`git diff --exit-code` 退出码 0。
- 现有 worktree：`.cindy-worktrees/fierce-hertz`，分支 `cindy/fierce-hertz`，同一 HEAD；未改动。
- 仓库根目录未发现 AGENTS.md；读取并遵守当前请求提供的 AGENTS.md。CONTRIBUTING.md 的直接提交 main 规则与本次禁止修改默认分支要求冲突，本次要求优先。
- 无 `.codegraph/`；未创建索引。此为既有项目续接，不追溯初始化 Trellis。
- PowerShell `7.7.0`；`git version 2.56.1.windows.1`；OS description `Microsoft Windows 10.0.26300`。未验证 Win10 22H2/1809。
- `Get-Command` / `where.exe` 未找到 rustc、cargo、gh。另查用户 `.cargo/bin`、`.local`、`Programs`、`AppData/Local/Programs`、`C:/Program Files/GitHub CLI`、`C:/Program Files/Rust stable MSVC 1.0`、`C:/tools`、`D:/Tools`，未发现对应可执行文件。不等于证明全盘未安装；未安装或更新任何工具。
- `docs/design-v1/manifest-sha256.json` 的 50 项逐一核对存在性、字节数及 SHA256：差异 0，PowerShell 命令退出码 0。保留全部原有未跟踪资料。

## 任务来源缺口

`get_chat_history` 已成功读取来源会话 `1c749ae3-42a6-428a-8cd4-14e4df049eba`，返回 hasMore=false；仅本次重试轮记录及截断的历史摘要，没有 Rusty TX-00 原始任务单。按工作目录读取历史也只返回当前任务。群历史能力搜索未暴露额外完整群历史接口。

本地任务单可读，载明固定 NSH `224d9a795c7ac9d10d75750de9351adac5549675`、设计导入/勘误、workspace/lockfile/工具链/features/许可、原生窗口及 G0；尚未与 Rusty 原文核对。本地条目没有明确 Release CI 触发、产物与候选分支规则。已请求 Rusty 补充原文，或确认以本地 TX-00 + HANDOFF T00/G0 为准并明确 CI 要求。未擅自补方案。

来源文件 SHA256：

- `docs/2026-11-16-texzi-开发与验收任务单.md`：`40E7752F4BF974CF2F1E3E627D1BE0CB8528569459C5468DAF641AA5BECD4104`
- `docs/design-v1/docs/HANDOFF.md`：`CEACA70D35CC833BBA2F7F8B7699F7120EE1C6F69F23E2D34819B36BC9F20193`

## 验证边界与下一步

静态预检 PARTIAL；构建、组件测试、真实原生窗口运行、人工验收、CI 产物下载与核验均 NOT_RUN。无 Release 程序 hash，无实现 commit。不以设计包旧测试结果作为本次证据。

收到完整任务确认后，在候选分支继续；核实 NSH 实际源码/API/features/许可和工具链，执行实现、构建与测试，并下载 CI 产物核验。Windows 10 环境缺口独立记录。
