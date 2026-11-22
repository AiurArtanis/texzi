# 549d780 · Paladin 复验

**PASS**。在 Paladin 的 Windows 桌面、200% 缩放下完成两项复验。

- **PASS · 候选身份**。群附件在本条消息中未送达。使用共享工作区 ZIP，通过 `git rev-parse` 和 `Get-FileHash` 核对完整 commit、ZIP 与解包后的 EXE。实际值均与交付一致，检查退出 0。见 [候选身份](candidate-identity.json)。
- **PASS · 首次 Esc 与编辑焦点**。运行 `pwsh -NoProfile -File run-escape-matrix.ps1 -IdentityPath candidate-identity.json`。菜单、命令搜索、Ctrl+H 各覆盖默认焦点和查找框焦点，另覆盖菜单入口切到正文。七组期望均为第一次 Esc 关闭替换栏并恢复正文编辑焦点。实际七组按前可见、按后关闭，无需点击即可输入各自标记，逐次 Ctrl+S 回读证实标记写入且已有正文保留。脚本、驱动和应用均退出 0。见 [七组记录](escape-matrix.json)、[界面前后帧索引](matrix-round/ocr.json)、[输入轨迹](matrix-round/trace.jsonl)。
- **PASS · 查找框输入法组字**。运行 `pwsh -NoProfile -File run-escape-ime.ps1`。Ctrl+H 打开替换栏，实体按键 N、I 触发组字。原生事件记录有非空预编辑，第一次 Esc 结束预编辑且替换栏仍在，第二次 Esc 关闭替换栏，符合预期。脚本、驱动和应用均退出 0。见 [组字结果](escape-ime.json)、[原生事件](ime-round/stderr.log)、[首次 Esc](ime-round/03-first-escape.png)、[第二次 Esc](ime-round/04-second-escape.png)。
- **NOT_RUN · 拖入与滑块**。既有 PASS 归属 09278b1。
