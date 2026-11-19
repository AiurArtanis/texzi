# Paladin Windows x64 构建基线

2026-11-18 已在隔离 checkout 验证提交 `256b399f2a7ff57893b20041dc91b75988551329`。Rust 1.97.0，目标 x86_64-pc-windows-msvc，VS 2022 Build Tools 17.14.45，MSVC 14.44.35207，Windows SDK 10.0.26100.0。

在 PowerShell 7 中运行 `pwsh -NoProfile -File scripts/build-candidate.ps1`。该版本 Release 构建、打包退出码均为 0。应用最终链接使用 MSVC HostX64/x64/link.exe，SDK 提供 x64 kernel32.lib。环境测试未启动 GUI。

本机原始证据目录为 D:/Work/texzi/evidence/env/msvc-256b399，包含环境验证报告.md、tool-inventory.json 和 final-app-link-command.txt。
