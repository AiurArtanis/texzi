# texzi 本次交付验证报告

日期：2026-11-15。验证对象为**设计包及浏览器原型源码**，不是尚未实施的 Windows 原生应用。

## 1. 状态定义

- **Pass**：本次实际执行且检查结果通过，注明对象与证据。
- **NotRun**：提供了流程/脚本，但本次没有执行，不能计为通过。
- **Blocked**：尝试的验证受到当前环境限制；记录具体原因，不反复换路线规避。
- **设计待验证**：实现团队须完成的原生能力或性能Gate。

## 2. 已完成检查

| 对象 | 本次结果 | 证据与实际范围 |
|---|---|---|
| JavaScript与HTML结构 | Pass，非视觉检查 | 原型及手册所有内联脚本通过Node --check；静态ID无重复；prototype/src/app.js语法无错误 |
| 纯逻辑回归 | **31/31 Pass** | node --test tests/core.test.cjs；根Agent复核运行0失败/0跳过，约192ms；耗时仅为测试运行时间，不是产品性能 |
| 搜索语义 | Pass，限纯函数 | 字面、大小写、全字、中文/组合附加符、Unicode scalar零宽、EOF、错误正则、截断标记、捕获及错误替换引用 |
| Markdown基础模型 | Pass，限纯函数 | 源区间、CRLF、末行、空白间隙、未知语法、未闭合fence、渲染不修改源码/历史 |
| 渲染输出转义 | Pass，限字符串输出 | 原始HTML、代码、链接属性不会从HTML字符串中逃逸；不是完整浏览器安全审计 |
| 命令/设置目录 | Pass | 115/63条唯一ID；来源引用；默认值类型/范围；七类导航；固定VS Code基线与源码模式转换 |
| 文档一致性 | Pass，静态复核 | x64、Zen、WYSIWYG、750ms、字体/行高、20/200MiB、Unicode长行阈值、恢复组合条件及外部变更ask分支统一 |
| 原生验收用例定义 | Pass，定义检查 | AC-001–AC-112 连续唯一，提供前提/动作/预期；用例执行状态全部NotRun |
| 性能样本生成器 | Pass，smoke / standard / stress | smoke两次同seed manifest一致；18文件/2,298,632 bytes，standard 27文件/464,720,648 bytes，stress 29文件/674,435,848 bytes。hash、尺寸、行统计和严格解码验证通过，故意无效输入按预期拒绝；清单见review/fixture-manifests/ |
| 浏览器脚本 | Pass，语法/清单 | tests/browser_smoke.py语法检查及--list成功；11个流程保持NotRun |
| 离线构建与文件包 | Pass，构建/完整性 | build.py和build_handbook.mjs构建成功；validate_package.py无错误；不依赖网络资源；各文件校验和列于manifest-sha256.json |

本轮修正了两项由测试暴露的真实逻辑问题：全字边界加入Unicode组合附加符；未知数字/命名捕获引用会明确报错，不会被悄悄替换为空。另在源码审查中补了搜索截断/错误时阻止替换、关闭最后dirty视图确认与七类导航。

## 3. 当前环境未完成的检查

| 验证 | 状态 | 具体限制与后续方式 |
|---|---|---|
| 原型实际浏览器渲染/截图 | **Blocked** | 本地Chromium启动受到socket权限限制；Browser连接超时；Cloud Browser对file协议页面不授予访问。不绕过环境策略。 |
| 原型11个真实UI流程 | **NotRun** | 提供browser_smoke.py，实施团队在自己的允许环境运行；未用fake DOM或截图代替。 |
| Windows原生构建与运行 | **NotRun** | 当前交付是设计和浏览器原型，无texzi原生二进制；T00开始实施。 |
| 中文输入法、候选窗与TSF | **NotRun** | 浏览器普通文字输入和纯函数Unicode测试不等于真实IME；执行AC/TP对应真机矩阵。 |
| Windows DPI/多窗口/UIA | **NotRun** | 需Win10 x64本机与多显示器、实际原生窗口和辅助技术。 |
| 文件原子保存/崩溃恢复/文件监听 | **NotRun** | 原型状态由演示按钮触发；正式实现须进行故障注入、字节hash与竞争测试。 |
| 所有原生性能目标 | **NotRun** | 无texzi Release测量；上游NSH/quiet.md数据、原型响应和逻辑测试耗时均不能替代。 |

这不是要求用户再做产品澄清。下一团队可以立即执行HANDOFF任务，在许可的开发/测试机器上关闭这些验证项。

## 4. 复现命令

在解压后的交付目录：

~~~powershell
py -3 tools\build.py
node --check prototype\src\app.js
node --test tests\core.test.cjs
py -3 tests\browser_smoke.py --list
py -3 tools\validate_package.py
~~~

真实UI验证需要Playwright与浏览器；在允许的环境中，先运行本地预览服务器，再按脚本说明传入明确URL和--run。脚本不会默认运行浏览器，也不会自动修改沙箱、发现其他浏览器或换路线重试。

~~~powershell
py -3 tests\browser_smoke.py --run --url http://127.0.0.1:8765/prototype/index.html
~~~

Windows原生的构建、性能和故障注入命令属于未来实施仓库接口，详见HANDOFF与PERFORMANCE_PLAN，不能在此包中假设已有xtask或texzi.exe。

## 5. 后续补验收的记录方式

每次结果保留构建hash、文件manifest、环境、原始输出、失败数与已知限制。浏览器用例通过后可更新其11项状态，但Windows原生112条仍单独记录。性能结果必须给p50/p95/p99/max、样本数和失败/剔除原因；不只写“很快”。

review/acceptance-results.csv是112条初始NotRun的登记表；review/performance-results-template.csv是空测量模板；data/acceptance.json包含机器可读用例。空白和NotRun均有意保留，不代表遗漏的“通过”数据。
