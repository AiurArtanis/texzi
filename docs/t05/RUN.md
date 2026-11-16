# T05/T06 原生富文本局部回写第一步

此阶段是未验收候选，不代表完整T05/T06或G2通过。首版原范围保留，未实现项不能改成只读预览后宣布完成。

- 启用固定NSH的rich-editor，与原源码Editor同一便携程序。工具栏“源码/富文本”切换同一正文；独立逻辑View共用唯一Store/history，RichEditor starter history明确关闭，Undo/Redo回Store。
- Markdown1.0.0及serde_json1.0.154原已在锁文件，新增直接依赖关系；未升级原包版本。实际features/全部依赖/SBOM/许可继续由候选CI记录。
- 按mdast顶层源byte区间建立投影，保留区间外空白/EOL；单text leaf且原文匹配唯一时做Unicode标量边界最小替换，结构编辑只序列化改变的连续块，不整篇serialize。补丁先重解析并与目标rich树一致，选择边界可严格映射，才提交唯一事务；不支持的操作显示错误，正文保持不变。
- HTML、定义和脚注等无法无损表达的语法作为可见codeBlock源码局部编辑，原件不丢弃；禁止把含opaque的跨块修改偷偷序列化。此局部源码fallback不是所有结构WYSIWYG已完成。
- 每次富文本写入校验doc revision和View epoch，选择更新不增加正文事务。投影仅缓存，原生回调通过UI线程弱注册表访问Owner，无unsafe Send/Sync。
- RichEditor自己的Windows隐藏sink也调整剩余视口宽度；不能继承源码IME测试为rich IME已通过。

## 覆盖与待补

组件新增：混合EOL/未知块旁的标题emoji局部修改、UTF16代理对内严格拒绝、opaque局部原字编辑，以及100次纯投影重建不写Store/token/history/dirty。组件测试的100次不是100次真实GUI切换证明。

真实GUI必须用本轮CI下载的新exe验证输入→切源码→Undo/Redo；旧2d不作新hash运行证明。实际GUI/IME、选区/滚动映射、100轮切换、全部指定结构、位置/性能/失败矩阵均待本轮及后续分项证据，不先写PASS。

当前仍有明确限制：投影每次富写重解析，未做增量CST、稳定BlockId/虚拟化/大文档预算；纯文本leaf以原文唯一匹配定位，复杂转义/重复文本可能拒绝操作；富视图恢复位置与多选能力未完成。已存在源码多选与逻辑独立View保留。上述为PARTIAL，不取消首版要求。T07多编码/EOL/安全写/冲突原范围保留，尚未接磁盘IO。旧剪贴板恢复未证明，本轮严格无剪贴板访问/快捷键C/X/V。

## 构建和产物

README现有cargo build --release --locked --target x86_64-pc-windows-msvc命令不变。CI额外编译执行app单元测试，将真实rich-projection.exe与原双流/exit随包交独测；失败不打成功候选。原文档组件/窗口失败路径证据保留。

补充：固定12语料每例100次无修改投影、跨逻辑View撤销/重做与原混合EOL字节恢复均为组件检查；真实GUI次数另计。显式长度诊断下的TX05_VIEW记录revision/token/history/dirty供核模式切换0文本事务，默认关闭，不输出正文。MIT移植许可补齐与blob来源见LICENSE-SOURCES.json；原NSH未注明移植所用PM/Tiptap revision，不自行编造。
