# texzi 原生输入阶段私有测试包

本包为T04接入中的内存编辑检查点，不是首版完整编辑器。解压全部文件后运行texzi-app.exe。
输入的正文仅在进程内；当前尚未接入打开/保存/恢复，关闭将丢弃本次生成文本，请只用测试内容。
T05/T06可编辑富文本及T07编码/EOL/安全保存均保留在首版范围，不能以本包代替。

主内容固定前景黑色/背景白色，不宣称全主题或高对比支持；系统标题栏由Windows管理。
NSH Editor使用随包可追溯fork：输入/多选区计划、剪贴板命令、Undo/Redo连接texzi-document唯一Store。
渲染Rope是共享不可变快照缓存，绑定模式不使用NSH自己的正文Undo/Redo栈，不逐键发全篇onChange。
逻辑View具独立位置；复杂续传/重放不作为UI前置，通知后检查版本/epoch，缺更新重新取快照。
上述为代码路径说明，实际通过范围须看绑定本包commit/run/hash的本机独测，不能由构建成功推断。

常用输入命令：方向键/Shift选区、Ctrl+A/C/X/V、Ctrl+Z/Y；多选区/矩形选择沿用NSH编辑器命令映射，
是否正确接入及IME组合行为需要真实界面验证。Unicode直接注入不是Pinyin/Sogou组合实测。
基础查找及富文本/源码切换后续集成；本阶段不声称全部T04或任何Gate已完成。

核SHA256SUMS.txt中的exe校验和及build-identity.txt的commit/run/attempt后执行：

```powershell
Get-FileHash ./texzi-app.exe -Algorithm SHA256
./texzi-app.exe 2> ./startup.log
$LASTEXITCODE
```

保留当前Windows版本、截图/录屏、输入方式、生成fixture、真实退出码、stderr和残留进程结果。
故障入口--tx00-fail-window-create与STARTUP_FAILURE.md仍保留，出现模态错误框需确认后记录真实退出。
不把等待错误框认作自动退出或挂死。

仅Windows x64，Rust1.97.0/MSVC，静态CRT；系统Direct2D/DirectWrite及系统字体，不捆字体或未经核验DLL。
新增启用nsh-base/editor；确切直接/传递依赖及许可见metadata.json、sbom.json、licenses/、feature图。
fork来源manifest和补丁边界见component-tests/NSH-FORK.md、NSH-UPSTREAM-MANIFEST.json。
Win10/1809交付后验证，本机未测环境不外推。无安装器/发布/签名；框架可能写缓存，不宣称全局无写入。
