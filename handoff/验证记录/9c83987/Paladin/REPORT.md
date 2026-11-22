# Paladin 复验 9c83987

Windows 10.0.26300，1774×770 物理像素，系统缩放 200%。单编辑组，复用 60 节 Markdown 基线。
用 Get-FileHash 核对附件 ZIP 和试用目录 EXE，SHA-256 期望与候选一致，实际一致，退出码 0。见 [身份](identity.json)。

执行命令：pwsh -NoProfile -File ./verify.ps1 -IdentityPath ./identity.json。脚本、驱动、应用退出码均为 0，候选进程无残留。见 [实测记录](9c83987-000915/summary.json)。

- 交互 PASS。预期两次切换后保留第 11 节和光标。实际三张截图均显示第 11 节，状态栏均为第 43 行第 25 列，应用追踪的光标位置均在视口顶部以下 76 DIP。纯切换保持干净及磁盘正文一致。切回富文本后直接输入 STEAMYEND，切到源码仍为未保存，Ctrl+S 后回读，仅第 43 行原段尾增加该标记。见 [切换前](9c83987-000915/03-markdown-rich.png)、[源码](9c83987-000915/03-markdown-source.png)、[切回](9c83987-000915/03-markdown-restored.png)。
- 视觉 PASS。相同窗口、缩放和文档对照 3768f10。预期两行命令折叠、保留右上角格式入口和模式按钮。实际默认态两行均消失，正文上移。模式按钮实测 32×28 DIP、两态坐标相同、源码态按下反馈可见。实际点击「格式命令」并打开段落样式，F1 可打开链接表单。见 [同条件对照](before-after.png)、[格式菜单](9c83987-000915/05-format-popup.png)、[段落样式](9c83987-000915/05-format-dialog.png)。

分屏、其他缩放和跨设备 NOT_RUN。
