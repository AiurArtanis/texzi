# IME-01 预编辑更新修复边界

独立测试已在28c085a确认：按N时显示n，继续I/H/A/O后候选为你好，但编辑区仍只有n；
候选位置同时下移。提交/取消/Undo等是另外的行为，不能因最后文字正确而忽略预编辑显示。
旧图、失败和独测记录保留，不把裁剪截图本身先判成产品裁切。

代码证据：固定Windows适配process只在Notes.changed时重新report_composition，selection-only
通知不读最新组合区间；编辑器隐藏native multiline sink宽仅1px，存在逐字换行风险。
本修订分别使组合期间每个process turn重新读取并去重预编辑，及给予sink当前行剩余视口宽度。
这是依据症状和源码制定的修复；最终根因/效果须由新候选真实分段输入和事件诊断核实。

TEXZI_TRACE_IME=1为显式测试诊断，默认不启用。只记录native标志/预编辑UTF16长度、render坐标、
owner revision/字节数/操作类别；不记录正文或预编辑内容。验证可读取该轮生成的stderr，完全不用剪贴板。
记录N→NI→NIHAO、Backspace、Escape取消、Space提交、Undo/Redo；组合中不能增加正文事务/history。
实际中文输入法profile/版本、位置/DPI及未覆盖项单列，不因一个场景通过外推完整G4。

Windows fork来源/许可与base fork都随包保留；不改变唯一Store/history或T05/T06/T07优先级。
本修复不是继续扩写复杂C08续传/重放。原组件协议、历史失败和首版范围均保留。
