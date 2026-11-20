# texzi

Windows 原生文本和 Markdown 编辑器，支持源码和所见即所得编辑。

## 构建

在 Windows x64 安装 Rustup、Visual Studio C++ Build Tools 和 Windows SDK。在项目根目录运行：

```powershell
cargo build --release
```

## 运行

双击 `target/release/texzi-app.exe`，或在 PowerShell 运行 `./target/release/texzi-app.exe`。版本范围与验收见 [V1-SCOPE](docs/V1-SCOPE.md)。
