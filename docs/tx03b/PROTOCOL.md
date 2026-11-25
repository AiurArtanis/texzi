# document-probe / JSON-lines v1

这是供独立验证调用的**内存组件入口**，可使用任意语言编写oracle。每行一个UTF8 JSON请求，
stdout每行一个JSON回复，成功`{"ok":true,"result":...}`，拒绝`{"ok":false,"error":"InvalidSelection"}`。
正常EOF exit0，管道I/O错误exit2；拒绝单个请求不退出。没有文件路径读取、GUI或磁盘写入操作。
输入使用生成fixture；输出可能含正文，不把它用于用户文档诊断日志。

包内`component-tests/document-probe.exe`、本说明和test-document-probe.py；
可直接启动进程，通过stdin发送行、读取stdout响应，不需Rust/MSVC开发环境。
外部Python驱动是实现者提供的参考，不是独立测试的唯一途径。`--trace`必须是新路径，避免覆盖历史。

## 身份、字段与响应

- `hello`返回protocol/scope/Unicode/合并配置，不需doc。
- 其它请求的doc是open时选定的唯一字符串别名；View也以全进程唯一别名引用。
- selection数组每项`{"anchor":0,"head":2}`，可增加`anchor_affinity`/`head_affinity`为before/after。
  caret默认After/After；正向范围Before/After，反向After/Before。primary是数组索引，默认0。
- 坐标是**逻辑正文UTF8 bytes**。旧revision/epoch要显式传入，不会由入口替你更新为最新。
- state返回source、serialized_utf8、revision、不透明token字符串（只用于身份相等比较）、dirty、
  undo/redo组数、history_charge和全部views。每个View返回epoch/mode、selection/primary、scroll及snaps。
- 提交回复额外有receipt：previous/revision/changed、forward/inverse。各delta含start/end/text/eols，
  正向范围基于提交前版本，逆向范围基于完整提交后版本；同批多个delta应从后向前apply。
- 拒绝后可state复读，比较正文/token/历史/全部View；不要只凭error名称断言零部分提交。

## 操作

| op | 必填和语义 |
|---|---|
| open | doc,text；新建内存文档，UTF8/BOM/EOL导入，InitialEmpty/内存clean baseline |
| state | doc；读取一致组件状态，不是C08恢复快照 |
| open_view | doc,view,selection；mode可plain/source/rich，scroll/pixel_bias可选；只是逻辑标签 |
| selection | doc,view,epoch,selection；primary可选；normalize=true先显式planner规范化，默认权威严格校验 |
| normalize | doc,selection；返回规范化selection/primary，不改状态 |
| mode | doc,view,epoch,mode；仅逻辑标签，切组，0内容事务 |
| close_view | doc,view,epoch；销毁逻辑View，不复用身份 |
| replace | doc,view,epoch,text,tick；kind=typing/paste/ime/structural，默认structural；从View当前选区规划事务 |
| submit | doc,view,epoch,base,edits,selection_revision,selection,tick；raw ViewTransaction，kind可选，明确验证任意多edit/选区反例 |
| service_submit | doc,base,edits；受信任服务路径，无selection_after覆盖，映射全部View并切组 |
| undo / redo | doc,view,epoch；共享最近组，调用View定位作用选区 |
| fence | doc；显式关闭当前合并组，不增加history/revision |
| save_begin | doc,save,target；内存捕获及lease，target只是调用方生成的协调键，不作路径解析 |
| save_complete_simulated | doc,save；模拟阶段/permit/已验证回执，不写或验证任何文件 |
| save_cancel | doc,save；模拟在不可撤回前取消 |

edits数组每项`{"start":0,"end":1,"text":"X"}`，默认输入规范化CRLF/CR并按提交前主导EOL生成表；
可显式`eols:["Lf","CrLf","Cr"]`制造EOL计划或错误。service_submit可scalar=true，submit_view拒绝scalar策略。
cancelled=true只覆盖入口取消bool；提交前/后故障闸门未实现。submit可selection_doc指定其它已打开doc作错误计划。
selection_revision是期望**提交后实际**revision（NoOp必须原值）；位置未来边界由真实候选正文校验。
不允许在此接口猜测未实现的C08、真实文件身份、read-only权限或系统IME语义。

## 可独立扩展的例子

```json
{"op":"open","doc":"d","text":"abcdef"}
{"op":"open_view","doc":"d","view":"v","selection":[{"anchor":0,"head":0}]}
{"op":"submit","doc":"d","view":"v","epoch":0,"base":0,"edits":[{"start":1,"end":3,"text":"X"},{"start":3,"end":3,"text":"YZ"}],"selection_revision":1,"selection":[{"anchor":4,"head":4}],"tick":0}
```

预期source=`aXYZdef`，revision1，一个undo组；forward两段，inverse分别[1,2)->bc、[2,4)->空。
在原byte3设置Before/After的被动View可分别观察新byte2/4；用EOF或InsideGrapheme的selection_after
制造拒绝时，须确认原source/revision/token/undo和全部View相同。
更多随机范围、Unicode独立预期、EOL表和Undo/Redo分支可由独测自行生成。
