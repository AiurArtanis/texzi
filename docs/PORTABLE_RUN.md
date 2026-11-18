# texzi 本机开发测试包

解压全部文件后，从资源管理器双击 `texzi-app.exe`。本包内嵌 Ink Z 图标，Release 为 Windows GUI 子系统。它仍是开发候选，请只用生成的测试文档：支持严格 UTF-8/BOM 读取，保存、完整编码与恢复尚未通过；关闭未保存内容必须明确选择放弃，不能把内存编辑当成已保存。

- “源码”可直接输入 Markdown；“富文本”用于同一正文的原生编辑。组合输入尚在进行时，先确认或取消组合，再切换模式。
- `Ctrl+Z` 撤销，`Ctrl+Y` 重做；两种模式共享撤销历史。源码多选区能力保留，富文本多选及复杂位置恢复尚未完成。
- 暂不能保真处理的操作会显示原因，并保留原状态；可切到源码编辑该范围。富文本结构和数据安全功能仍在开发中，不是完整首版验收。
- 外框为 Windows 原生菜单，Alt/方向键/Esc 操作菜单。提供标签、文件夹浏览、查找替换、命令面板、源码行号、双组视图、主题与 Zen。双组共享同一正文和撤销历史，当前最多两个组；新窗口是独立空窗口，不承诺跨窗迁移。保存项明确不可用，设置仍未接入，其余行为以本包对应的逐项验证记录为准。
- 普通查找在富文本中按可见文字查找；替换和源码行号先切到源码。目录搜索限制为严格 UTF-8 文本，并显示预算、跳过、错误与取消，不声称覆盖任意目录或完整编码。
- `Ctrl+K` 组合键等待第二键；无效第二键取消。`Ctrl+B` 在富文本中保留加粗，侧栏统一使用 `Ctrl+K Ctrl+B`。代码、未知语法与数学等原样类的边界见 `component-tests/SYNTAX-MATRIX.md`；组件通过不等于全部 GUI 交互已通过。
- 当前验证暂停全部剪贴板读写及 `Ctrl+C/X/V`，也不执行剪贴板恢复探测。此前测试未证明完整恢复原剪贴板，该缺口保留。

运行前核对 `build-identity.txt` 的 commit/run/attempt 和 `SHA256SUMS.txt`：

```powershell
Get-FileHash ./texzi-app.exe -Algorithm SHA256
$app = Start-Process -FilePath (Resolve-Path ./texzi-app.exe) -PassThru -RedirectStandardError ./startup.log
$ownedHandle = $app.Handle
$app.WaitForExit()
$app.ExitCode
```

正常关闭应退出进程。错误弹窗须确认后再检查真实退出码，不能把等待确认当作自动退出或挂死；隔离故障入口见 `STARTUP_FAILURE.md`。

本包仅面向 Windows x64 本机开发验证，使用系统 Direct2D/DirectWrite、系统字体和静态 MSVC CRT，不另装运行时。依赖许可随 `licenses/`、`sbom.json` 提供。组件测试与开发限制见 `component-tests/RICH-RUN.md`；构建成功不代表实际运行或人工验收通过。
