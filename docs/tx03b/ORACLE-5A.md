# 固定5a14645的Steamy独立oracle补验

这是已有T03a审测链内的补验支持，不回滚/暂停T03b，也不更改产品源码或测试预期。
产品固定`5a14645d38dbab6e6612cea5e0a585c740fb6cd7`，已审harness SHA256
`60D4A5F5BBA71F359488A1F9D27F9A0ECA25E4DB7A0F79F49C17A6D12ECD53F0`。
版本化输入`tests/oracles/5a14645/steamy_oracle.rs`设置-text，不能cargo fmt或换行转换。

在现有tx00.yml以workflow_dispatch `oracle_only=true`执行独立job，正常Release/logging job跳过。
普通push/默认dispatch的既有流程保持；此补验不创建Release/tag/公开发布，不推默认分支。
job以固定Actions checkout完整历史，脚本在RUNNER_TEMP的新隔离目录
`git -c core.autocrlf=false -c core.eol=lf archive`导出完整产品，显式禁止Windows换行转换。
只注入上述原hash测试；Python批量读取Git blob逐文件核原字节，再检查导出文件集合没有其它注入。
manifest明确`transformation=none`，不把原Steamy archive/source-check等同于Git blob核验。

Rust使用导出产品的1.97.0，Cargo显式--manifest-path和--config指向导出文件，--locked，
独立--target-dir；当前T03b源码/依赖/feature不参与这次5a编译。MSVC/SDK选择和实际oracle链接原文
单独记录。编译`-p texzi-document --test steamy_oracle --release --no-run --message-format=json`，
保留混合stdout原文、仅解析JSON记录以找到唯一exe；将真实exe复制进artifact后执行。
不改五个测试来追求通过。失败也上传可用exe、源码清单、lock/harness/编排脚本和原始双流/真实exit。

artifact identity严格区分product_commit、harness_sha256、orchestration_commit、新run/attempt，
exe和artifact下载后另算hash。不使用旧run43823763825或原document-contracts.exe的hash。
artifact还含fixed-product-source.zip、精确源码manifest、原harness、Cargo.lock/toolchain、
构建及最终链接日志和artifact-files.sha256.json。status只记录构建/执行结果，不是产品最终验收。
原Steamy须独立下载本机执行后交原Rusty；此前五项仍保持原环境BLOCKED记录，不回写历史。

本机只进行输入导出/字节校验和脚本静态检查，不重复已知失败的本机cargo test，不安装MSVC。
仅PALADIN开发范围；不读联系远端设备，不新建任务或改变权限/模型。内部交接不等于群汇报。
