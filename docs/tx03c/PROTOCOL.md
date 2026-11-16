# document-probe 观察协议扩展 / Observer schema1

沿用JSON-lines v1，新增obs_操作；原T03b接口和返回保留。全部为内存逻辑组件，非网络服务/真实IO/NSH。
可以自行编写客户端读取JSON事件并维护自己的完整状态，也可显式驱动内置缓存测试半批次/错误帧。
包内另附OBSERVER-PROTOCOL.md、本驱动test-observer-probe.py；旧PROTOCOL.md对应原T03b操作。

cursor为`{store:{session:[两个16位hex字符串],instance:u64},doc:文档别名,scope:{schema:1,views:[排序别名]},seq:u64}`。
Doc/View别名在进程内解析为不透明ID，store nonce使重启后的同别名不能复用旧身份。
JSON整数要求无损u64处理（Python/Rust可用），不能用浮点近似水位。scope输入views经显式planner规范化。

## 服务端操作

| op | 参数/返回 |
|---|---|
| obs_config | 开Doc前设置journal、queue_max、subscriptions_max；范围见README，不改已有流容量 |
| obs_observe | doc,views,capacity,snapshot新别名,handle新别名；返回同点cursor/subscription/state，原子注册W+1 |
| obs_snapshot | snapshot别名；读取冻结快照，绝不补读最新cursor |
| obs_poll | handle,poll新别名；返回Batch/Pending/Lagged/DocGone，保存原始数据供后续喂缓存 |
| obs_unsubscribe | handle；关闭注册，不推进文档水位 |
| obs_resume | doc,views,capacity,handle新别名,cache；要求内置cache完整可resume，注册并rebind新句柄 |
| obs_resume_raw | doc,views,capacity,handle新别名,cursor完整JSON；外部客户端须已有匹配完整状态，可作身份/旧游标反例 |
| obs_close_document | doc；测试内存owner生命周期，active save拒绝，非用户文件关闭授权 |

schema可显式传入制造UnsupportedSchema/ScopeMismatch。所有历史别名保留，复用别名拒绝，避免覆盖证据。
错误延续`ok:false,error:Debug字符串`，测试可核错误变体前缀；公共Rust API提供结构化ObserverError。
尤其CursorTooOld/Lagged错误诊断范围不授权改cache cursor；必须重新observe并安装所有状态。

Batch含cursor、subscription、events和frames。events为Content/Global/View，空投影时[]。
Content含PublicationId、group、previous/revision/token、forward/inverse；所有delta以对应完整版本为基准。
Global含immutable document（source/serialized_utf8/EOL/profile/provenance）、baseline/dirty、attention/save/history。
View为Open的独立逻辑状态及明确NotImplemented域，或Closed墓碑。来源sources/spans只用于原字节依据，
不是第二个可编辑正文或可直接提交的OriginalByteOffset。

frames按Begin(parts)、Part(index,event)、End(parts)排列；不要把Part索引当成恢复水位。
Lagged携带当前cursor诊断、registered_at、earliest、head、capacity；DocGone也携带原注册身份。

## 接收缓存操作

| op | 参数/作用 |
|---|---|
| obs_cache_prepare | snapshot,cache新别名,buffer_frames；尚未安装，state=null，允许提前缓冲 |
| obs_cache_install | cache；一次安装所有域及W，再处理缓冲，stale拒绝 |
| obs_cache_state | cache；返回cursor、installed/stale/partial和最后完整state；不读取服务器最新状态 |
| obs_cache_resume_cursor | cache；完整/非stale/无半批次时才返回resume凭据 |
| obs_cache_frame | cache,poll(Batch别名),frame(0-based)；显式喂一个真实保存帧，返回outcome及cache |
| obs_cache_control | cache,poll(Lagged/DocGone别名)；标stale但不跳水位，旧句柄控制消息忽略 |

可在obs_cache_frame覆盖seq/subscription/schema/frame_doc，覆盖Begin/End的parts或Part的part_index，
或unknown=true制造未知帧。原poll数据不修改；用于检查身份先于序号、计数冲突、缺片和去重。
漏喂一片/跳过一批即可构造缺口；从另一poll喂真实帧并覆身份也可构造重复index内容冲突。
也可只消费真实JSON frames，在独测自己的客户端实现恢复，不必复用实现者cache。

## 初订阅与半批次示例

先用原接口open d/open_view a，再：

```json
{"op":"obs_observe","doc":"d","views":["a"],"capacity":8,"snapshot":"s","handle":"h"}
{"op":"obs_cache_prepare","snapshot":"s","cache":"c","buffer_frames":128}
```

在服务器做一笔正文编辑，`obs_poll(handle=h,poll=p)`取得Batch；按frame索引逐一喂入c。
安装前state必须保持null；或先install再只喂Begin/部分Part，完整state/cursor必须保持安装点。
直到正确End才能推进一个水位；重喂整批不重复apply正文；End缺片/同index冲突则stale。

EX14：snapshot捕获save Encoding/Verifying，断开订阅，模拟保存完成（多个同revision阶段）再resume；
重放中baseline/dirty/save/history必须按水位完整恢复，不能仅比较revision。
EX15：只变未消费View且全局history未变，必须收到空Batch并推进W；旧句柄大序号不覆盖新scope。
EX16：journal=2、capacity=1时连续发布造成Lagged；旧cursor过期或第二次Lagged都要新observe，
不能cursor=head而保留旧baseline/save/View。

驱动命令（新trace目录，不能覆盖旧round）：

```powershell
python component-tests/test-observer-probe.py --probe component-tests/document-probe.exe --trace-dir evidence/generated-observer-rounds-new
```

原始stdout.bin先保存再JSON解析，逐条trace、stderr.bin、identity.json和exit.json独立保留。
驱动只是实现者的测试参考；实际CI/独测结果与范围必须绑定各自commit/run/exe hash另记。
