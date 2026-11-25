# 在 Windows x64 试用 texzi

1. 下载并解压整个 Windows x64 ZIP。
2. 双击解压目录中的 **texzi-app.exe**。使用测试文件验证打开、保存与 Markdown 编辑。
3. 通过顶栏的「文件、编辑、外观」访问常用命令。按 Ctrl+P 搜索文件，按 Ctrl+Shift+P 搜索命令。打开 Markdown 后，点编辑器右上角的 **>_** 切换源码与富文本。

包内的 build-identity.txt 记录完整 Git 提交和构建运行。可在 PowerShell 中运行 Get-FileHash ./texzi-app.exe，将输出与包内 SHA256SUMS.txt 比对。外部 SHA256SUMS.txt 用于核对下载的 ZIP。
