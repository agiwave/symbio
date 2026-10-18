// docs 阶段：静态审计守卫。
// 判定型守卫的**回归测试**先跑：一个只会亮绿灯的守卫等于没有守卫——它腐烂的
// 方式恰恰是「规则写错了所以永远不命中」，只有注入真实违规并断言脚本变红，
// 才能区分「通过」与「没在工作」。
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, '..', '..')

// 带回归测试的判定型守卫。
// ⚠️ 这两份名单**手写**是刻意的（每条都带着「为什么值得跑」的说明），但手写就有漏登：
// `gate-wiring-audit`（GW-001…003）拿 `scripts/**/*.test.mjs` 的目录事实与它们对账，
// 所以下面两处 `export` 不是给人读的出口，是给那条守卫读的输入。
export const GUARDS = [
  'grep-audit',
  'mechanism-audit',
  'plugin-entry-audit',
  'protocol-mirror-audit',
  'style-audit',
  'doc-link-audit',
  'test-layout-audit',
  'dead-code-audit',
  // `core-naming-audit` 把 `symbio_core/README.md` §1.2 的**域前缀对照表**当作规则源
  // 解析，逐个核对公开符号的前缀是否落在所属域登记的前缀里。它守的是「规范」本身：
  // 此前那张表**没有任何脚本在判**，于是漂移了三处（`PLUGIN_PROVIDER_FIELD` 住 `vdfs`
  // 却姓 `PLUGIN`、`KEY_PROVIDER` 住 `plugin` 却用 `keys` 的前缀、插件工厂 id 住 `keys`
  // 却姓 `PLUGIN`）。它的失效形态有两种，都要靠回归测试钉住：
  // ① **规则源读不出来**（表被改写/搬走）却仍亮绿灯——绿灯只说明没检查；
  // ② **前缀比对写宽了**（按字面 contains 而非按词）⇒ 误报一片，最后被人用豁免喂到失效。
  'core-naming-audit',
  // `core-export-audit` 判 `symbio_core` 的**架构出口有效性**，两条来自架构要求：
  // 「子目录不许对外 `pub`，必须在根显式输出」（C-001）与「根输出的每个符号必须被
  // ≥2 个模块消费」（C-002 是深引检查的全量版，**取代了 plugin-entry-audit 的 E-010**：
  // 域目录动态取自 core、连裸 `use …::symbio_core::<域>;` 一起拦，判定方只留一个）。
  // 失效形态两头都要钉：① 规则写宽 ⇒ 深引/单消费方照样过（守卫永不命中）；
  // ② C-003 的棘轮基线写死成大数 ⇒ 存量只进不出，机制退化成装饰——测试用
  // `CORE_EXPORT_BASELINE=0:0` 把基线压到零，证明它**真的会红**。
  'core-export-audit',
  // `core-shape-audit` 判 `symbio_core` 的**公共形状**（CS-001 三格棘轮：`pub trait` 数 /
  // `fn` 成员总数 / 带默认体的成员数；CS-002：放宽数字必须挂一条能打开的 ADR）。
  // 它补的是 `08 §0 纪律 3`「架构改动必须逐条论证必需」唯一的真缺口：那条纪律的判据一直写着
  // 「人工核对」，而 C2 那一批（把一个 trait 扩成十个方法、多挂 9 个默认体）在既有全部
  // 判据下**符号数不变、根出口不变 ⇒ 门禁全绿**。失效形态两头都要钉：① 提取写宽
  // （漏数默认体 / 把默认体内的 `fn` 当成成员）⇒ 形状变宽照样绿；② 基线写成大数
  // ⇒ 只进不出，机制退化成装饰。测试用 `CORE_SHAPE_BASELINE=0:0:0` 压到零证明它真的会红。
  'core-shape-audit',
  // `doc-symbol-audit`（D-005）判「**现行文档指认的 Rust 符号确实存在**」：
  // 反引号内 ≥2 段的 `module::symbol` / `Type::method` 形态，取末段查整词
  // 是否出现在 .rs 语料（359 文件 / 203 条指认）。这类失效读者看得见、守卫
  // 看不见：doc-link 只管链接、facts/current 只生成结构表，都不校验散文
  // 里的符号指认——校准日一次就抓出五处改名残留（`KEY_PAYLOAD` →
  // `PLUGIN_PAYLOAD_KEY`、`HOOK_FIRE` → `ROUTE_HOOK_FIRE` 等，先修后建）。
  // 豁免出口只对应「文档说的是历史」：`docs/010-adr/`（ADR 正文）与 `archive`
  // 行内历史词、`<!-- doc-symbol-allow: 理由 -->` 承认通道（空理由不算）。
  // 回归测试双向钉住：违规必须变红，注释/单段词/ADR 不得误报，范围读不出必须 exit 1。
  'doc-symbol-audit',
  // `doc-count-audit`（D0-001…003）判「文档里少数几个『几个』的数 = 代码里的数」：
  // `check_all` 条数（数 `invariants/mod.rs` 的函数体）、机制表顶层键数与含子键
  // 键数（读 M1 的生成物 `verify/facts/mod.rs`，不重新解析 01 §8——提取实现只有一份）。
  // 它的存在是因为 `plan/13` 的 D3 / D4 出口写着「由**新增计数守卫**守」，而那个守卫
  // 此前不存在——两批会退化成一次性人工对账（纪律 4 的直接来源）。
  // 只判**三个封闭名字**是刻意的：给本仓几百个数字都配来源就是写第二份真相。
  'doc-count-audit',
  // `slo-exit-audit`（plan/13 批 P1）判「**SLO 出口表里每个用户可见指标都指得到一列
  // 读数**」：`04 §1.3` 那张逐阶出口表的 measurement 一栏此前全是「埋点 / 日志扫描 /
  // 投影比对 / 单测」——没有一行说得出用户在界面上读哪里，而 `session/stats` 这个只读口
  // 早就存在。它治的是 P1 登记的原始问题：**出口判据停留在「测试有没有过」，不是
  // 「用户看见的数变了没有」**。
  //
  // 不判「产品判断对不对」（纪律 4：不可机械化），只判一条可核对的锚：表里指认的
  // 每一列，代码里真有一列。`—` = 这一项还没有用户可见读数——**不是豁免，是待建**，
  // 所以 `—` 的行数每次门禁都打进日志。
  //
  // 失效形态与 doc-count-audit 同族且更隐蔽：列名从 `stats.rs` 的 `Serialize` 结构体
  // 字段提，**不是手上的一份清单**。提取器漏提 ⇒ 表里的指认被判「不存在」而红（安全
  // 的一侧）；退化成硬编码 ⇒ 加列时表与守卫一起说谎而全绿。回归测试因此双向钉：
  // 改名后旧指认必须红（而新名绿）、读不出必须 exit 2、整表 `—` 不得假红。
  'slo-exit-audit',
  // `panic-audit`（PN-001…003）判「**每一处 panic 面都是一个显式事实**」：非测试 Rust 里的
  // `unwrap` / `expect` / `panic!` / `unreachable!` / `todo!` / `unimplemented!` 逐处按
  // `文件::函数` 归并，每一项必须在**同一个文件**里有一行 `// panic-allow …: 非空理由`。
  // 它治的是 J3 在这一层没有落点：「这一处是不变量」与「这一处是事故」在文本上完全一样，
  // 而 64 处此前一处也没说清。PN-003 反向判「登记了而代码没了」，防豁免只增不减。
  // ⚠️ 它的失效形态**只有一种且最坏**：提取器一坏就数出 0 个命中，然后报「全部已登记」
  // 退出 0。回归测试因此拿真实仓库的登记项 / 命中数做非零对照。
  'panic-audit',
  // `no-direct-call-audit`（S6 第 14 步，出口判据「直连调用 0」）判「**任意两主体
  // 无直连——一切经事实源**」（[roadmap/S08 §5](../../docs/009-task/128-多主体与对等承诺.md "T-128-多主体与对等承诺")）：
  // 直接调用的前提是持有对方句柄，故禁两类形态——NDC-001 主体类型名出现在
  // 定义域/根重导出/测试文件之外的运行时代码（注释不算，文档 ≠ 对象图边）；
  // NDC-002 任何文件里的主体句柄（`Arc<P>` / `Rc<P>` / `Arc::new(P)` /
  // `Box<dyn P>` / `&dyn P`——协作只走事件）。主体名单是**显式清单**（加主体
  // 登记一行），不存在注释豁免通道。回归测试双向钉住：运行时引用 / 互持句柄
  // 必须变红，定义域 / 重导出 / 测试驱动 / 注释提及不得误报。
  'no-direct-call-audit',
  // `list-write-fact-audit`（LWF-001–002，ADR-054 的**永久结构守卫**）判「**消息列表的
  // 变更只有唯一入口**」：切主读路径之后，重建只看事实，事实里没有的变更**等于没发生过**
  // （被截断的消息复活、编辑过的内容回退）。它治的病是**机制缺失**不是疏忽——10 个写入口里
  // 6 个没有事实，因为没有任何东西要求「改了列表就必须落事实」。类型强制（`MessageChange` +
  // `apply`）把「漏落事实」变成表达不出来的形态；守卫守的是类型管不住的那半——**有没有第二条路**：
  // `append_messages` / `message_events::{added,edited,removed}` 出 `chat_session/write.rs`
  // ⇒ LWF-001；`replace_all` 出 `write.rs` 且 ±20 行内没有 `consolidated = Some(...)` 的
  // **真实赋值**（寄存锚，剥注释后匹配——注释里提一句不算寄存）⇒ LWF-002。
  // 判定是**结构性**的：v1 的「就地登记 + `pending` 棘轮」（及 LWF-003/004 两码）随 b3 前置
  // pending 归零一并退役，守卫转正为永久结构守卫（ADR-054 实施补记 3/4）。失效形态是
  // **判据自身的钝化**（窗口算错 / 正则过宽 / 测试文件漏排除），由回归测试逐分支钉住。
  'list-write-fact-audit',
  // `gate-codes-audit` 判**判据码本身**：撞号（两个脚本声明同一码）、未登记前缀、
  // 输出里出现没登记的码、空命名空间、同一前缀两种说法。它守的是判据的**地址**——
  // 豁免注释、门禁日志、文档指认都按码定位，码一旦撞车，「已豁免」会在两个脚本里
  // 同时命中而人只看见其中一个。失效形态与 `35-baseline` 同族：判据被削不需要动
  // 任何测试，且不留痕迹，所以判据必须是**独立一跑**而不靠人记得住号段。
  'gate-codes-audit',
  // `gate-wiring-audit` 判**门禁自己有没有在跑这些测试**：名单是手写的，而「加了一份
  // 测试但没接线」这件事没有任何地方会报——实测 `windows-restart` 两份测试躺在仓里
  // 从没被跑过，其中一份早就红了（`prepare` 用 realpath 建 stage，测试拿未归一化的
  // 临时目录名比前缀，盘符大小写不同必然假红）。它反向也判：名单指名的文件不存在 ⇒
  // 那一步跑的是空气。判据是「名单 vs 目录事实」两个方向的差集，不是词表。
  'gate-wiring-audit',
  // `registry-audit`（RG-001…013）判**登记条目本身**——「单文档 + 元信息 + 自动汇总」
  // 这套顶层机制（plan/14）的全部强制力：front-matter 可解析、字段按 kind 收口、
  // 编号唯一且文件名即编号、状态机与状态×字段矩阵（done 必有 evidence、wont 必有
  // reason）、依赖不悬空 / 不成环 / 开工时前置已闭环、正文小节齐、SP-001 字段表与
  // 代码 schema 恰好相等（RG-011）、出口判据指认的脚本必须存在（RG-012）。
  // 它治的病是 13 §0 纪律 2 / 4 在**状态登记处自身**上的缺口：手工表格的状态流转
  // 此前没有任何机械核对。失效形态只有一种——判定写宽 ⇒ 违规条目照样过且日志一片绿，
  // 故回归测试逐条注入钉「必须红、且只红这一条」，外加本仓 dogfood 断言防「读空即全绿」。
  'registry-audit',
  // `plugin-special-case-audit`（SC-001…SC-005）判**插件层的特判回流**：core 侧有六个
  // 可执行审计，插件层此前零个——后果不是理论风险，同一轮复盘里三条回流各自照出过一次
  // （工具执行流程按工具名字面量判定、三协议各写各的 `max_tokens` 下限、冻结字段有定义
  // 无读方）。本守卫收的是能机械化的两条：工具名不许进比较位置（SC-001）、供应商差异
  // 必须落档案表（SC-002 预算数值 / SC-003 思考字段名 / SC-004 供应商 `if` 分支）。
  //
  // 它**不用数字基线**：基线是源码常量，改小它不动测试、不留痕迹（`T-032` 的弱点）。
  // 改为逐处豁免（`// special-case-allow SC-00X: 理由`，空理由不算），并由 SC-005 反向
  // 判悬空——于是「多一处」= 「多写一条理由」，而豁免本身也进了棘轮。
  //
  // 失效形态三种，回归测试各钉一头：① 工具名集合读不出来 ⇒ 一条都不命中却照样绿
  // （故空集合直接 exit 1）；② 比较位置的识别写宽 ⇒ 误报一片、豁免喂到失效；
  // ③ 注释没剥 ⇒ 注释里提一句 `agent_run` 就红。近邻形态（`unwrap_or("ask_user")`、
  // `.get("heartbeat")`、`match` 别名臂、`anthropic-version` 头名）都逐条断言不误报。
  'plugin-special-case-audit',
]
// 不是**判定型**审计脚本，只跑回归测试（共享库 / 门禁原语 / 报告型脚本）：
//   - `color` 带一道「scripts/ 下不得手写 ANSI」守卫；
//   - `gate.d/_shared` 的 `autoWork` 是「自动执行的工作」原语。它的失效方式
//     与守卫同源且更隐蔽：**看起来在修、其实没把修复带进提交**——本地跑一次门禁
//     完全看不出来（文件确实被格式化了），只在「修复前就已脏/已暂存」时暴露。
//   - `cli-binary` 是「二进制必须对应当前源码」的原语。它的失效方式是**跑起来了
//     但跑的是过期产物**：用例照常执行、断言照常失败，只是失败形态与眼前的源码
//     矛盾（源码里明明有的字段，运行时是 undefined），把排查方向引到源码上。
//     回归测试钉住「指纹必须随构建输入变」——判据退化成「文件存在就算新鲜」时会红。
//   - `tauri-binary` 是同一个原语的**壳侧**那一份，失效方式更贵：它不回显错误、
//     只是让**日志**看起来来自当前源码。于是「日志里有一条源码中不存在的行」会被
//     当成「代码没接上」去读一遍代码，而真相是跑着过期产物。回归测试钉住三件事：
//     指纹随输入变（含 `symbio/src`——整棵插件树被编译进壳）、
//     「产物比构建戳旧」必须判不可信（构建失败时旧产物会看起来新鲜）、
//     输入未变时不得重写戳（否则「什么都不用重建」会变成假警报）。
//   - `schema-audit` 是**报告型**（见下 `REPORT_ONLY`），失效形态不是假绿灯而是
//     **说假话**：把在用的模块列成「下放候选」，读的人顺着去改本来没坏的东西。
//     实测事故：`schemas/hook` 被报成「仅 1 个外部消费文件」，而它实际有 3 个消费文件
//     ——hook 插件走 `schemas::{HookEvent}` 顶层再导出名，路径里没有子模块名。
//     报告不判失败 ⇒ 坏了没人发现，故它比判定型守卫**更需要**回归测试。
//   - `core-surface-audit` 同样是报告型，同样说假话的形态：**少算消费方** ⇒ 在用的
//     符号被列成「下放候选」。开发时真踩了两次，两条都写成了回归测试：① 三条
//     `pub use <域>::*` 撞进同一个 Map 键 ⇒ 公开面从 251 掉到 134，`PLUGIN_*` /
//     `PathKey` / `PluginStopReason` 全部消失；② `symbio/src` 直属文件（`lib.rs` /
//     `plugins/mod.rs`）被跳过 ⇒ 只在注册表里被用到的符号被算成 **0 个消费方**。
//     「数不到」与「真的没人用」是两件事。
//   - `rust-scan` 是**共享扫描库**，被 `plugin-entry-audit` 与 `facts/current`
//     共同引用。它的失效形态是本仓最危险的一类：**写错不报错，只静默漏报**——
//     朴素实现在字符串里的 `//`、`format!("{{}}")` 的字面量括号、未跨行的引号上
//     都会翻车，而后果是「生成器说某条路由不存在，其实是抽漏了」。回归测试对每种
//     翻车输入各钉一条。
//   - `commit` 是提交入口（不是门禁的一环）。它的失效形态是**卡住自动化**：
//     默认读 stdin 等交互 ⇒ 无 TTY 时挂起（实测后台提交卡在
//     `Detected unsettled top-level await`），而它偏偏是「跑完门禁后那一句」。
//     另两种：repoRoot 不认调用方 cwd ⇒ 在别的仓库跑却读本仓索引；
//     `--gate` 被预览分支吞掉 ⇒ 人以为门禁跑过了。回归测试用真实临时 git 仓库
//     钉住这三条（`stdio: ['ignore', ...]` 让「默认读 stdin」暴露成超时）。
//   - `windows-restart` / `windows-restart-prepare` 是「换壳重启」监督器的真实进程测试
//     （只在 win32 跑）：它们此前**从没被门禁跑过**，其中 prepare 一份早已红了
//     ——正是 `gate-wiring-audit` 存在的理由。
export const TEST_ONLY = [
  'color',
  'rust-scan',
  'gate.d/_shared',
  // `gate.d/_logs`：门禁日志落点（每轮一个目录）与失败详情（末 N 行印在失败处）。
  // 失效形态**全是静默的**：写错一级目录 / 指针没写 / `failureDetail` 少读一段
  // （`note`、`logFile` 数组形态），门禁照样全绿，只是下一轮读者拿不到数据、
  // 失败处少一块信息。所以逐条注入坏形态（同名占用、指针悬空、聚合任务的多个
  // 子日志、日志已被删）断言预期表现。
  'gate.d/_logs',
  'cli-binary',
  'tauri-binary',
  'schema-audit',
  'core-surface-audit',
  'commit',
  // `registry-lists`（RG-026 / RG-027 / RG-029，清单归属）的失效形态**全是恒真**：
  // 三条判据分别判「同一 id 两个 owner」「声明与正文标记两面对账」「role 是否在闭集内」，
  // 任何一条写宽或写漏都表现为「永远通过」——而「永远通过的分工守卫」比没有守卫更坏，
  // 它让人以为分工已被守住。
  // 首次接入时 RG-027 当场漏过一次（owner 是免登记的生成物 CURRENT.md，对判据隐形）；
  // 已退役的 RG-028 也当场恒真过一次（needle 用了去扩展名的 `CURRENT`，正文里满地都是）——
  // 两条教训同源：**判据的输入会悄悄消失，而恒真不会报错**。
  // 所以这个文件只做一件事：注入坏形态，断言每一种都判红。
  'registry-lists',
  // `gate`（`gate.test.mjs`）的失效模式是**代价**不是报错：`--help` 不被识别时，
  // 脚本会照常跑完全量门禁（10+ 分钟）并覆盖 `.workbuddy-ai/gate-logs/*.log`——
  // 查一次用法就丢掉上一次的失败日志。判据 = 退出 0 + 打印用法 + **没进门禁主流程**。
  // 与 `commit` 同类（那里 `--help` 会真的提交一次），两处同批补上。
  'gate',
  // `md-table` 是全仓唯一的 Markdown 管道表解析库（`core-naming-audit` 与
  // `gen-verify-facts` 都吃它）：它的判据是「读不出来必须返回 null」，
  // 而返回 null 与返回空表在调用方那边是红与绿的区别。
  'md-table',
  // `gen-verify-facts` 是 verify 三程序的**输入**来源。它自己的 18 条测试里
  // 14 条是反向用例（文档与声明各说各话必须抛）——不跑它，「外置」就只是把
  // 手填数据从程序里搬进脚本，绿灯照样可能是假的。
  'gen-verify-facts',
  // `prompt-shape-audit`（PS-001…003）是 prompt 形状基线的**提取与比对**实现，
  // 登记成 TEST_ONLY 而不是 GUARDS：它的输入是**运行时抓的真实请求体**，静态跑不出来
  // ——门内的执行方是 `e2e/cases/t46-prompt-shape-baseline.mjs`（58-e2e 阶段自动纳入）。
  // 这里跑的是它的回归测试，防的是本仓最坏的那一类失效形态：**提取写宽 ⇒ 结构载体
  // 没进形状 ⇒ 拍平红不了**，以及反向的**提取写严 ⇒ 规模成了判据 ⇒ 改一句用例措辞就红**。
  // 两头各钉一条，见 `prompt-shape-audit.test.mjs`。
  'prompt-shape-audit',
  // `gen-doc-refs`（RF-001 / RF-002）是「文档引用只写编号、地址由机制生成」的执行方：
  // `[[编号]]` 展开成链接，已展开的地址按编号**重算**（文档搬家后入链自愈），
  // 解析不出的编号判红。它是**生成器**（门内执行方是 `60-facts` 的 autoWork），
  // 这里跑的是它的回归测试——防的失效形态与前几个生成器同源：**提取/重写字宽 ⇒
  // 该改的不改（入链静默失效）或不该改的乱改（守卫开始改坏正文）**。
  'gen-doc-refs',
  // `facts/routes-ts` + `route-facts` 是前端路由契约的**唯一真源链**（后端 `route()` 臂 →
  // `constants/routes.gen.ts`）。它的失效形态是静默少一条：提取口径一旦把某个形态
  // （复合臂 / 根容器 home）读漏，前端就拿到一份「后端没有的地址」，而 M-008 的词表
  // 恰恰来自这份生成物——生成物漏了，守卫也跟着漏。反向用例（不是臂的字符串不算路由、
  // 手改生成物必红）因此必须由门禁跑。
  'facts/routes-ts',
  // `gen-facts` + `facts/_lib` 是「事实生成」这条机制的执行方：目录遍历、描述符契约
  // 校验、幂等落盘与溯源行剔除。跑它的**回归测试**是因为这类 runner 的坏死方式全是
  // 静默的——目录读不出 / 描述符破损 / `out` 越界 / render 掺时间戳，四种都会让
  // 「什么都没生成」被当成「生成成功」。只测好输入的话，一个把所有事都吞掉的 runner
  // 也能全绿。
  'facts',
  // 两份 Windows 换壳重启监督器的真实进程测试（`skip` 在非 win32）。它们一直没接线，
  // 于是 prepare 那份红了很久没人知道——接线由 GW-001 保证不会再丢。
  'windows-restart',
  'windows-restart-prepare',
]
/**
 * 被 `import` 或被 `spawn` 调用、但**没有自己的门禁步骤**的 `.mjs`。
 *
 * 这张名单与上面两张合起来把 `scripts/` 的**命名空间闭合**：每一个非测试 `.mjs`
 * 必须落在三张名单之一（`gate-wiring-audit` 的 GW-004 判）。此前没有这张，
 * 于是「既没有回归测试、又不在任何名单里」的 `.mjs` 是完全隐形的——写一份
 * `foo-audit.mjs` 忘了登记，它就静默地不存在。实测的现成例子是 `route-facts.mjs`：
 * 它被 M10 立为「什么算一条路由臂」的唯一语法实现、被三个判定共读，却既没有同名
 * `.test.mjs`，也不在任何名单上（只被 `facts/routes-ts.test.mjs` 与
 * `plugin-entry-audit.test.mjs` 的夹具**间接**碰到）。
 *
 * ⚠️ 它与前两张的区别是**它自己不会被当成一道门禁步骤跑**：前两张每一条都跑
 * 回归测试（或守卫本体），这一张只被引用。所以放进来的门槛是「它必须真的被谁
 * 引用」——一份没人 import 也没人 spawn 的 `.mjs` 放进来就是自我豁免，
 * `gate-wiring-audit` 因此同时判「名单里的名字磁盘上必须存在」（GW-002 扩面）。
 *
 * 逐条理由：
 *  - `color`：ANSI 输出的唯一实现（`scripts/` 下不得手写转义序列）；
 *  - `rust-scan`：Rust 文本扫描库，被 `core-export-audit` / `facts/current` /
 *    `plugin-entry-audit` / `route-facts` 共读；**写错不报错、只静默漏报**，
 *    后果是「生成器说某条路由不存在，其实是抽漏了」；
 *  - `md-table`：全仓唯一的 Markdown 管道表解析库（`core-naming-audit` 与
 *    `gen-verify-facts` 都吃它），判据是「读不出来必须返回 null」；
 *  - `gate.d/_shared`：门禁原语（`BASELINE` / `ratchetVerdict` / `autoWork` / `color`）；
 *  - `core-surface`：`core-surface-audit` 的公开面统计库（它只被那一个审计引用）；
 *  - `route-facts`：`docs/006-ref/006-当前事实表.md` §1 路由列、`routes.gen.ts`、`plugin-entry-audit`
 *    三份判定的共读提取实现（M4/M10 立）；
 *  - `gen-facts`：**事实生成机制的执行方**——遍历 `scripts/facts/` 逐个生成，
 *    由本阶段的 `60-facts` spawn；目录即接线，新增一份事实不需要改门禁；
 *  - `facts/_lib`：上述契约校验 + 幂等落盘 + 溯源行剔除的共享实现；
 *  - `facts/_capability`：`docs/006-ref/006-当前事实表.md` §6 能力矩阵的提取实现（工具描述 /
 *    风险 / 保留策略、VDFS 操作定义、路由职责）。它的失效形态与 `rust-scan` 同族——
 *    **提取写宽或写窄都不报错，只静默少一条**，而「少了哪一条」正是这张表要回答的事；
 *  - `facts/current`：`docs/006-ref/006-当前事实表.md` 的生成器，由 `gen-facts` 调用；
 *  - `facts/gate-codes`：`reference/GATE_CODES.md` 的生成器，由 `gen-facts` 调用；
 *  - `facts/lists-index`：`ref/007-清单归属总账.md` 的生成器，**渲染时发现分工重叠
 *    直接抛错**（一份会说谎的总账比没有总账更坏——它是给人查的，人会信它）；
 *  - `facts/routes-ts`：`tauri/src/constants/routes.gen.ts` 的生成器，由 `gen-facts` 调用；
 *  - `line-count`：门禁报告里的行数统计；
 *  - `doc-find`：「某条约定写在哪」的检索入口（`docs/README.md` 导航里对用户承诺的命令）。
 *  - `registry-lib`：登记机制（plan/14）的共享实现——front-matter 解析、schema 校验
 *    （RG-001…013 的判据与消息字面量都在它身上，判据码声明在它文件头）、汇总渲染；
 *    被 `registry-audit` 与 `registry-board` 共读，判据只写一份。
 *
 * 刻意**不在**这里：`check-commit-msg` 与 `cargo-offline-refresh` 由 git hook /
 * 人工触发，不是门禁的一环；`gate.mjs` 自己即门禁入口。
 */
export const LIBS = [
  'color',
  'rust-scan',
  'md-table',
  'gate.d/_shared',
  // `gate.d/_logs`：门禁日志的**落点**（每轮一个目录 + 保留最近 N 轮）与**失败详情**
  // 的呈现（末 N 行 + 路径）。被 `gate.mjs` 与两个 e2e 读者共用，本体不是独立
  // 门禁步骤。它治的两个病都关于「看不到」：上一轮的失败证据被下一轮冲掉；
  // 失败只给一行文件名、正文要自己去文件里翻（于是人再跑一遍 7 分钟的门禁）。
  'gate.d/_logs',
  'core-surface',
  'route-facts',
  'gen-facts',
  'facts/_lib',
  'facts/_capability',
  // `_sets` 是「封闭集判据」的**成员集合 + owner 声明**（读 front-matter 的
  // `lists: [<id>=owner]`，与 RE-007 同源）。登记成 LIBS：判据在它唯一的消费方
  // `facts.test.mjs` 的跨文档判据里，本体不是独立门禁步骤——跑两遍就是同一件事。
  // ⚠️ 它**不许**进 GUARDS：判据的住所是被判东西的对面，合并会让「改判据」与
  // 「改被判物」在同一次提交里互相掩护。
  'facts/_sets',
  // `_ref-link` 是生成物里的编号引用构造器（`createRefLink`）：facts 各模块靠它产出
  // 与 gen-doc-refs 逐字同形态的链接。登记成 LIBS：判据在它两个消费方的落盘产物里
  // （`facts.test.mjs` 扫「生成物里不得留下字面 refLink(」），本体不是独立门禁步骤。
  'facts/_ref-link',
  'facts/current',
  'facts/gate-codes',
  'facts/lists-index',
  'facts/routes-ts',
  'gen-doc-refs',
  'line-count',
  'doc-find',
  'registry-lib',
  // `e2e-concurrency` 的门内版本在 `58-e2e.mjs` 里逐轮打印，本脚本供人工复核 + `--ci`。
  //
  // 登记成 LIBS 而不是 GUARDS：它**不是一个独立的门禁步骤**，而是 e2e 阶段的
  // 判据在门外的可执行形态——跑两遍就是同一件事。GW-004 要的只是「它有落点」。
  'e2e-concurrency',
]
/** 不进门禁的辅助脚本：登记它们是为了让 GW-004 的「谁引用它」这条判据有落点。 */
export const OUT_OF_GATE = [
  'check-commit-msg',
  'cargo-offline-refresh',
  'gate',
  // `registry-board`：登记看板的**查询台**（stdout 打印，支持 --kind / --status / --id）。
  // 登记成 OUT_OF_GATE 而不是 GUARDS：校验判据全在 `registry-audit`（它进门禁），
  // 本脚本是同一条校验的只读出口——先校验后打印，有违规拒绝出结论，本身无新判据。
  'registry-board',
]
// 报告型：只防崩溃（退出码恒 0，判定需人工复核），走日志不刷屏。
// 刻意 `echo: 'none'`：这份报告的候选会长期存在（大部分是签名组成部分与自引用），
// 每次门禁都刷一遍只会训练人忽略它。要看结论就单独跑一次脚本。
const REPORT_ONLY = ['schema-audit', 'core-surface-audit']

/**
 * 已知的**长杆**：标 `long: true` 让并发池先起它们（LPT，见 `gate.mjs::runConcurrent`）。
 *
 * 并发池按声明序发任务，而本阶段 61 项里有一项的时长超过其余 60 项之和：这两份
 * 「换壳重启」测试用**真实进程**，一轮要起近百次 `powershell.exe`（冷启动
 * 2.2–2.6s/次，与 cargo 争核时更慢）。实测 `windows-restart` 单独 87s–229s，
 * 排在声明序第 38 位时，docs 阶段墙钟 = 等它被轮到的 ~50s + 它自己 —— 那 50s 是纯加项。
 *
 * ⚠️ 这是**时长提示**不是判据：写错了最多白排一次序（结果按声明序落，判定不变）。
 * 加新的慢测试时顺手加进来；别指望框架自己知道谁慢。
 */
const LONG_TESTS = new Set(['windows-restart'])

export default {
  id: 'docs',
  title: '静态审计',
  // 回归测试彼此独立（各自 spawn 一个 `node --test`），并发跑省掉 30+ 次串行启动。
  //
  // 守卫本体（下面的 `--strict`）**也进同一批**（2026-10-18）：它们同样彼此独立
  // （只读扫仓，各自一个进程）。此前为了保住每条的**结论行**（「N 条规则全部通过」）
  // 而把它们排除在并发之外，代价是整段串行——实测 20 个守卫 ≈ 32s 墙钟，而
  // `windows-restart` 一个测试就要 200s+，于是 docs 阶段 = 200s（批）+ 32s（串行尾）。
  // 现在结论行由框架的 `summary: true` 在批结束后补印（读各自日志的最后一行），
  // **信息一条不少，串行尾消失**。判据 = `gate.test.mjs` 里「批内任务保留结论行」。
  //
  // 阶段级并发：本阶段**只读**（扫 `.rs` / docs / scripts，不写任何文件），与
  // backend（只写 `target/`）、baseline（只读 git）、msrv（独立 target 目录）互不
  // 干扰；fmt 已在 05 屏障阶段跑完，看到的源码是定格的。⚠️ verify 不能进这批：
  // 它的 gen-verify-facts 会重写 `verify/facts/mod.rs`，而本阶段的 doc-count-audit
  // 读同一个文件——并发就是「审计读到半截生成物」（见 55-verify.mjs）。
  //
  // ⚠️ 这份"只读"曾经不成立：`gate.test.mjs` 的两条失败详情判据会往 `scripts/gate.d/`
  // 写一个演示阶段再删掉，于是 `gate-wiring-audit`（扫那个目录做 GW-005）在并发下
  // 随机把演示阶段报成"排在 60-facts 之后的未登记阶段"。当时的处置是把它挪到批后
  // 串行——**那是绕开症状**：真正的病是测试改门禁自己的活配置。现在演示阶段落在
  // 临时目录、由 `GATE_EXTRA_STAGES` 注入（见 `gate.mjs` 阶段发现处），本阶段才真的
  // 只读，全部任务得以在一批里跑完，没有串行尾。
  parallel: true,
  concurrency: 8,
  *tasks() {
    for (const name of [...GUARDS, ...TEST_ONLY]) {
      yield {
        label: `${name} 回归测试`,
        cmd: process.execPath,
        args: ['--test', path.join(scriptDir, '..', `${name}.test.mjs`)],
        cwd: repoRoot,
        parallel: true,
        long: LONG_TESTS.has(name), // 长杆先起，见 `LONG_TESTS`
      }
    }
    for (const name of GUARDS) {
      // ⚠️ `--strict` **不是可选的**：`GUARDS` 里凡是带 WARNING 级判定的
      // （`grep-audit` 的「疑似吞错需人工 review」、`plugin-entry-audit` /
      // `mechanism-audit` / `test-layout-audit` / `style-audit` 的 WARNING），
      // 不传它就**永远只打印、永不红**——只剩 ERROR 会拦，而 WARNING 恰恰是
      // 「需要人看一眼」的那一类，于是它天天被打印、天天没人看。
      //
      // 这不是假设：`doc-link-audit` 为同一个原因改过一次，它的文件头至今写着
      // 「2026-09-20 前失效链接只在 `--strict` 下失败，而门禁从不带该参数 ⇒ **从未
      // 真的红过**」。同一个坑不踩第二次。
      //
      // 实测当前每个守卫在 `--strict` 下都是 exit 0，故本条是**接线**而非清账；此后
      // 出现 WARNING 就必须**修掉或写下豁免理由**（`grep-audit` 有
      // `grep-audit-allow S-xxx: 理由` 这类留痕口）——强制那次
      // 人工 review 真的发生，而不是靠打印一行指望有人注意到。
      yield {
        label: `scripts/${name}.mjs`,
        cmd: process.execPath,
        args: [path.join(scriptDir, '..', `${name}.mjs`), '--strict'],
        cwd: repoRoot,
        echo: 'all',
        parallel: true,
        // 批内静音，但**结论行**要留（见本阶段头注释与 `gate.mjs::lastLogLine`）。
        summary: true,
      }
    }
    for (const name of REPORT_ONLY) {
      yield {
        label: `scripts/${name}.mjs（报告型）`,
        cmd: process.execPath,
        args: [path.join(scriptDir, '..', `${name}.mjs`)],
        cwd: repoRoot,
        echo: 'none',
        parallel: true,
      }
    }
  },
}
