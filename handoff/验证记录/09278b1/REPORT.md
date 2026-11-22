# 09278b1 · Paladin 开发侧复验

**FAIL**。定稿要求保留「编辑 → 替换」，候选在 100%／150% 明暗四组均缺此菜单项。另两个入口和两个专项场景通过。

- **PASS · 候选身份**。Windows 10.0.26300，PowerShell 7.7.0。运行 `Get-FileHash -Algorithm SHA256` 和 `git rev-parse '09278b1^{commit}'`，ZIP、EXE、完整 commit 与候选声明一致，退出 0。见 [identity-verified.json](identity-verified.json)。
- **FAIL · 菜单和定稿核对**。运行 `pwsh -NoProfile -File run-dpi-matrix.ps1`，应用窗口分别报告 96 和 144 DPI。明暗四组用 Alt+E 查看原生菜单，均只有「查找」，期望出现的「替换」缺失。文件和外观菜单也采集了画面，搜索均能找到「替换」和「全部保存」，静态核对的菜单命令数为 17，定稿为 18。驱动和应用各轮退出 0。见 [100% 深色](dpi-100/dark-edit-popup.png)、[100% 浅色](dpi-100/light-edit-popup.png)、[150% 深色](dpi-150/dark-edit-popup.png)、[150% 浅色](dpi-150/light-edit-popup.png) 和 [静态对照](disposition-static.json)。
- **PASS · 另外两个替换入口**。运行 `pwsh -NoProfile -File run-replacement.ps1`，在 200% 下从 F1 命令搜索和 Ctrl+H 各打开面板并点击「替换当前」。每轮起始目标正文为 `STEAMYOLD target`，保存回读分别是 `PALETTENEW target` 和 `HOTKEYNEW target`，另一标签的可见正文和保存回读均仍是 `STEAMYOLD control`。驱动和应用退出 0。见 [搜索](replacement/palette-check.json)、[快捷键](replacement/hotkey-check.json) 和 [动作脚本](replacement/actions.ps1)。
- **PASS · Windows 资源管理器混合拖入**。运行 `pwsh -NoProfile -STA -File run-explorer-medium.ps1`。Explorer 和候选完整性级别均为 8192。对两项实际拖放，`A-success.txt` 打开并显示 `SUCCESSSTEAMYCONTENT`，`B-blocked.txt` 因显式 UTF-8 BOM 后有无效字节报 `InvalidBytes` 且无标签，原标签仍显示 `ORIGINALSTEAMYCONTENT`。驱动和应用退出 0。前几轮由高完整性驱动直接启动候选，拖入无响应。见 [输入与权限轨迹](explorer-medium/trace.jsonl)、[失败提示](explorer-medium/mixed-drop.png)、[成功项](explorer-medium/after-warning.png) 和 [原正文](explorer-medium/original-after.png)。
- **PASS · 《十日终焉》富文本滑块**。运行 `pwsh -NoProfile -File run-ui-and-thumb.ps1` 和 `pwsh -NoProfile -File summarize-thumb.ps1`，在 200% 下按住采样。下拖由 y=485 越过旧停点 y≈538 到 y=697.5；反向回到 y=484.5，再下行至 y=646.5。两轮均采到滑块与正文随输入更新，行程端点除外，松手后采样未见回跳。松手至首张与最终正文采样一致的帧分别为 97.49 ms 和 130.25 ms，属于采帧上界。驱动和应用退出 0，样本哈希前后一致。见 [时间线](thumb-timelines.json) 和 [取帧](thumb-frames)。
- **PASS · 缩放恢复**。恢复原 200%，前后显示配置相同，测试实例正常退出。见 [恢复后显示配置](dpi-after.json)。
- **BLOCKED · 只读测试副本清理**。自动执行策略拒绝了删除本轮在 TEMP 创建的只读 `novel-thumb.txt` 副本。副本目录与该轮临时目录同名，另有 `-fixtures` 后缀，参见 [运行记录](ui-thumb.json)。
