# texzi 产品约束

> **2026-11-20 起本文只作参考。** v1 的范围和验收只看 [docs/V1-SCOPE.md](../V1-SCOPE.md)。

<!-- impeccable:product-schema 1 -->

## Platform

Windows desktop native，x64。用户确认最低系统大版本 Windows 10；本包建议的 NSH 技术兼容基线为 Windows 10 1809+，验证主机 Windows 10 22H2 与 Windows 11。

## Stack

NSH Native Shell，固定核查版本 v0.5.1 / 224d9a795c7ac9d10d75750de9351adac5549675。交付中的浏览器原型仅用于交互审阅，不作为原生实现或性能证据。

## Users

日常管理文字、临时文本、Markdown 和代码片段的 Windows 用户。熟悉 VS Code 的编辑和窗口布局，但不需要完整 IDE。

## Product Purpose

启动快、输入流畅、布局友好、简洁耐用；一个应用完成临时记事、文件夹文本管理、Markdown 直接写作和代码片段编辑。

## Confirmed Constraints

- 名称 texzi（text 与“字”结合）。
- 顶栏不显示产品名称；菜单并入窗口外框，搜索与窗口按钮同排。
- Markdown 必须所见即所得编辑，正文右上 >_ 原位切换源码。
- 分屏、多编辑区、独立窗口、文件与文件夹、多标签、多选和多光标、撤销重做、完整搜索替换。
- Z 模式明确为 Zen 模式。
- 其他编程语言只要求语法高亮，不扩展为完整开发功能。
- 不识别或判断用户输入的敏感性。
- 首版不做插件或扩展生态。
- 2026-11-15 用户确认概念修改后授权完整 A—E 交付，并明确架构是 x64。
- 用户接受前轮性能目标，具体环境、口径和模式范围见 PERFORMANCE_PLAN.md；未声称有原生实测结果。

## Evidence on Hand

concepts/01-main.png、02-markdown-wysiwyg-split.png、03-settings.png 为已确认视觉依据。用户要求优先于任何设计技能的默认视觉偏好。

## Product Principles

1. 文档内容与用户操作可追溯，切换视图不改写原文。
2. 同一文档一条撤销历史；每个视图独立光标、滚动与模式。
3. 复杂工作在后台进行，中文组合输入优先。
4. 限制与降级可见，恢复与保存行为可解释。
5. 熟悉的 Windows 操作与克制的视觉结构优先。

## Open Engineering Decisions

具体依赖版本、未获支持语法的原文块适配、RichEditor 多视图和虚拟化补丁、Windows 签名与更新通道。全部是可执行验证任务，不再阻塞设计交付。
