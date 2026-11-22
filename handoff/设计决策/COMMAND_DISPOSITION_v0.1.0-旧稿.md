# texzi 下一版命令删留清单

基线为 v0.1.0（b7b7e16）。快捷键列抄录旧注册表，执行时保留编辑器、查找栏与富文本原有作用域。第二类保留原有可用性、上下文入口与输入行为；这次菜单调整不补做其他尚未接线的功能。

Ctrl+E / Ctrl+P 进入文件搜索，F1 / Ctrl+Shift+P 进入命令搜索，Ctrl+F 进入文内查找。模式切换器复用原命令。原生菜单与快捷键注册分开，Alt+F/E/V 按稳定标识打开三组菜单，移除原顶级菜单 Alt+S/G/H。另保留 Ctrl+Shift+W、Ctrl+F4 等已有外壳别名。

| 旧命令 ID | 名称 | 旧注册键 | 下一版去向 |
|---|---|---|---|
| texzi.file.new | 新建文本 | Ctrl+N | 留在菜单（文件） |
| texzi.file.open | 打开文件 | Ctrl+O | 留在菜单（文件） |
| texzi.folder.open | 打开文件夹 | Ctrl+K Ctrl+O | 留在菜单（文件） |
| texzi.folder.close | 关闭文件夹 | Ctrl+K F | 仅留搜索和原快捷键 |
| texzi.file.save | 保存 | Ctrl+S | 留在菜单（文件） |
| texzi.file.saveAs | 另存为 | Ctrl+Shift+S | 留在菜单（文件） |
| texzi.file.saveAll | 全部保存 | Ctrl+K S | 仅留搜索和原快捷键 |
| texzi.file.revert | 从磁盘重新载入 | — | 彻底移除用户入口及绑定 |
| texzi.file.reopenWithEncoding | 以编码重新打开 | — | 彻底移除用户入口及绑定 |
| texzi.file.saveWithEncoding | 使用编码保存 | — | 彻底移除用户入口及绑定 |
| texzi.file.changeEol | 转换换行符 | — | 彻底移除用户入口及绑定 |
| texzi.file.copyPath | 复制完整路径 | Ctrl+K P | 彻底移除用户入口及绑定 |
| texzi.file.revealInExplorer | 在Windows资源管理器中显示 | Ctrl+K R | 仅留搜索和原快捷键 |
| texzi.window.new | 新建窗口 | Ctrl+Shift+N | 仅留搜索和原快捷键 |
| texzi.window.close | 关闭窗口 | Alt+F4、Ctrl+Shift+W | 仅留搜索和原快捷键 |
| texzi.edit.undo | 撤销 | Ctrl+Z | 留在菜单（编辑） |
| texzi.edit.redo | 重做 | Ctrl+Y、Ctrl+Shift+Z | 留在菜单（编辑） |
| texzi.edit.cut | 剪切 | Ctrl+X、Shift+Delete | 留在菜单（编辑） |
| texzi.edit.copy | 复制 | Ctrl+C、Ctrl+Insert | 留在菜单（编辑） |
| texzi.edit.paste | 粘贴 | Ctrl+V、Shift+Insert | 留在菜单（编辑） |
| texzi.edit.pastePlain | 原样粘贴文本 | Ctrl+Shift+V | 仅留搜索和原快捷键 |
| texzi.edit.selectAll | 全选 | Ctrl+A | 留在菜单（编辑） |
| texzi.edit.deleteBackward | 向前删除 | Backspace、Shift+Backspace | 仅留搜索和原快捷键 |
| texzi.edit.deleteForward | 向后删除 | Delete | 仅留搜索和原快捷键 |
| texzi.edit.deleteWordLeft | 删除前一个词 | Ctrl+Backspace | 仅留搜索和原快捷键 |
| texzi.edit.deleteWordRight | 删除后一个词 | Ctrl+Delete | 仅留搜索和原快捷键 |
| texzi.edit.indent | 增加缩进 | Ctrl+] | 仅留搜索和原快捷键 |
| texzi.edit.outdent | 减少缩进 | Ctrl+[ | 仅留搜索和原快捷键 |
| texzi.edit.tab | Tab输入或结构导航 | Tab | 仅留搜索和原快捷键 |
| texzi.edit.shiftTab | 反向Tab或减少缩进 | Shift+Tab | 仅留搜索和原快捷键 |
| texzi.edit.moveLineUp | 上移行 | Alt+Up | 仅留搜索和原快捷键 |
| texzi.edit.moveLineDown | 下移行 | Alt+Down | 仅留搜索和原快捷键 |
| texzi.edit.copyLineUp | 向上复制行 | Shift+Alt+Up | 仅留搜索和原快捷键 |
| texzi.edit.copyLineDown | 向下复制行 | Shift+Alt+Down | 仅留搜索和原快捷键 |
| texzi.edit.deleteLine | 删除整行 | Ctrl+Shift+K | 仅留搜索和原快捷键 |
| texzi.edit.insertLineAbove | 在上方插入行 | Ctrl+Shift+Enter | 仅留搜索和原快捷键 |
| texzi.edit.insertLineBelow | 在下方插入行 | Ctrl+Enter | 仅留搜索和原快捷键 |
| texzi.edit.pasteLinesToCursors | 按行分配粘贴 | — | 仅留搜索和原快捷键 |
| texzi.cursor.move | 移动光标（逐方向） | Left、Right、Up、Down、Home、End、Ctrl+Home、Ctrl+End、PageUp、PageDown、Ctrl+Left、Ctrl+Right | 仅留搜索和原快捷键 |
| texzi.selection.extend | 扩展选区（逐方向） | Shift+Left、Shift+Right、Shift+Up、Shift+Down、Shift+Home、Shift+End、Ctrl+Shift+Home、Ctrl+Shift+End、Shift+PageUp、Shift+PageDown、Ctrl+Shift+Left、Ctrl+Shift+Right | 仅留搜索和原快捷键 |
| texzi.selection.line | 选择当前行 | Ctrl+L | 仅留搜索和原快捷键 |
| texzi.navigation.gotoLine | 转到行/列 | Ctrl+G | 仅留搜索和原快捷键 |
| texzi.navigation.quickOpen | 快速打开文件 | Ctrl+P、Ctrl+E | 仅留搜索和原快捷键 |
| texzi.navigation.back | 后退到上个位置 | Alt+Left | 仅留搜索和原快捷键 |
| texzi.navigation.forward | 前进到下个位置 | Alt+Right | 仅留搜索和原快捷键 |
| texzi.multicursor.addMouse | 鼠标添加光标 | Alt+Click | 仅留搜索和原快捷键 |
| texzi.multicursor.above | 在上方添加光标 | Ctrl+Alt+Up | 仅留搜索和原快捷键 |
| texzi.multicursor.below | 在下方添加光标 | Ctrl+Alt+Down | 仅留搜索和原快捷键 |
| texzi.multicursor.addNext | 选择下一个相同内容 | Ctrl+D | 仅留搜索和原快捷键 |
| texzi.multicursor.skipNext | 跳过当前匹配选择下一个 | Ctrl+K Ctrl+D | 仅留搜索和原快捷键 |
| texzi.multicursor.selectAllOccurrences | 选择所有相同内容 | Ctrl+Shift+L | 仅留搜索和原快捷键 |
| texzi.multicursor.selectWordOccurrences | 选择当前词全部出现位置 | Ctrl+F2 | 仅留搜索和原快捷键 |
| texzi.multicursor.undo | 撤销上次光标操作 | Ctrl+U | 仅留搜索和原快捷键 |
| texzi.multicursor.lineEnds | 在所选行末添加光标 | Shift+Alt+I | 仅留搜索和原快捷键 |
| texzi.selection.column | 矩形选择 | Ctrl+Shift+Alt+Left、Ctrl+Shift+Alt+Right、Ctrl+Shift+Alt+Up、Ctrl+Shift+Alt+Down、Ctrl+Shift+Alt+PageUp、Ctrl+Shift+Alt+PageDown、Shift+Alt+Drag | 仅留搜索和原快捷键 |
| texzi.find.open | 查找 | Ctrl+F | 留在菜单（编辑） |
| texzi.find.replaceOpen | 替换 | Ctrl+H | 仅留搜索和原快捷键 |
| texzi.find.next | 下一个匹配 | F3、Enter | 仅留搜索和原快捷键 |
| texzi.find.previous | 上一个匹配 | Shift+F3、Shift+Enter | 仅留搜索和原快捷键 |
| texzi.find.toggleCase | 区分大小写 | Alt+C | 仅留搜索和原快捷键 |
| texzi.find.toggleWholeWord | 全字/部分匹配 | Alt+W | 仅留搜索和原快捷键 |
| texzi.find.toggleRegex | 正则表达式 | Alt+R | 仅留搜索和原快捷键 |
| texzi.find.toggleSelection | 仅在选区中查找 | Alt+L | 仅留搜索和原快捷键 |
| texzi.find.replaceOne | 替换当前匹配 | Ctrl+Shift+1、Enter | 仅留搜索和原快捷键 |
| texzi.find.replaceAll | 替换全部匹配 | Ctrl+Alt+Enter | 仅留搜索和原快捷键 |
| texzi.find.selectAllMatches | 选中全部查找匹配 | Alt+Enter | 仅留搜索和原快捷键 |
| texzi.find.close | 关闭查找栏 | Escape、Shift+Escape | 仅留搜索和原快捷键 |
| texzi.find.inFolder | 在文件夹中查找 | Ctrl+Shift+F | 仅留搜索和原快捷键 |
| texzi.tab.close | 关闭标签 | Ctrl+W、Ctrl+F4 | 留在菜单（文件） |
| texzi.tab.closeOthers | 关闭同组其他标签 | — | 彻底移除用户入口及绑定 |
| texzi.tab.closeAll | 关闭窗口所有标签 | Ctrl+K Ctrl+W | 彻底移除用户入口及绑定 |
| texzi.tab.reopenClosed | 重新打开已关闭标签 | Ctrl+Shift+T | 仅留搜索和原快捷键 |
| texzi.tab.keepOpen | 将预览标签转为常驻 | Ctrl+K Enter | 仅留搜索和原快捷键 |
| texzi.tab.pin | 固定/取消固定标签 | Ctrl+K Shift+Enter | 仅留搜索和原快捷键 |
| texzi.tab.next | 下一个标签 | Ctrl+PageDown | 仅留搜索和原快捷键 |
| texzi.tab.previous | 上一个标签 | Ctrl+PageUp | 仅留搜索和原快捷键 |
| texzi.tab.recent | 按最近使用顺序切换标签 | Ctrl+Tab、Ctrl+Shift+Tab | 仅留搜索和原快捷键 |
| texzi.tab.moveLeft | 将标签左移 | Ctrl+Shift+PageUp | 仅留搜索和原快捷键 |
| texzi.tab.moveRight | 将标签右移 | Ctrl+Shift+PageDown | 仅留搜索和原快捷键 |
| texzi.group.splitRight | 向右拆分 | Ctrl+\ | 仅留搜索和原快捷键 |
| texzi.group.splitDown | 向下拆分 | Ctrl+K Ctrl+\ | 仅留搜索和原快捷键 |
| texzi.group.focusIndex | 聚焦编辑组 | Ctrl+1、Ctrl+2、Ctrl+3、Ctrl+4、Ctrl+5、Ctrl+6、Ctrl+7、Ctrl+8 | 仅留搜索和原快捷键 |
| texzi.group.focusDirection | 聚焦相邻编辑组 | Ctrl+K Ctrl+Left、Ctrl+K Ctrl+Right、Ctrl+K Ctrl+Up、Ctrl+K Ctrl+Down | 彻底移除用户入口及绑定 |
| texzi.group.joinAll | 合并为单一编辑组 | — | 仅留搜索和原快捷键 |
| texzi.tab.moveToNewWindow | 将标签移到新窗口 | — | 彻底移除用户入口及绑定 |
| texzi.tab.copyToNewWindow | 在新窗口打开同一文档 | Ctrl+K O | 彻底移除用户入口及绑定 |
| texzi.sidebar.toggle | 显示/隐藏侧栏 | Ctrl+B、Ctrl+K Ctrl+B | 留在菜单（外观） |
| texzi.view.toggleZen | 进入/退出Zen | Ctrl+K Z | 仅留搜索和原快捷键 |
| texzi.view.exitZen | 退出Zen | Escape Escape | 仅留搜索和原快捷键 |
| texzi.view.toggleFullScreen | 切换全屏 | F11 | 仅留搜索和原快捷键 |
| texzi.editor.toggleWrap | 切换自动换行 | Alt+Z | 留在菜单（外观） |
| texzi.settings.open | 打开设置 | Ctrl+, | 彻底移除用户入口及绑定 |
| texzi.settings.keybindings | 打开键盘快捷键 | Ctrl+K Ctrl+S | 彻底移除用户入口及绑定 |
| texzi.commandPalette.open | 打开命令面板 | Ctrl+Shift+P、F1 | 仅留搜索和原快捷键 |
| texzi.editor.toggleTabFocus | Tab切换焦点模式 | Ctrl+M | 仅留搜索和原快捷键 |
| texzi.file.changeLanguage | 更改语言高亮模式 | Ctrl+K M | 仅留搜索和原快捷键 |
| texzi.markdown.toggleSource | 切换Markdown编辑/源码 | — | 仅留搜索和原快捷键 |
| texzi.markdown.bold | 切换加粗 | Ctrl+B | 仅留搜索和原快捷键 |
| texzi.markdown.italic | 切换斜体 | Ctrl+I | 仅留搜索和原快捷键 |
| texzi.markdown.insertLink | 插入或编辑链接 | — | 仅留搜索和原快捷键 |
| texzi.markdown.toggleSyncScroll | 切换关联区域同步滚动 | — | 仅留搜索和原快捷键 |
| texzi.markdown.pasteMarkdown | 粘贴Markdown | — | 仅留搜索和原快捷键 |
| texzi.markdown.copyMarkdown | 复制Markdown | — | 仅留搜索和原快捷键 |
| texzi.markdown.exitCodeBlock | 退出代码块 | — | 仅留搜索和原快捷键 |
| texzi.markdown.strikethrough | 切换删除线 | — | 仅留搜索和原快捷键 |
| texzi.markdown.inlineCode | 切换行内代码 | — | 仅留搜索和原快捷键 |
| texzi.markdown.toggleTask | 切换任务完成状态 | — | 仅留搜索和原快捷键 |
| texzi.markdown.setCodeLanguage | 设置代码块语言 | — | 仅留搜索和原快捷键 |
| texzi.markdown.table.insertRow | 插入表格行 | — | 仅留搜索和原快捷键 |
| texzi.markdown.table.deleteRow | 删除表格行 | — | 仅留搜索和原快捷键 |
| texzi.markdown.table.insertColumn | 插入表格列 | — | 仅留搜索和原快捷键 |
| texzi.markdown.table.deleteColumn | 删除表格列 | — | 仅留搜索和原快捷键 |
| texzi.markdown.table.alignColumn | 设置表格列对齐 | — | 仅留搜索和原快捷键 |
| texzi.recovery.open | 打开恢复中心 | — | 仅留搜索和原快捷键 |
| texzi.performance.reassessDocument | 重新评估当前文档模式 | — | 仅留搜索和原快捷键 |
| texzi.help.guide | 使用说明 | — | 仅留搜索和原快捷键 |
| texzi.help.about | 关于 texzi | — | 仅留搜索和原快捷键 |
| texzi.appearance.light | 浅色主题 | — | 留在菜单（外观） |
| texzi.appearance.dark | 深色主题 | — | 留在菜单（外观） |
| texzi.multicursor.clear | 清除额外选区 | — | 仅留搜索和原快捷键 |
