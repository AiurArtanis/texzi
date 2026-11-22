# d14cc36 · Paladin 复验

**FAIL**。替换入口修复通过，首次按 Esc 关闭替换栏仍失败。

- **PASS · 候选身份**。群 ZIP 附件未送达。本轮从共享工作区读取 `evidence/v0.2-edit-menu/texzi-v1-d14cc36-win-x64.zip`，解包后以 `Get-FileHash` 比对 ZIP 与 EXE 的 SHA-256，并以 `git rev-parse` 核对完整 commit `d14cc36d47961732515b9bfa3768e8b698376068`。三项期望与实际一致，身份检查退出 0。见 [候选身份](candidate-identity.json)。
- **PASS · 命令清单静态核对**。运行 `pwsh -NoProfile -File check-disposition.ps1 -Disposition <定稿文件>`，按 ID 和菜单分组独立比较。期望 18／90／12，实际一致，文件／编辑／外观分别 6／8／4，120 项无差异。退出 0。见 [清单对照](disposition.json)。
- **PASS · 真实应用四组菜单**。运行 `pwsh -NoProfile -File run-dpi-matrix.ps1`，以窗口 DPI 96 和 144 验证 100% 与 150%。两种缩放下分别使用明暗主题，按 Alt+E 后均能看到「替换 Ctrl+H」。驱动和应用各轮退出 0。见 [100% 深](dpi-100/dark-edit-popup.png)、[100% 浅](dpi-100/light-edit-popup.png)、[150% 深](dpi-150/dark-edit-popup.png)、[150% 浅](dpi-150/light-edit-popup.png)。
- **PASS · 三个实际替换入口**。运行 `pwsh -NoProfile -File run-replacement.ps1`，在原 200% 缩放下分别点击「编辑 → 替换」、从 F1 命令搜索执行替换、按 Ctrl+H。每轮从 `STEAMYOLD target` 开始，点击「替换当前」，Ctrl+S 保存并回读，分别得到预期的 `MENUNEW target`、`PALETTENEW target`、`HOTKEYNEW target`，另一标签的保存回读始终为 `STEAMYOLD control`。驱动与应用退出 0。见 [菜单](replacement/menu-check.json)、[搜索](replacement/palette-check.json)、[快捷键](replacement/hotkey-check.json)。
- **FAIL · 首次 Esc**。期望一次 Esc 关闭替换栏。三个入口各在两种条件下实测：打开后直接按 Esc；点击「查找文字」输入框并输入探针文字后按 Esc。后者的按键前画面有输入文字及光标。六次均仍显示替换栏。另在菜单入口点击正文、栏仍可见时按 Esc，也未关闭。运行 `pwsh -NoProfile -File run-escape-direct.ps1` 和 `pwsh -NoProfile -File compare-escape-frames.ps1`，两脚本与被测应用退出 0。见 [菜单轨迹](escape-direct/trace.jsonl)、[输入框聚焦前后图索引及像素对照](escape-frame-diffs.json)、[按键后一帧](replacement/menu-esc-after-first.png)。
- **NOT_RUN · 拖入与滑块**。现有 PASS 归属 [09278b1](../09278b1-paladin/REPORT.md)。
- **PASS · 环境恢复**。测试实例正常退出，Windows 显示缩放回到原 200%。见 [恢复配置](dpi-after.json)。

## 群附件与续键补验

- **PASS · 群附件身份**。用 `Get-FileHash` 核对群 ZIP 和已运行 EXE 的 SHA-256。ZIP 与先前复验的共享工作区 ZIP 完全相同，EXE 与交付记录匹配，退出 0。见 [群附件身份](attachment-check.json)。
- **PASS · Ctrl+K 续键**。运行 `pwsh -NoProfile -File run-command-chords.ps1`，在输入法英文模式实测。先编辑测试文件。按 Ctrl+K Ctrl+S 后，磁盘仍为 `ORIGINAL`，哈希不变，应用记录续键未匹配。第一次前缀后 926.28 ms 按第二次 Ctrl+K，再过 753.72 ms 按 S，应用记录 `texzi.file.saveAll`，磁盘正文变为预期的 `SAVEDCHORDORIGINAL`。S 落在第一次前缀的 1500 ms 超时点之后。脚本、驱动、应用均退出 0。见 [输入时序与写盘](command-chords.json) 和 [应用轨迹](chord-round/stderr.log)。
