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
  // `doc-symbol-audit`（D-005）判「**现行文档指认的 Rust 符号确实存在**」：
  // 反引号内 ≥2 段的 `module::symbol` / `Type::method` 形态，取末段查整词
  // 是否出现在 .rs 语料（359 文件 / 203 条指认）。这类失效读者看得见、守卫
  // 看不见：doc-link 只管链接、gen-current-facts 只生成结构表，都不校验散文
  // 里的符号指认——校准日一次就抓出五处改名残留（`KEY_PAYLOAD` →
  // `PLUGIN_PAYLOAD_KEY`、`HOOK_FIRE` → `ROUTE_HOOK_FIRE` 等，先修后建）。
  // 豁免出口只对应「文档说的是历史」：`DECISIONS.md`/`archive` 整体排除、
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
  // `panic-audit`（PN-001…003）判「**每一处 panic 面都是一个显式事实**」：非测试 Rust 里的
  // `unwrap` / `expect` / `panic!` / `unreachable!` / `todo!` / `unimplemented!` 逐处按
  // `文件::函数` 归并，每一项必须在**同一个文件**里有一行 `// panic-allow …: 非空理由`。
  // 它治的是 J3 在这一层没有落点：「这一处是不变量」与「这一处是事故」在文本上完全一样，
  // 而 64 处此前一处也没说清。PN-003 反向判「登记了而代码没了」，防豁免只增不减。
  // ⚠️ 它的失效形态**只有一种且最坏**：提取器一坏就数出 0 个命中，然后报「全部已登记」
  // 退出 0。回归测试因此拿真实仓库的登记项 / 命中数做非零对照。
  'panic-audit',
  // `no-direct-call-audit`（S6 第 14 步，出口判据「直连调用 0」）判「**任意两主体
  // 无直连——一切经事实源**」（[roadmap/S08 §5](docs/plan/roadmap/S08-多主体与对等承诺.md)）：
  // 直接调用的前提是持有对方句柄，故禁两类形态——NDC-001 主体类型名出现在
  // 定义域/根重导出/测试文件之外的运行时代码（注释不算，文档 ≠ 对象图边）；
  // NDC-002 任何文件里的主体句柄（`Arc<P>` / `Rc<P>` / `Arc::new(P)` /
  // `Box<dyn P>` / `&dyn P`——协作只走事件）。主体名单是**显式清单**（加主体
  // 登记一行），不存在注释豁免通道。回归测试双向钉住：运行时引用 / 互持句柄
  // 必须变红，定义域 / 重导出 / 测试驱动 / 注释提及不得误报。
  'no-direct-call-audit',  // `gate-codes-audit` 判**判据码本身**：撞号（两个脚本声明同一码）、未登记前缀、
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
//   - `rust-scan` 是**共享扫描库**，被 `plugin-entry-audit` 与 `gen-current-facts`
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
  'cli-binary',
  'tauri-binary',
  'schema-audit',
  'core-surface-audit',
  'commit',
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
  // `gen-routes-ts` + `route-facts` 是前端路由契约的**唯一真源链**（后端 `route()` 臂 →
  // `constants/routes.gen.ts`）。它的失效形态是静默少一条：提取口径一旦把某个形态
  // （复合臂 / 根容器 home）读漏，前端就拿到一份「后端没有的地址」，而 M-008 的词表
  // 恰恰来自这份生成物——生成物漏了，守卫也跟着漏。反向用例（不是臂的字符串不算路由、
  // 手改生成物必红）因此必须由门禁跑。
  'gen-routes-ts',
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
 * `.test.mjs`，也不在任何名单上（只被 `gen-routes-ts.test.mjs` 与
 * `plugin-entry-audit.test.mjs` 的夹具**间接**碰到）。
 *
 * ⚠️ 它与前两张的区别是**它自己不会被当成一道门禁步骤跑**：前两张每一条都跑
 * 回归测试（或守卫本体），这一张只被引用。所以放进来的门槛是「它必须真的被谁
 * 引用」——一份没人 import 也没人 spawn 的 `.mjs` 放进来就是自我豁免，
 * `gate-wiring-audit` 因此同时判「名单里的名字磁盘上必须存在」（GW-002 扩面）。
 *
 * 逐条理由：
 *  - `color`：ANSI 输出的唯一实现（`scripts/` 下不得手写转义序列）；
 *  - `rust-scan`：Rust 文本扫描库，被 `core-export-audit` / `gen-current-facts` /
 *    `plugin-entry-audit` / `route-facts` 共读；**写错不报错、只静默漏报**，
 *    后果是「生成器说某条路由不存在，其实是抽漏了」；
 *  - `md-table`：全仓唯一的 Markdown 管道表解析库（`core-naming-audit` 与
 *    `gen-verify-facts` 都吃它），判据是「读不出来必须返回 null」；
 *  - `gate.d/_shared`：门禁原语（`BASELINE` / `ratchetVerdict` / `autoWork` / `color`）；
 *  - `core-surface`：`core-surface-audit` 的公开面统计库（它只被那一个审计引用）；
 *  - `route-facts`：`docs/CURRENT.md` §1 路由列、`routes.gen.ts`、`plugin-entry-audit`
 *    三份判定的共读提取实现（M4/M10 立）；
 *  - `gen-current-facts`：`docs/CURRENT.md` 与本批 D0 计数守卫的共读生成器，
 *    由 `60-facts` 阶段 spawn；
 *  - `gen-gate-codes`：`reference/GATE_CODES.md` 的生成器，由本阶段 spawn；
 *  - `line-count`：门禁报告里的行数统计；
 *  - `doc-find`：「某条约定写在哪」的检索入口（`docs/README.md` 导航里对用户承诺的命令）。
 *
 * 刻意**不在**这里：`check-commit-msg` 与 `cargo-offline-refresh` 由 git hook /
 * 人工触发，不是门禁的一环；`gate.mjs` 自己即门禁入口。
 */
export const LIBS = [
  'color',
  'rust-scan',
  'md-table',
  'gate.d/_shared',
  'core-surface',
  'route-facts',
  'gen-current-facts',
  'gen-gate-codes',
  'line-count',
  'doc-find',
  // e2e 并发度的**离线判据**（`scripts/e2e-concurrency.mjs`）：门内版本在
  // `40-e2e.mjs` 里逐轮打印，本脚本供人工复核 + `--ci`。
  //
  // 登记成 LIBS 而不是 GUARDS：它**不是一个独立的门禁步骤**，而是 e2e 阶段的
  // 判据在门外的可执行形态——跑两遍就是同一件事。GW-004 要的只是「它有落点」。
  'e2e-concurrency',
]
/** 不进门禁的辅助脚本：登记它们是为了让 GW-004 的「谁引用它」这条判据有落点。 */
export const OUT_OF_GATE = ['check-commit-msg', 'cargo-offline-refresh', 'gate']
// 报告型：只防崩溃（退出码恒 0，判定需人工复核），走日志不刷屏。
// 刻意 `echo: 'none'`：这份报告的候选会长期存在（大部分是签名组成部分与自引用），
// 每次门禁都刷一遍只会训练人忽略它。要看结论就单独跑一次脚本。
const REPORT_ONLY = ['schema-audit', 'core-surface-audit']

export default {
  id: 'docs',
  title: '静态审计',
  // 回归测试彼此独立（各自 spawn 一个 `node --test`），并发跑省掉 30+ 次串行启动。
  //
  // 守卫本体（下面的 `--strict`）**刻意不并行**：它们的结论行（「N 条规则全部通过」）
  // 是终端上的主要信息，而并发批会静音逐行输出（见 gate.mjs 的并发契约）——
  // 用「少看 16 行结论」换几秒不划算。
  concurrency: 8,
  *tasks() {
    for (const name of [...GUARDS, ...TEST_ONLY]) {
      yield {
        label: `${name} 回归测试`,
        cmd: process.execPath,
        args: ['--test', path.join(scriptDir, '..', `${name}.test.mjs`)],
        cwd: repoRoot,
        parallel: true,
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
      }
    }
    for (const name of REPORT_ONLY) {
      yield {
        label: `scripts/${name}.mjs（报告型）`,
        cmd: process.execPath,
        args: [path.join(scriptDir, '..', `${name}.mjs`)],
        cwd: repoRoot,
        echo: 'none',
      }
    }
  },
}
