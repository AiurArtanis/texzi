# texzi 设置规格 v1

> **2026-11-20 起本文只作参考。** v1 的范围和验收只看 [docs/V1-SCOPE.md](../../V1-SCOPE.md)。本文中的「必须」「阻断发布」「Gate」「验收」不再构成门禁；与 V1-SCOPE 冲突时，以 V1-SCOPE 为准。

本文件定义首版设置的唯一字段名、类型和生效语义。机器可读对应文件为 `../data/settings.json`。平台为已确认的 Windows 10 x64；界面只有 Windows 适配。除明确标为“已确认”的 Markdown 默认编辑方式外，下表默认值均为本方案建议，不冒充用户已确认的要求。

## 1. 设置如何生效

优先级为 `documentSession > workspaceLocal > user > builtIn`。`documentSession` 是当前文档会话覆盖，恢复会话时随文档恢复；`workspaceLocal` 按规范化根文件夹路径保存在本机应用数据目录，不因打开一个文件夹就在其中生成配置文件；`user` 影响所有窗口。无工作区时跳过 workspaceLocal。只允许表内列出的作用域，外观与恢复设置不能被文件夹偷偷覆盖。

有效值先完成类型、范围、依赖校验，再一次性应用并持久化。保存失败显示错误，页面保留用户输入和未保存标记；不能显示“已保存”。未知字段保留但不执行。重置单项只移除当前作用域的覆盖，显示恢复后的有效来源；重置全部先列出将重置的作用域。数值输入允许临时空白，失焦/Enter提交时校验，越界不静默截断。

即时改字号、字体、行距时，通过源码锚点保留当前选区和视口；不得创建正文撤销步。切换主题不得闪白整个窗口。依赖不满足的控件显示禁用原因；依赖重新满足时恢复已保存值，不自动覆盖它。

## 2. 设置页面结构

设置作为每窗口单个普通可关闭标签，已打开时再次调用只聚焦。顶层导航保持七类：常用、编辑器、外观、文件与恢复、Markdown、窗口与标签、快捷键。查找与大文件归入编辑器子分组，Zen归入窗口与标签子分组，同时保留搜索别名；JSON的category直接对应五个实际设置分类；常用与快捷键为虚拟页。查找、大文件、文件、恢复、Zen等原分组保存在subcategory字段，不能生成额外顶层导航。顶部是设置搜索，支持中文名称、描述和稳定ID。常用页选取主题、源码与代码字体/字号、Markdown默认编辑方式、自动换行、自动保存、会话恢复，不复制另一套状态。设置页上方菜单与窗口控制按钮仍在同一行；不新增产品名标题。

“源码与代码字体”是准确标签，不能写成暗示同时控制全部正文的“编辑器字体”。字号用DIP；Windows缩放按系统DPI转换，不能把150%缩放下16 DIP误记为16物理像素。

## 3. 完整设置表

下表按内容子组排列，便于逐项检索；实际顶层导航仍为第2节七类。

### 外观

| ID / 名称 | 作用 | 类型 / 可选值 | 建议默认值 | 生效时机 | 依赖 | 作用域 | 状态 / 优先级 |
|---|---|---|---|---|---|---|---|
| `appearance.theme`<br>主题 | 选择整体浅色、深色或跟随Windows系统；系统高对比度开启时优先使用可访问配色。 | enum；system（跟随系统）；light（浅色）；dark（深色） | system | 立即生效 | 无 | user | 设计建议 / P0 |
| `appearance.interfaceFontSize`<br>界面字号 | 控制菜单、文件树、标签和设置控件字号，不改变正文。 | integer；12–20 DIP；步长1 | 14 DIP | 立即生效 | 无 | user | 设计建议 / P0 |
| `appearance.reducedMotion`<br>减少动态效果 | 关闭面板与滚动的装饰性过渡；不影响光标和进度反馈。system跟随Windows动画设置。 | enum；system（跟随系统）；on（减少）；off（标准） | system | 立即生效 | 无 | user | 设计建议 / P0 |

### 编辑器

| ID / 名称 | 作用 | 类型 / 可选值 | 建议默认值 | 生效时机 | 依赖 | 作用域 | 状态 / 优先级 |
|---|---|---|---|---|---|---|---|
| `editor.fontFamily`<br>源码与代码字体 | 按顺序选择等宽字体；缺失时回退Consolas，再由Windows选择CJK字形。WYSIWYG正文另用Markdown字体。 | string；自由文本；字体名列表或system；不存在的字体按回退规则处理 | Cascadia Mono, Consolas | 立即重新排版并保留光标锚点 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `editor.fontSize`<br>源码与代码字号 | 只改变纯文本、代码和Markdown源码区域字号。 | integer；10–36 DIP；步长1 | 16 DIP | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `editor.lineHeight`<br>源码与代码行距 | 源码、纯文本及代码行高相对于字号的倍数。 | number；1.2–2.2 倍；步长0.1 | 1.5 倍 | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `editor.lineNumbers`<br>显示行号 | 在源码和代码区域显示逻辑行号；软换行续行不重复编号。 | boolean；false（关闭）；true（开启） | true | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `editor.renderWhitespace`<br>显示空白字符 | 显示空格与Tab的辅助标记，不写入文件；全选区模式只标出选中范围。 | enum；none（不显示）；selection（选区内）；all（全部） | selection | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `editor.highlightCurrentLine`<br>突出当前行 | 轻量背景提示主光标所在行；多光标不铺满整屏。 | boolean；false（关闭）；true（开启） | true | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `editor.cursorWidth`<br>光标宽度 | 设置线形插入光标宽度；不改变文字宽度和输入位置。 | integer；1–4 DIP；步长1 | 2 DIP | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `editor.cursorBlinking`<br>光标闪烁 | system使用系统光标闪烁周期；solid保持常亮。输入法组合期间保留组合态可见。 | enum；system（跟随系统）；solid（常亮） | system | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `editor.wordWrap.text`<br>纯文本自动换行 | 控制.txt与未知语言纯文本的软换行；不插入实际换行符。 | enum；on（on）；off（off） | on | 立即生效 | 无 | user / workspaceLocal / documentSession | 设计建议 / P0 |
| `editor.wordWrap.markdown`<br>Markdown源码自动换行 | 仅控制Markdown源码视图；WYSIWYG正文按区域宽度排版。 | enum；on（on）；off（off） | on | 立即生效 | 无 | user / workspaceLocal / documentSession | 设计建议 / P0 |
| `editor.wordWrap.code`<br>代码自动换行 | 控制其他语法高亮语言的软换行。极长行保护可以临时关闭并给出状态提示。 | enum；on（on）；off（off） | off | 立即生效 | 无 | user / workspaceLocal / documentSession | 设计建议 / P0 |
| `editor.tabSize`<br>缩进宽度 | Tab显示宽度与使用空格缩进的单级宽度；默认4，JSON不额外暗设2。 | integer；1–8 列；步长1 | 4 列 | 立即生效 | 无 | user / workspaceLocal / documentSession | 设计建议 / P0 |
| `editor.insertSpaces`<br>缩进使用空格 | 开启时Tab/增加缩进使用空格，关闭时使用Tab；不自动转换已有缩进。 | boolean；false（关闭）；true（开启） | true | 立即生效 | 无 | user / workspaceLocal / documentSession | 设计建议 / P0 |
| `editor.detectIndentation`<br>检测现有缩进 | 打开已有文件时按有效非空行样本推断缩进；置信度不足采用设置。检测结果在状态栏可见，可手动覆盖。 | boolean；false（关闭）；true（开启） | true | 下次打开文件；手动重新检测即时 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `editor.autoIndent`<br>自动缩进 | keep在Enter时延续当前行前导空白；none不补缩进。Markdown结构化列表处理独立于此项。 | enum；keep（保持上一行缩进）；none（关闭） | keep | 立即生效 | 无 | user / workspaceLocal / documentSession | 设计建议 / P0 |
| `editor.multiCursorModifier`<br>鼠标添加光标修饰键 | 默认Alt+单击添加光标；可改Ctrl。Ctrl被用于多光标时，打开链接改用Alt+单击并更新提示。 | enum；alt（alt）；ctrl（ctrl） | alt | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `editor.copyEmptySelection`<br>空选区复制当前行 | 源码中没有选区时，Ctrl+C/Ctrl+X复制/剪切整行；WYSIWYG中空选区不删除整个块。 | boolean；false（关闭）；true（开启） | true | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `editor.smoothScrolling`<br>平滑滚动 | 开启短时滚动插值；关闭时直接响应滚轮/触控板增量。减少动态效果开启时强制关闭。 | boolean；false（关闭）；true（开启） | false | 立即生效 | appearance.reducedMotion != on；system时取Windows实际状态 | user / workspaceLocal | 设计建议 / P0 |
| `editor.scrollBeyondLastLine`<br>末行后保留滚动空间 | 允许末行滚动到视口中部；关闭时滚动范围止于内容末端。 | boolean；false（关闭）；true（开启） | false | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |

### Markdown

| ID / 名称 | 作用 | 类型 / 可选值 | 建议默认值 | 生效时机 | 依赖 | 作用域 | 状态 / 优先级 |
|---|---|---|---|---|---|---|---|
| `markdown.defaultView`<br>Markdown默认编辑方式 | 新打开的Markdown默认采用可直接编辑的排版视图；源码按钮>_切换当前区域。已恢复区域沿用原模式。 | enum；wysiwyg（Markdown编辑）；source（源码） | wysiwyg | 下次打开尚未记录视图状态的Markdown | 无 | user / workspaceLocal | 已确认方向 / P0 |
| `markdown.fontFamily`<br>Markdown正文字体 | 仅控制WYSIWYG正文；system使用Segoe UI与Microsoft YaHei UI等Windows系统回退。代码块仍使用源码与代码字体。 | string；自由文本；字体名列表或system；不存在的字体按回退规则处理 | system | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `markdown.fontSize`<br>Markdown正文字号 | 控制段落基准字号；标题按设计比例派生。 | integer；12–28 DIP；步长1 | 16 DIP | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `markdown.lineHeight`<br>Markdown正文行距 | 控制段落行高倍数；列表、引用和表格采用相同文字基线。 | number；1.3–2.2 倍；步长0.1 | 1.7 倍 | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `markdown.contentWidth`<br>Markdown正文宽度 | 单栏排版的最大正文宽度；区域较窄时自动缩小，不强制扩大窗口。Zen可复用此宽度。 | integer；480–1200 DIP；步长40 | 760 DIP | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `markdown.syncScroll`<br>关联区域同步滚动 | 同一文档的源码与WYSIWYG区域按源码锚点跟随主动滚动区域；手动关闭后各自滚动。 | boolean；false（关闭）；true（开启） | true | 立即生效 | 同一文档同时在至少两个区域中显示；两区域模式不同 | user / workspaceLocal / documentSession | 设计建议 / P0 |
| `markdown.remoteImages`<br>远程图片加载 | ask显示占位及加载按钮，选择加载后只加载该资源；always按文档请求自动加载；never保持占位。相对本地图片正常显示。 | enum；ask（点击后加载）；always（自动加载）；never（不加载） | ask | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `markdown.formatToolbar`<br>格式工具栏 | onSelection仅在选择正文后显示小型格式工具栏；hidden仅通过菜单、命令或Markdown输入操作。 | enum；onSelection（选中文字时显示）；hidden（隐藏） | onSelection | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |

### 文件

| ID / 名称 | 作用 | 类型 / 可选值 | 建议默认值 | 生效时机 | 依赖 | 作用域 | 状态 / 优先级 |
|---|---|---|---|---|---|---|---|
| `files.defaultEncoding`<br>新文件默认编码 | 只影响新文档保存的初始选择；已有文件维持检测/手动指定的编码。编码不可表示字符时阻止保存并提供UTF-8或另存。 | enum；utf-8（UTF-8 无BOM）；utf-8-bom（UTF-8 含BOM）；utf-16le（UTF-16 LE 含BOM）；utf-16be（UTF-16 BE 含BOM）；gb18030（GB18030）；gbk（GBK）；windows-1252（Windows-1252） | utf-8 | 新建文档；不转换已打开文件 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `files.defaultEol`<br>新文件默认换行 | 只影响新文档；已有文件保留原换行。混合换行的统一转换需要显式命令。 | enum；CRLF（CRLF）；LF（LF） | CRLF | 新建文档；不转换已打开文件 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `files.autoGuessEncoding`<br>尝试猜测无BOM编码 | UTF-8严格解码失败后尝试提示候选编码；关闭时直接要求选择。猜测不能覆盖用户手动选择，不以替换字符静默保存。 | boolean；false（关闭）；true（开启） | false | 下次打开文件 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `files.autoSave`<br>自动保存 | off只在显式保存时写原文件；afterDelay停止输入后保存；onFocusChange编辑区域失焦后保存。未命名文档仍进入恢复区，不自动挑选磁盘路径。 | enum；off（关闭）；afterDelay（延迟后）；onFocusChange（失去焦点时） | off | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `files.autoSaveDelay`<br>自动保存延迟 | 最后一次已提交编辑后经过此时间触发保存；输入法组合中不写半成品。外部冲突暂停自动保存。 | integer；500–60000 ms；步长100 | 1000 ms | 立即生效 | files.autoSave == afterDelay | user / workspaceLocal | 设计建议 / P0 |
| `files.trimTrailingWhitespace`<br>保存时移除尾随空白 | 开启会修改正文，仅在显式保存事务中执行并可一次撤销；Markdown双空格硬换行不移除。 | boolean；false（关闭）；true（开启） | false | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `files.insertFinalNewline`<br>保存时补末尾换行 | 仅在末尾不存在换行时补一个，不额外删除空行；变更进入撤销记录。 | boolean；false（关闭）；true（开启） | false | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `files.externalChange`<br>未修改文件的外部变更 | reloadClean仅在文档未修改时自动重载并保留位置；ask对未修改文档也先提示，用户确认后才重载。已修改文档不论此值均展示冲突，不自动覆盖。 | enum；reloadClean（未修改时自动重载）；ask（始终询问） | reloadClean | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |

### 恢复

| ID / 名称 | 作用 | 类型 / 可选值 | 建议默认值 | 生效时机 | 依赖 | 作用域 | 状态 / 优先级 |
|---|---|---|---|---|---|---|---|
| `session.restoreOnStartup`<br>启动时恢复上次工作 | 启动时恢复窗口、根目录、标签、编辑组、模式及位置；路径失效保留恢复项。与session.restoreUnsaved均开启且恢复快照成功刷盘时，窗口关闭可保留未保存文档；关闭本项不删除恢复日志。 | boolean；false（关闭）；true（开启） | true | 关闭窗口及下次启动时读取当前值；不关闭现有视图 | 无 | user | 设计建议 / P0 |
| `session.restoreUnsaved`<br>自动恢复未保存内容 | 启动时自动挂载未保存恢复副本；关闭后仍保留记录并可从恢复中心访问。任一恢复开关关闭时，关闭最后dirty文档所在窗口走保存/放弃/取消，不默默隐藏内容。 | boolean；false（关闭）；true（开启） | true | 关闭窗口及下次启动时读取当前值；不关闭现有视图 | 无 | user | 设计建议 / P0 |
| `recovery.intervalSeconds`<br>未保存内容恢复间隔 | 定期将已提交编辑记入恢复日志；与保存原文件的自动保存独立。组合输入未提交内容不构成已持久化承诺。 | integer；1–10 秒；步长1 | 1 秒 | 立即生效 | 无 | user | 设计建议 / P0 |
| `recovery.retentionDays`<br>已解决恢复记录保留期 | 已保存/已确认放弃后的恢复历史保留天数；仍未处理的恢复副本不按此期限自动删除。 | integer；1–90 天；步长1 | 30 天 | 立即生效 | 无 | user | 设计建议 / P0 |

### 窗口与标签

| ID / 名称 | 作用 | 类型 / 可选值 | 建议默认值 | 生效时机 | 依赖 | 作用域 | 状态 / 优先级 |
|---|---|---|---|---|---|---|---|
| `window.openFiles`<br>外部打开文件的位置 | 通过资源管理器或文件关联打开文件时，使用最近活动窗口或新窗口；对话框明确指定的位置优先。 | enum；current（最近活动窗口）；new（新窗口） | current | 立即生效 | 无 | user | 设计建议 / P0 |
| `window.openFolders`<br>打开文件夹的位置 | 每窗一个根文件夹。new在新窗口打开；current只替换当前根树与搜索范围，保留已打开标签、dirty文档和视图，不触发文档关闭的保存确认。 | enum；new（新窗口）；current（当前窗口） | new | 立即生效 | 无 | user | 设计建议 / P0 |
| `window.restoreGeometry`<br>恢复窗口位置与大小 | 启动时还原可见屏幕范围内的位置、大小和最大化状态；显示器缺失时回到主屏可见区。 | boolean；false（关闭）；true（开启） | true | 下次打开窗口 | 无 | user | 设计建议 / P0 |
| `workbench.previewTabs`<br>启用预览标签 | 文件树单击复用当前组的预览标签；双击、编辑或固定后成为常驻标签。未保存标签不得被替换。 | boolean；false（关闭）；true（开启） | true | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `workbench.tabCloseButton`<br>标签关闭按钮 | all始终显示，active仅活动标签显示，hover在悬停或键盘焦点时显示。修改标记与关闭按钮不得互相遮蔽。 | enum；all（全部标签）；active（活动标签）；hover（悬停或聚焦时） | all | 立即生效 | 无 | user | 设计建议 / P0 |
| `workbench.showStatusBar`<br>显示状态栏 | 显示编码、换行、语言、位置和文件状态；Zen中的隐藏只是临时覆盖。 | boolean；false（关闭）；true（开启） | true | 立即生效 | 无 | user | 设计建议 / P0 |

### 查找

| ID / 名称 | 作用 | 类型 / 可选值 | 建议默认值 | 生效时机 | 依赖 | 作用域 | 状态 / 优先级 |
|---|---|---|---|---|---|---|---|
| `search.matchCase`<br>默认区分大小写 | 新建查找会话的大小写开关；当前查找栏操作可以覆盖，不改变文档。 | boolean；false（关闭）；true（开启） | false | 新建查找会话；在查找栏操作即时 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `search.wholeWord`<br>默认全字匹配 | 关闭即允许部分匹配；开启按既定Unicode词边界规则匹配，不启用中文语义分词。 | boolean；false（关闭）；true（开启） | false | 新建查找会话；在查找栏操作即时 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `search.regex`<br>默认正则表达式 | 新建查找会话启用PCRE2约定语法；WYSIWYG开启正则时会原位切源码并清楚显示范围变更。 | boolean；false（关闭）；true（开启） | false | 新建查找会话；在查找栏操作即时 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `search.wrapAround`<br>查找到末尾后循环 | 到最后/最前匹配后回到首/末项，并给出一次轻量提示。 | boolean；false（关闭）；true（开启） | true | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `search.workspaceExclude`<br>文件夹搜索排除规则 | 相对根文件夹的glob数组；只影响搜索，不隐藏文件树，不改动文件。界面显示排除已启用，可本次忽略。 | stringArray；glob字符串数组；逐条校验；空数组表示不额外排除 | [".git/**", "node_modules/**", "target/**"] | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `search.followSymlinks`<br>搜索跟随符号链接 | 开启后仍使用规范路径去重与循环检测；不会扩大到用户未选根目录外。 | boolean；false（关闭）；true（开启） | false | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `search.maxResults`<br>文件夹搜索结果上限 | 达到上限停止收集并明确提示已截断；不展示虚假的完整匹配数。可继续缩小范围重新查找。 | integer；100–100000 项；步长100 | 20000 项 | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |

### 大文件

| ID / 名称 | 作用 | 类型 / 可选值 | 建议默认值 | 生效时机 | 依赖 | 作用域 | 状态 / 优先级 |
|---|---|---|---|---|---|---|---|
| `performance.largeFileThresholdMiB`<br>大文件模式阈值 | 文件大小达到或超过此阈值（>=）时提供大文件优化路径；延迟非必要高亮/布局。WYSIWYG不静默切源码，仅提供明确可选的源码入口；为性能建议起点，须实测调整。 | integer；5–100 MiB；步长5 | 20 MiB | 下次打开文件；可通过当前文档命令重评 | 无 | user / workspaceLocal | 设计建议 / P0 |
| `performance.hugeFileThresholdMiB`<br>超大文件提示阈值 | 文件大小达到或超过此阈值（>=）时先做资源预估，提供只读流式查看与尝试编辑选项；不把只读模式宣称为完整编辑能力。 | integer；50–2048 MiB；步长10 | 200 MiB | 下次打开文件 | 值必须大于 performance.largeFileThresholdMiB | user / workspaceLocal | 设计建议 / P0 |
| `performance.longLineThresholdChars`<br>超长行保护阈值 | 逻辑行Unicode标量数量达到或超过阈值（>=）时，按可见片段绘制并暂停该行高亮/软换行；状态条说明临时降级，原内容不截断。 | integer；5000–1000000 字符；步长1000 | 20000 字符 | 立即生效 | 无 | user / workspaceLocal | 设计建议 / P0 |

### Zen

| ID / 名称 | 作用 | 类型 / 可选值 | 建议默认值 | 生效时机 | 依赖 | 作用域 | 状态 / 优先级 |
|---|---|---|---|---|---|---|---|
| `zen.hideTabs`<br>Zen中隐藏标签栏 | 进入Zen时临时隐藏标签栏；退出恢复此前布局。 | boolean；false（关闭）；true（开启） | true | 立即生效 | 无 | user | 设计建议 / P0 |
| `zen.hideSidebar`<br>Zen中隐藏侧栏 | 进入Zen时临时隐藏文件夹侧栏；退出恢复此前布局。 | boolean；false（关闭）；true（开启） | true | 立即生效 | 无 | user | 设计建议 / P0 |
| `zen.hideStatusBar`<br>Zen中隐藏状态栏 | 进入Zen时隐藏常规状态栏；保存失败、恢复冲突仍保留可见提示。 | boolean；false（关闭）；true（开启） | true | 立即生效 | 无 | user | 设计建议 / P0 |
| `zen.centerContent`<br>Zen中正文居中 | WYSIWYG采用Markdown正文宽度居中，源码采用当前编辑宽度；不改变文件换行。 | boolean；false（关闭）；true（开启） | true | 立即生效 | 无 | user | 设计建议 / P0 |
| `zen.enterFullScreen`<br>Zen进入全屏 | 开启时进入Zen同时进入全屏；关闭时保留普通窗口，方便多窗口办公。 | boolean；false（关闭）；true（开启） | false | 立即生效 | 无 | user | 设计建议 / P0 |

### 恢复开关与关闭的组合规则

`session.restoreOnStartup`控制布局/会话自动恢复，`session.restoreUnsaved`控制未保存副本自动挂载。它们都不关闭崩溃日志，也不删除已有恢复记录。

| 启动恢复 | 自动恢复未保存 | 恢复快照刷盘 | 关闭窗口中的最后dirty文档 |
|---|---|---|---|
| 开 | 开 | 成功 | 保留恢复记录并关闭，下次启动自动恢复 |
| 开 | 关 | 任意 | 保存 / 放弃 / 取消；保留的记录可在恢复中心访问 |
| 关 | 开或关 | 任意 | 保存 / 放弃 / 取消；不自动恢复布局 |
| 开 | 开 | 失败 | 保持窗口可见，提供重试恢复写入、保存到文件或取消关闭；不显示已保留 |

其他窗口仍引用同一文档时仅移除关闭窗口的视图。显式关闭最后一个dirty标签始终执行保存/放弃/取消；放弃写tombstone，不能下次自动恢复。对窗口关闭流程用户明确选择放弃的文档遵循相同规则。打开、替换或关闭根目录仅改变根树和搜索范围，保留已有标签与dirty内容。

## 4. 关联规则与边界

- **编码与换行**：默认编码/换行只决定新文件初始状态。已存在文件的BOM、有效编码、CRLF/LF及混合换行由文档元数据保留；用户必须显式转换。`files.autoGuessEncoding=false`不禁止BOM识别和严格UTF-8验证，只禁止将无把握的猜测直接当结果。已识别的UTF-16应保留端序与BOM状态。GB18030具体标准版本由技术方案固定并用样本验收。
- **自动保存与恢复**：自动保存写原始文件，恢复机制写恢复日志，两者分开。自动保存关闭不关闭恢复。未命名内容不被自动分配任意路径；已修改内容遇到外部变化一律暂停自动保存并展示冲突处理。
- **Markdown双模式**：`markdown.defaultView`只决定无记录视图的初始模式。单文档不同编辑区可以分别显示WYSIWYG或源码。正文右上`>_`切换当前区域，不改写文件、不清空撤销、不创建模式切换的撤销步骤。
- **查找范围**：WYSIWYG普通查找查可见正文，界面标“正文”；开启正则、替换或选择全部匹配时先切源码，明确提示“搜索范围已切换为源码”，重新计算结果。全字关闭就是部分匹配；首版不做中文语义分词。正则采用PCRE2 UTF/UCP及受限后台任务，正则语法和替换语法以技术方案为准，原型中的JS RegExp只能标为演示。
- **缩进**：默认4列并使用空格；检测缩进只影响已打开文档的有效值，不反向修改用户默认。JSON不设置隐含2列例外。状态栏手动选择优先于检测结果。
- **字体与滚动**：字体缺失有可见回退说明。平滑滚动默认关闭；减少动态效果优先于平滑滚动。Win10触控板高分辨率滚动不依赖开启插值才能使用。
- **大文件**：20/200 MiB与20000 Unicode scalar超长行均以达到或超过阈值（>=）触发，均为建议起点，性能测试后可改；它们不构成已经实现的性能保证。阈值变化不在用户连续输入中突然重建整个编辑器。轻量模式可暂停高亮和排版，但必须清楚展示可用能力。
- **图片**：本地相对图片正常显示；远程图片默认占位按需加载。HTML、数学、Mermaid等未经实现的能力使用原样源码块退化，不在设置中给出不存在的开关。
- **首版范围**：不提供插件、语言服务器、调试、AI识别或内容敏感性判断设置。设置表也不把未来架构预留变成首版功能。

## 5. 验收清单

1. 以空用户配置启动，逐项检查本表63项默认值与JSON一致；Markdown默认WYSIWYG，源码及代码16 DIP，自动保存off，恢复on。
2. 修改用户字号与文档wrap覆盖，重启后恢复；切到其他文档能正确显示其自己的有效值来源。
3. 输入越界数值、无效glob、依赖逆序阈值，必须展示具体错误且不写入无效配置。
4. 移除某个字体后打开应用，按回退链显示，不能丢失中文、错位光标或截断组合字符。
5. 自动保存关闭情况下强制结束应用，已持久化恢复记录仍可恢复；关闭自动恢复后记录仍可在恢复中心访问。
6. 更改新文件编码/EOL后检查新旧文件分别行为；修改设置不悄悄转换已有文件。
7. WYSIWYG普通Ctrl+F保持排版态；Alt+R或Ctrl+H转源码时范围标签、计数和锚点同步更新。
8. 开启Windows高对比度/减少动画、切换100%/150%/200%DPI，控件与光标仍可辨，焦点不丢失。
9. 让配置目录只读或注入写入失败，设置页必须报告失败并保留待保存值；不能只改内存而提示成功。
10. 当前文档不受设置支持的语法必须保留原文，不出现隐式格式转换或新增敏感内容判断。

## 6. 依据与追溯

本文件中的默认值和产品行为是texzi设计建议，不从VS Code现行默认值推导既定需求。Windows快捷键基准与版本证据见 [COMMANDS.md](COMMANDS.md)。Markdown原文保留策略参考Quiet.md的优点和限制，但不直接采用其整篇canonical导出作为切换实现。

- [VS Code 1.146官方发布说明](https://code.visualstudio.com/updates/v1_146)。
- [Quiet.md v0.1.12 Markdown解析/保存](https://github.com/s1dashu/quiet-md/blob/v0.1.12/crates/core/src/markdown.rs)：其preserve失败会回退canonical，因此texzi需坚持源码为唯一真实数据。
- [Quiet.md v0.1.12架构](https://github.com/s1dashu/quiet-md/blob/v0.1.12/docs/ARCHITECTURE.md)。
