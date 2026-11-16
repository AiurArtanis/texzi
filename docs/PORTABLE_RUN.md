# texzi 本机开发测试包

解压全部文件后运行 `texzi-app.exe`。这是内存编辑阶段的私有候选，关闭会丢弃本次输入；当前没有打开、保存或恢复功能，请只使用生成的测试内容。

- “源码”可直接输入 Markdown；“富文本”用于同一正文的原生编辑。组合输入尚在进行时，先确认或取消组合，再切换模式。
- `Ctrl+Z` 撤销，`Ctrl+Y` 重做；两种模式共享撤销历史。源码多选区能力保留，富文本多选及复杂位置恢复尚未完成。
- 暂不能保真处理的操作会显示原因，并保留原状态；可切到源码编辑该范围。富文本结构和数据安全功能仍在开发中，不是完整首版验收。
- 当前验证暂停全部剪贴板读写及 `Ctrl+C/X/V`，也不执行剪贴板恢复探测。此前测试未证明完整恢复原剪贴板，该缺口保留。

运行前核对 `build-identity.txt` 的 commit/run/attempt 和 `SHA256SUMS.txt`：

```powershell
Get-FileHash ./texzi-app.exe -Algorithm SHA256
./texzi-app.exe 2> ./startup.log
$LASTEXITCODE
```

正常关闭应退出进程。错误弹窗须确认后再检查真实退出码，不能把等待确认当作自动退出或挂死；隔离故障入口见 `STARTUP_FAILURE.md`。

本包仅面向 Windows x64 本机开发验证，使用系统 Direct2D/DirectWrite、系统字体和静态 MSVC CRT，不另装运行时。依赖许可随 `licenses/`、`sbom.json` 提供。组件测试与开发限制见 `component-tests/RICH-RUN.md`；构建成功不代表实际运行或人工验收通过。
