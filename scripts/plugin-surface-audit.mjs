#!/usr/bin/env node
/**
 * plugin-surface-audit — **扩展层插件的源码只认宿主面**（HS-001 / HS-002，D-253）
 *
 * ## 它治的病
 *
 * `symbio_kernel` / 宿主机制层 / `symbio_plugin_api` 三个 crate 拆完之后，每个插件
 * 仍在 manifest 里列三到四行内部依赖、源码里混写三种前缀。「插件依赖几个架构模块」
 * 于是由**某段代码当初顺手写了哪个前缀**决定，而不是由边界决定——cargo 依赖图表达不了
 * 任何意图，评审也看不出某次改动是不是在扩大插件的可见面。
 *
 * 裁决见 DE-023 D-253：扩展层插件的唯一宿主依赖是 `symbio-plugin-api`，门面里是一份
 * **白名单**（每条都能报出消费方）。**manifest 依赖边只由 `plugin-entry-audit` 的
 * E-009 裁定**（白名单 = `plugin-api` / `kernel` / `providers` + 第三方）；本脚本只剩
 * 源码那一半，避免同一条依赖行有两个 owner。
 *
 * | 编号 | 判什么 | 红了怎么办 |
 * |---|---|---|
 * | HS-001 | 非控制面插件的源码（注释外）出现 `symbio_sdk::` / `symbio_deploy::` / `symbio_kernel::` | 从门面取那个符号；门面里没有 ⇒ 说明它还不该给插件看，先论证再加白名单 |
 * | HS-002 | 门面白名单条数超过基线（只许降） | 加一条要配一条决策理由，并同步上调基线 |
 * | HS-003 | 门面白名单里有符号在**扩展层插件代码里零引用** | 删掉它——白名单的每一条都要报得出消费方（与 C-003 同一把尺子）；报不出的就是空转条目 |
 * | HS-004 | 插件胶水层 `symbio-providers` 把**嵌入臂**设成默认开启（`default = ["embedding"]` 或依赖不是 `optional`） | 那一条 `default = []` 是有意的：`ort` / `tokenizers` 带来 55 个传递依赖（168 → 113），而插件侧只有 `local` 一个消费者。要开就在宿主与需要它的插件那里显式开（D-254） |
 * | HS-005 | 门面 manifest **或任一活跃 crate 的源码 / manifest** 里出现旧 core 名（两种拼写） | 旧 core 已整棵并入 `symbio-sdk`（D-279）；门面只依赖 kernel + sdk + deploy，活跃区不再把已删名写成引用。要一条控制面符号，先论证它属于扩展面还是该换成宿主调用口（D-276） |
 * | HS-006 | 语法臂条数 > 基线，或模块级转发条数 > 基线 | 两格都「只许不涨」：语法臂不计债但计件；模块转发一行换 N 条，要显式（D-278） |
 *
 * `symbio_kernel::` 前缀对扩展层插件同样不放行（D-418）：DE-002 说扩展层插件的**唯一**
 * 宿主依赖是 `symbio-plugin-api`，而语法臂本来就把扩展层要用的共享类型转出去了。放行
 * 源码前缀等于绕开门面的计件与消费方核对——插件随手写一条 `symbio_kernel::…` 即可，
 * 既不进门面棘轮、也不需要论证。控制面（`loop` / `session`）仍豁免这一档。
 *
 * ## 为什么按插件身份分档，而不是一刀切
 *
 * `loop` / `session` 是**控制面**（DE-026 的影响类）：它们按轮的顺序运行、认识幕与
 * 容器概念。**源码前缀**这一档对它们豁免（D-253 候选 b 已否：压进门面只会逼出假豁免）；
 * 但 **manifest 依赖边不豁免**——控制面插件同样是插件，`symbio-sdk` / `symbio-deploy`
 * 一样由 E-009 判红。
 *
 * ## 失效形态
 *
 * ① **剥注释写漏** ⇒ 文档里那句「见 `symbio_sdk::vdfs::address`」被判红，天天误报的
 * 守卫最后只会被豁免喂死（复用 `extension-vocab-audit` 的 `stripRustCode`，只认代码）；
 * ② **控制面名单写成通配**（`symbio-plugin-*`）⇒ 判据变成空转，谁都能进来；
 * ③ **白名单基线写成大数** ⇒ 只进不出，机制退化成装饰。三者各由回归测试钉住。
 *
 * 用法：node scripts/plugin-surface-audit.mjs [--root=<目录>]
 * 退出码：0 = 通过；1 = 有违规；2 = 扫描范围读不出。
 */

// 判据码命名空间（登记表 docs/004-ref/GATE_CODES.md 由这些行生成，判据见 gate-codes-audit.mjs）
// @ns HS 扩展层插件宿主面
// @codes HS-001 HS-002 HS-003 HS-004 HS-005 HS-006

import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { red, green, yellow } from './color.mjs'
import { stripRustCode } from './extension-vocab-audit.mjs'

/**
 * 控制面：按 DE-026 的影响类运行，认识宿主的容器概念。**逐个名字写死**，不写通配——
 * 一份通配的白名单就是零张门。
 *
 * 注意：这里的豁免只作用于**源码前缀**（HS-001）。插件的 **manifest 依赖边**归
 * `plugin-entry-audit` 的 E-009 统一裁定，控制面插件不豁免。
 */
export const CONTROL_PLANE = ['symbio-plugin-api', 'symbio-plugin-loop', 'symbio-plugin-session']

/**
 * 源码里不得出现的三个前缀：`symbio_sdk::` / `symbio_deploy::`（应用侧门面）与
 * `symbio_kernel::`（共享语法）。
 *
 * kernel 也在其中（D-418）：扩展层插件的唯一宿主依赖是 `symbio-plugin-api`，语法臂
 * 已经把扩展层要用的共享类型转了出去。直接写 `symbio_kernel::` 会绕开门面的计件与
 * 消费方核对——那正是本判据要守的通道。控制面（`loop` / `session`）按身份豁免。
 *
 * manifest 依赖边不在这里判——它只有 E-009 一个 owner。
 */
export const FORBIDDEN_PREFIXES = ['symbio_sdk::', 'symbio_deploy::', 'symbio_kernel::']

/**
 * 机制臂条数基线：**只降不升**（HS-002）。实测低于它时打提示要求同步下调。
 *
 * 只数机制臂。门面按来源分三臂（D-255 / D-258）：语法臂来自 `symbio-kernel`（直接依赖
 * 6 个，插件取共享类型不增加任何机制面）、SDK 臂来自 `symbio-sdk`（机制实现，但不认
 * 容器概念，同样不计债）、机制臂曾来自旧 core（每条都要控制面支撑）。
 * 把前两臂计进基线，会让「欠账还剩多少」被两个与债无关的数字稀释——248 听起来离 0
 * 很远，133 才是真实的待还量。
 *
 * 115 → 72（D-258 第一批：`text` / `clock` / `logger` / `event_bus` / `creator` /
 * `vdfs` / `keys` 七个域搬进 `symbio-sdk`）→ 25（第二批：`chat` / `capability` /
 * `adapters` / `llm` / `plugin` 五个域；另把 `chat` 的三个子命名空间与内核语法
 * `ring` 各归其臂）→ 19（第三批：`agent` 与 `embedding` 两个域）→ 6（第四批：`turn` /
 * `projection` / `invariants` 三个域，D-259）。这些条**不是还清了存量**，是搬了家：
 * 它们一条不少地留在门面上，只是改从 `symbio_sdk` / `symbio_kernel` 取（HS-003 照旧
 * 逐条核消费方）。
 *
 * 剩的 6 条曾在 `authz`（4）与 `actors`（2），理由是「它们认识此刻这个主体能做什么」。
 * **D-276 复核后改判 0**：`principal_of` / `production_matrix` 只查一张部署表，`CAP_*` 是
 * 那张表的闭集，`actor_can_assign_work` / `commitment_events` 只读 `ActorSpec` 与事件流——
 * 没有一条认识轮次、幕或判决。它们随 `governance` / `authz` / `view` / `actors` 的
 * spec+commitment 一起进 `symbio-sdk`，门面从此不依赖旧 core（HS-005）。
 */
export const WHITELIST_BASELINE = 0

/**
 * SDK 臂（机制面）的基线：**这一臂同样进棘轮**（D-269 第 3 条）。
 *
 * 此前这一臂「不计债」——只由 HS-003 要求「每条有扩展层消费方」。代价是它成了
 * 门面增长的**零代价通道**：`D-261` 加两条（校准视图 / 字符截断）、`D-268` 改判前
 * 还要再加一条（保真度内核），没有任何门禁会红。机制面是「能力能调用宿主什么」的
 * 完整清单，与 `core` 臂是同一件事的两种住址，因此必须同一把尺子。
 *
 * 准入（D-269 §5.1 的两条判据）：与容器概念无关 ∧ ≥2 条能力要用。一条能力撑不起
 * 一次架构改动——要加一条就把理由写成决策，并同步上调本基线。
 *
 * 105：实测 105，低于此前基线 107（两格漏跟降）；按「账本只许降、基线跟着实测降」
 * 收平。这是下调，不是放宽。
 *
 * 106（D-386）：升格闸搬进 conation 后，`PRINCIPAL_AUTONOMOUS` 成为扩展层第二处消费的
 * 授权常量（债源与执行体同一把尺）。它满足 §5.1 两条：不提能力名（是一条主体身份），
 * 且 ≥2 条能力要用（conation 的债与执行体）。上调一格，记在 D-386 的「判据」节。
 */
// 106 → 120（D-413）：门面收掉九份纯转发壳文件之后，原先藏在壳里的出口在根上如实
// 计件——7 个已搬走的模块名（`behaviors` / `context` / `effects` / `fact_source` /
// `retrieval` / `rows` / `settle`）、改名转出的 `text`、5 个提交宏与
// `publish_deployment_root`。**没有一个名字是新增的**：它们此前分别经壳里的
// `pub use …::*;` 与壳内宏转发给出，旧读数只是看不见（`readFacadeArms` 只读 lib.rs）。
// 同一批里扩展面对 `drive_entry` 的出口从 9 个符号 + 整模块收窄到 1 个符号。
// 这是「把读数改成真的」，不是放宽：物理门面行数同期从 2703 降到 2635（FA-002）。
// 122 → 124（D-416）：跨插件报表的登记面与读口整块搬进 sdk 之后，门面在根上转出
// 模块名 `reports` 与提交宏 `submit_reports`。两者此前分别经门面 `reports.rs` 与
// `#[macro_export]` 给出，**同两个名字换住所**。视图缺席原因 `ViewUnavailable` 只有
// 控制面 `session` 用，按 HS-003 不上门面（`session` 走 E-009）。
// 124 → 126（D-420）：`chat` 组合模块分拣——`chat_node` / `principal` 两个模块级条目
// 回到语法臂（它们本来就是内核共享语法），SDK 臂腾出的两格换成 sdk 自己的实现出口：
// `AssistReply` / `AssistRequest` / `SEG_MESSAGES` / `message_of_node`。净 +2 是把原先
// 藏在「一行放出整个命名空间」里的实现摊到账上（HS-006 的另一格，D-278），四条的实现
// 与消费方都没变——不是新增机制面。
// 126 → 127（D-427）：能力自持配置之后，插件侧需要一条通用的「本轮快照贡献」声明口。
// 它满足 §5.1 两条准入：不提任何能力名，且每一条带开关的能力都要用（≥2 条）。
// 宿主侧读口只有控制面 session 消费，按 HS-003 不上门面（与 `ViewUnavailable` 同例）。
// 127 → 128（D-428）：子作用域改名工具要向会话装配交出「(前缀名, 本名)」这一对，
// 声明面沿用能力表的开放族构造口，门面上只多转出族常量 `SCOPE_TWIN` 一个名字；
// 读口 `capability_scope_twins` 只有控制面用，按 HS-003 不上门面。两条准入同样成立：
// 常量不含任何能力名，声明方（agent 子作用域）与读方（session 装配）是两个消费方，
// 且前缀形状归声明方——控制面不再按名字猜副本。
// 119 → 143（`D-438`）：`llm` 与 `dialog` 两块词汇从内核迁进 `symbio-sdk`（内核里零读者，
// 凭据只有一条已不存在的 trait）。门面上**同一个名字换了个住所**——24 条出口从语法臂
// 搬到 SDK 臂，一条不是新增：`RoundOutput` / `RoundToolCallInfo` / `PromptMessage` /
// `PromptToolCall` / `ModelFinishReason` / `ModelUsage` / `ModelSpend` 与 `dialog` 的
// 判决 / 措辞契约，消费方一条没变（HS-003 照旧逐条核）。语法臂同批 −24，见
// `GRAMMAR_BASELINE` 的注。这是「把读数改成真的」，不是放宽。
// 143 → 138（`D-439`）：一次往返的**产物**五个名字（`RoundOutput` / `RoundToolCallInfo` /
// `ModelFinishReason` / `ModelUsage` / `ModelSpend`）退回 model 插件——`port.model` 之后
// 会话侧读的是事实键（`usage` / `spend` / `reason`），全仓没有第二个读者，它们不再是
// 跨插件词汇。留下的请求侧（`PromptMessage` / `PromptToolCall`）与 `dialog` 的三个消费方
// 没变，所以这是一个**净减**，不是搬臂。
export const SDK_BASELINE = 138

/**
 * 语法臂基线：**只许不涨**（D-278）。
 *
 * 这一臂长期「不计债」——理由是内核本来就有的共享类型不增加机制面。理由成立，但它顺带
 * 让 136 条（门面里最大的一臂）成了唯一没有任何数字约束的通道：加一条语法符号不需要过
 * 任何棘轮。不计**债**与不计**数**是两件事，后者改起来一分钱不花。
 */
// 136 → 137（`D-314`）：`response` 载荷的写方移入执行体后，`answer_id` 必须与读侧
// 共用内核那一份派生（`event_output_text_node_id`）；两处各算一遍会静默漂移。
// 消费方是 `symbio-plugin-model`，不是控制面缺口。
// 137 → 138（`D-319`）：model 执行体组装 assistant 应答要声明 `tool_calls` 的配对键，
// 需要 `PromptToolCall` 这一值类型；插件不得直依赖 `symbio_kernel`（HS-001），只能经
// 门面转出。消费方是 `symbio-plugin-model` 的执行体。
// 138 → 137（`D-339`）：`OWNER_PRINCIPAL` 在扩展层零消费（唯一引用方 session 属控制面），
// 按 HS-003 从门面删除；控制面改从 `symbio_kernel::event` 取，棘轮随实测下调。
// 137 → 139（`D-418`）：HS-001 对扩展层禁用 `symbio_kernel::` 之后，model 执行体原先
// 直取的三条语法（`event::branch::{removed_ids, unselected_roots}`、`ClosureReason`）
// 必须经门面转出——它们本来就是内核共享类型，只是此前有一条旁路没被计件。此前实测
// 136（基线 137 留了一格余量），加三条后按实测收平到 139。
// 139 → 141（`D-420`）：`chat_node` / `principal` 两个模块级条目从 SDK 臂换到语法臂。
// 它们本来就是 `symbio-kernel` 的共享语法，此前经 sdk 的整模块转出才被记在 SDK 臂；
// 同两个名字换主人，门面之外没有任何新增的可写路径。
// 141 → 116（`D-438`）：其中 24 条是 `llm` / `dialog` 迁进 `symbio-sdk` 之后换臂（名字、
// 消费方都没动，见 `SDK_BASELINE` 的注），余下 1 条是 `D-437` 撤掉的模型行读口。
// 语法臂「只许不涨」，下调是账本跟着实测走。
export const GRAMMAR_BASELINE = 116

/**
 * 模块级转发的基线：**只许不涨**（D-278）。
 *
 * 白名单里有一类条目指向的不是一条符号，而是一个**子模块**（`chat_node` / `ring` /
 * `principal` / `family`）：一行放出的是一整个命名空间，而 HS-003 只按名字核消费方
 * （`symbio_plugin_api::chat_node::…` 用到的那些名字根本不在白名单的任何一行里）。
 * 这不是要禁掉模块转发——同族符号整块给出是合理的——而是要它**显式**：一行换 N 条，
 * 得在账上有数，不能是排版事故。
 */
// 6 → 13（D-413）：同一次「壳文件收掉」把 7 个模块名与改名转出的 `text` 抬到根上。
// 它们此前各自在壳里以 `pub use …::*;` 放出一整个命名空间——那正是本判据要显式化的
// 形状（D-278），只是壳文件让读数漏掉了。一行换 N 条的账现在记在门面上，条目一个不少。
// D-415 复查时补上**单段模块转出**（`pub use symbio_sdk::events;`）这一形态：它同样
// 一行放出一整个命名空间，此前不在 `collect` 的匹配里。补完后实测仍是 13——`events`
// 按 CS-008 / CP-004 留在门面，没有换成 `pub use`。
// 13 → 14（D-416）：`reports` 模块随登记面整块搬进 sdk，一行放出报表的全部类型与读口；
// 这是 D-278 要显式化的形状。`views` 没有上门面（零扩展消费方，HS-003）。
// D-420 后 `chat_node` / `principal` 改由语法臂从内核转出：同两个名字换来源，条数不变。
export const MODULE_REEXPORT_BASELINE = 14

/** 门面白名单所在的文件与块的起止标记。 */
export const FACADE = 'crates/symbio-plugin-api/src/lib.rs'
/**
 * 插件胶水层 `symbio-providers` 的**重臂**：嵌入推理（`ort` + `tokenizers`）。
 * 它默认必须是关的（D-254）——插件要的是 `DefaultToolVisitor` / `MemoryFile` 这些胶水，
 * 不是一套 ONNX 运行时。判据只看 manifest：`optional = true` + `default` 不含它。
 */
export const HEAVY_ARM = {
  manifest: 'crates/symbio-providers/Cargo.toml',
  dep: 'symbio-provider-embedding',
  feature: 'embedding',
}

/** 返回 `symbio-providers` manifest 里重臂的违规项（文件不存在 ⇒ 无此风险，返回空）。 */
export function heavyArmViolations(toml) {
  const errors = []
  const depLine = toml.split(/\r?\n/).find((l) => new RegExp(`^${HEAVY_ARM.dep}\\s*=`).test(l.trim()))
  if (!depLine || !/optional\s*=\s*true/.test(depLine)) {
    errors.push(
      `${HEAVY_ARM.manifest}  里 \`${HEAVY_ARM.dep}\` 不是 \`optional = true\`——` +
        `嵌入臂必须可选（HS-004，D-254）：插件不该为一个胶水符号链接整套 ONNX 运行时`,
    )
  }
  const idx = toml.indexOf('[features]')
  const rest = idx >= 0 ? toml.slice(idx + '[features]'.length) : ''
  const next = rest.search(/^\[/m)
  const fb = next >= 0 ? rest.slice(0, next) : rest
  const dm = /^\s*default\s*=\s*\[([^\]]*)\]/m.exec(fb)
  if (dm && dm[1].split(',').some((s) => s.trim().replace(/["']/g, '') === HEAVY_ARM.feature)) {
    errors.push(
      `${HEAVY_ARM.manifest}  的 \`[features] default\` 含 \`${HEAVY_ARM.feature}\`——` +
        'HS-004 / D-254：默认关，宿主与需要它的插件各自显式开；默认开等于把这条裁决作废',
    )
  }
  return errors
}

/**
 * 门面里按来源 crate 分区的三条 `pub use` 语句形态。
 *
 * **这里原来是一条块锚点**（`BLOCK_START` 锚的是旧的 core 门面块）。D-279 删掉那个
 * crate 之后锚点再也打不中，`readWhitelist` 于是恒返回 `[]`——HS-002 读到「机制臂 0 条」、
 * 恒等于基线 0、恒绿。**判据与它守的对象一起消失，而它自己不红**（D-280 的第二次实例，
 * 而且是同一个人在同一件事上第二次踩）。
 *
 * 改法是换判据的形状：不问「那段块在不在」，问「门面把这些 `pub use` 都发给了谁」。
 * 分区是**穷尽**的——除 kernel / sdk / deploy 之外的任何来源都算机制臂，所以
 * 旧 core 若回来不需要任何锚点也当场红；同时要求**至少读到一条** `pub use …::`，
 * 读不到就退 null（判不了不许亮绿灯）。
 */
const USE_STMT_RE = /pub use\s+([A-Za-z_][A-Za-z0-9_]*)(?:::[A-Za-z0-9_]+)*::(\{[^}]*\}|[A-Za-z0-9_]+|\*)\s*;/g

/** 门面允许的三个来源：语法（kernel）/ 机制（sdk）/ 装配常量（deploy）。 */
export const FACADE_ARMS = ['symbio_kernel', 'symbio_sdk', 'symbio_deploy']

/**
 * 已退役的旧 core crate 名——HS-005 要禁的就是它（Cargo `[dependencies]` 侧的连字符拼写）。
 *
 * 拆开拼写：它已不在活跃区作引用，但这条判据必须能报出被禁的名字。
 */
export const RETIRED_CORE_CRATE = `symbio${'-'}core`
/** 同一旧名在 Rust 源码里的下划线拼写（`pub use` 语句按它判「三臂之外」）。 */
export const RETIRED_CORE_MODULE = `symbio${'_'}core`
/** 语法臂：可能有多块（按 kernel 子模块分组），因此按正则全量取。 */
const GRAMMAR_RE = /pub use symbio_kernel(?:::[A-Za-z0-9_:]+)?::\{([^}]*)\}/g
/**
 * 单名一行的写法（`pub use symbio_kernel::view::Budget;` / `pub use symbio_sdk::clock::clock_now_ms;`）。
 *
 * 花括号组与单名一行是同一张名单的两种排版，**判据必须两种都读到**：
 * 只认花括号组时，一条单名重导出让 HS-003 的「每条都要报得出消费方」整条绕过去——
 * 那正是最容易藏匿空转条目的形状（一行、无逗号、无人注意）。
 */
const GRAMMAR_SINGLE_RE = /pub use symbio_kernel(?:::[A-Za-z0-9_]+)*::([A-Za-z0-9_]+);/g
/**
 * SDK 臂：同样按模块分组、可能有多块（D-258）。它**不计债**——这些机制只依赖内核，
 * 插件取它们不增加「要认识整个宿主」的代价——但它是门面对插件的承诺，所以和语法臂
 * 一样受 HS-003 管（每一条都要报得出消费方）。
 */
const SDK_RE = /pub use symbio_sdk(?:::[A-Za-z0-9_:]+)?::\{([^}]*)\}/g
const SDK_SINGLE_RE = /pub use symbio_sdk(?:::[A-Za-z0-9_]+)*::([A-Za-z0-9_]+);/g
/**
 * 带改名的转发（`pub use symbio_sdk::text::tokenizer as text;`）。
 *
 * 三臂的另外两条正则都要求 `;` 紧跟在名字后，改名写法读不到——而插件看到的正是
 * **别名**。读不到它，这一行既不进棘轮、也逃过 HS-003（一条完全不可审的出口）。
 * 条目名取别名：消费方写的就是别名。
 */
const AS_ALIAS_RE =
  /pub use\s+(symbio_kernel|symbio_sdk|symbio_deploy)((?:::[A-Za-z0-9_]+)*)::([A-Za-z0-9_]+)\s+as\s+([A-Za-z0-9_]+)\s*;/g

/** 来源 crate 的源码根（判「这条转发放出的是不是一个模块」）。 */
const CRATE_SRC = {
  symbio_kernel: 'symbio-kernel',
  symbio_sdk: 'symbio-sdk',
}

/** `pub use <crate>::<segs>::<name>` 指向的是不是一个真实模块（按文件判，不按名字猜）。 */
function isModuleItem(root, crate, segs, name) {
  const dir = CRATE_SRC[crate]
  if (!dir) return false
  const base = path.join(root, 'crates', dir, 'src', ...segs)
  return fs.existsSync(path.join(base, name + '.rs')) || fs.existsSync(path.join(base, name, 'mod.rs'))
}

/**
 * 门面在根上转出的**模块名**（含改名后的别名）。
 *
 * 这些名字是「本 crate 内的转手」：`pub use rows::{RowHandler, …};` 的 `rows` 已由
 * 根上一行 `pub use symbio_sdk::plugin::rows;` 从机制臂转来，第二条只是把同一个
 * 来源的符号再摊到根上——它不是「三臂之外的来源」，不该按白名单条目计。判据按
 * **来源**分区，所以别名必须跟随它的来处（`D-413`）。
 */
export function facadeModuleAliases(root, src) {
  const out = new Set()
  for (const m of src.matchAll(/pub use\s+(symbio_kernel|symbio_sdk)::([A-Za-z0-9_:]+)::\{([^}]*)\};/g)) {
    const segs = m[2].split('::')
    for (const raw of m[3].replace(/\n/g, ' ').split(',')) {
      const parts = raw.trim().split(/\s+as\s+/)
      const name = (parts[0] ?? '').trim()
      if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) continue
      if (isModuleItem(root, m[1], segs, name)) out.add((parts[1] ?? name).trim())
    }
  }
  for (const m of src.matchAll(
    /pub use\s+(symbio_kernel|symbio_sdk)::([A-Za-z0-9_:]+)::([A-Za-z0-9_]+)(?:\s+as\s+([A-Za-z0-9_]+))?;/g,
  )) {
    if (isModuleItem(root, m[1], m[2].split('::'), m[3])) out.add(m[4] ?? m[3])
  }
  return out
}

/** 把 `a, b as c` 这类清单切成符号名（取 `as` 之后的别名）。 */
function splitSymbols(body) {
  return body
    .replace(/\n/g, ' ')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean)
    .map((s) => s.split(/\s+as\s+/).pop().trim())
}

/**
 * 门面按来源 crate 分区读数。
 *
 * 返回 `{ counts, other }`：`counts` 是每个来源 crate 的语句数（日志里显示，让「0 条机制臂」
 * 是一次**读数**而不是一次沉默），`other` 是除三臂之外那些语句放出的符号名。
 * 读不到任何 `pub use …::` ⇒ `null`（判据的输入不在，不许亮绿灯，D-280）。
 */
export function readFacadeArms(root) {
  const p = path.join(root, FACADE)
  if (!fs.existsSync(p)) return null
  const src = fs.readFileSync(p, 'utf8')
  // 门面自己的模块（`pub use context::{…}` 这类）不是「来源」，是同一 crate 内的转手。
  // 按**文件是否存在**判，而不是按名字清单：新加一个本模块的转发不必回改这里。
  const ownDir = path.join(root, 'crates', 'symbio-plugin-api', 'src')
  const own = new Set(
    fs.existsSync(ownDir)
      ? fs.readdirSync(ownDir).map((f) => f.replace(/\.rs$/, '')).concat(['crate', 'self', 'super'])
      : [],
  )
  // 根上从三臂转出的模块名也是「本 crate 内的转手」：`pub use rows::{…}` 的来源
  // 不是另一个 crate，而是同一份机制臂的再摊开（`D-413`）。
  for (const name of facadeModuleAliases(root, src)) own.add(name)
  const counts = {}
  const other = []
  let read = 0
  for (const m of src.matchAll(AS_ALIAS_RE)) {
    read += 1
    counts[m[1]] = (counts[m[1]] ?? 0) + 1
  }
  for (const m of src.matchAll(USE_STMT_RE)) {
    read += 1
    const crate = m[1]
    counts[crate] = (counts[crate] ?? 0) + 1
    if (FACADE_ARMS.includes(crate) || own.has(crate)) continue
    const body = m[2]
    if (body === '*') {
      other.push('*')
      continue
    }
    other.push(...splitSymbols(body.replace(/^\{|\}$/g, '')))
  }
  if (read === 0) return null
  return { counts, other, read }
}

/**
 * 读出门面白名单的符号清单：**除三臂之外的一切来源**。
 *
 * 空清单是一次**测量结果**（门面今天只从 kernel / sdk / deploy 取东西），它的证据是
 * `readFacadeArms` 确实读到了 `read > 0` 条语句——这正是它和「锚点打不中所以返回 []」
 * 的区别，也是 D-280 要求的那一步。
 */
export function readWhitelist(root) {
  const arms = readFacadeArms(root)
  return arms === null ? null : arms.other
}

/** 读出门面语法臂的符号清单（`symbio_kernel` 各分块的并集）。 */
export function readGrammar(root) {
  const p = path.join(root, FACADE)
  if (!fs.existsSync(p)) return null
  const src = fs.readFileSync(p, 'utf8')
  const out = []
  for (const m of src.matchAll(GRAMMAR_RE)) out.push(...splitSymbols(m[1]))
  for (const m of src.matchAll(GRAMMAR_SINGLE_RE)) out.push(m[1])
  for (const m of src.matchAll(AS_ALIAS_RE)) if (m[1] === 'symbio_kernel') out.push(m[4])
  return out.length ? out : null
}

/** 读出门面 SDK 臂的符号清单（`symbio_sdk` 各分块的并集）。 */
export function readSdkArm(root) {
  const p = path.join(root, FACADE)
  if (!fs.existsSync(p)) return null
  const src = fs.readFileSync(p, 'utf8')
  const out = []
  for (const m of src.matchAll(SDK_RE)) out.push(...splitSymbols(m[1]))
  for (const m of src.matchAll(SDK_SINGLE_RE)) out.push(m[1])
  for (const m of src.matchAll(AS_ALIAS_RE)) if (m[1] === 'symbio_sdk') out.push(m[4])
  return out.length ? out : null
}

/** manifest 的 `[dependencies]` 段里是否有某个键（HS-005 只问门面自己，不判插件）。 */
function dependenciesHas(toml, name) {
  let inDeps = false
  for (const line of toml.split(/\r?\n/)) {
    const t = line.trim()
    if (/^\[.*\]$/.test(t)) {
      inDeps = t === '[dependencies]'
      continue
    }
    if (!inDeps) continue
    if (new RegExp(`^${name.replace(/[-/\\^$*+?.()|[\]{}]/g, '\\$&')}\\s*=`).test(t)) return true
  }
  return false
}

/**
 * 扩展层插件真正引用到的门面符号（剥注释后）：`symbio_plugin_api::X` 与
 * `symbio_plugin_api::{A, B}` 两种形态。测试文件也算消费方——这里只判「这条出口有没有
 * 人用」，不判「只有测试在用」（后者归 C-003 的三档口径，不在这里复判）。
 */
export function collectConsumers(root, pluginNames) {
  const used = new Set()
  for (const name of pluginNames) {
    const srcDir = path.join(root, 'crates', name, 'src')
    if (!fs.existsSync(srcDir)) continue
    for (const f of walkRs(srcDir)) {
      const code = stripRustCode(fs.readFileSync(f, 'utf8'))
      for (const m of code.matchAll(/symbio_plugin_api::\{([^}]*)\}/g)) {
        for (const raw of m[1].split(',')) {
          const parts = raw.trim().split(/\s+as\s+/)
          // 别名只是本地名，**被消费的是 `as` 前面那个门面符号**。`X as _` 尤其如此：
          // 它把 trait / 常量带进作用域只为调方法（`LlmAdapter as _`），旧实现取别名
          // 得到 `_`、把真正的消费方数丢，白名单条目被误判成空转（HS-003 假红）。
          for (const s of [parts[0].trim(), parts.length > 1 ? parts[1].trim() : '']) {
            if (s !== '_' && /^[A-Za-z_][A-Za-z0-9_]*$/.test(s)) used.add(s)
          }
        }
      }
      for (const m of code.matchAll(/symbio_plugin_api::([A-Za-z_][A-Za-z0-9_]*)/g)) used.add(m[1])
    }
  }
  return used
}

/**
 * 门面里**指向子模块**的条目（D-278）。
 *
 * 判法是「看文件」而不是「看名字」：对 `pub use <crate>::<父路径>::<名字>;`，去
 * `<crate>/src/<父路径>/<名字>.rs` 或 `<父路径>/<名字>/mod.rs` 找那个模块。名字撞车是常态
 * （`projection::transcript` 是模块，`transcript::transcript` 是函数），按名字猜会把函数
 * 误判成模块，判据就变成噪音。
 */
export function collectModuleReexports(root) {
  const out = new Set()
  const crates = {
    symbio_kernel: path.join(root, 'crates', 'symbio-kernel', 'src'),
    symbio_sdk: path.join(root, 'crates', 'symbio-sdk', 'src'),
  }
  const src = fs.readFileSync(path.join(root, FACADE), 'utf8')
  const isMod = (base, segs, name) => {
    const dir = path.join(base, ...segs)
    return fs.existsSync(path.join(dir, name + '.rs')) || fs.existsSync(path.join(dir, name, 'mod.rs'))
  }
  for (const m of src.matchAll(/pub use (symbio_kernel|symbio_sdk)::([A-Za-z0-9_:]+)::\{([^}]*)\};/g)) {
    const base = crates[m[1]]
    const segs = m[2].split('::')
    for (const raw of splitSymbols(m[3])) if (isMod(base, segs, raw)) out.add(raw)
  }
  for (const m of src.matchAll(/pub use (symbio_kernel|symbio_sdk)::([A-Za-z0-9_:]+)::([A-Za-z0-9_]+);/g)) {
    const base = crates[m[1]]
    const segs = m[2].split('::')
    if (isMod(base, segs, m[3])) out.add(m[3])
  }
  // 单段转出（`pub use symbio_sdk::events;`）同样一行放出一整个命名空间：不给它留
  // 不计数的一格，否则「换个写法」就能绕过本判据（D-415）。
  for (const m of src.matchAll(/pub use (symbio_kernel|symbio_sdk)::([A-Za-z0-9_]+);/g)) {
    if (isMod(crates[m[1]], [], m[2])) out.add(m[2])
  }
  // 改名转出：放出的名字是别名，但对插件来说就是那一整个模块（`text` → `tokenizer`）。
  for (const m of src.matchAll(AS_ALIAS_RE)) {
    if (!(m[1] in crates)) continue
    const segs = m[2].split('::').filter(Boolean)
    if (isMod(crates[m[1]], segs, m[3])) out.add(m[4])
  }
  return out
}

function walkRs(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) walkRs(p, out)
    else if (e.name.endsWith('.rs')) out.push(p)
  }
  return out
}

/** 递归收集 `Cargo.toml` —— HS-005 的另一半扫描面（源码只盖住 `use`，依赖键在 manifest）。 */
function walkManifests(dir, out = []) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name)
    if (e.isDirectory()) walkManifests(p, out)
    else if (e.name === 'Cargo.toml') out.push(p)
  }
  return out
}

export function run(root) {
  const cratesDir = path.join(root, 'crates')
  if (!fs.existsSync(cratesDir)) {
    console.log(`${red('[ERROR]')} plugin-surface-audit：范围读不出——缺少 crates/`)
    return 2
  }

  const plugins = fs
    .readdirSync(cratesDir, { withFileTypes: true })
    .filter((e) => e.isDirectory() && e.name.startsWith('symbio-plugin-'))
    .map((e) => e.name)
    .sort()
  if (plugins.length === 0) {
    console.log(`${red('[ERROR]')} plugin-surface-audit：crates/ 下一个 symbio-plugin-* 都没有`)
    return 2
  }

  const errors = []
  let scannedFiles = 0
  let scannedPlugins = 0

  for (const name of plugins) {
    if (CONTROL_PLANE.includes(name)) continue
    scannedPlugins++
    const dir = path.join(cratesDir, name)

    const srcDir = path.join(dir, 'src')
    if (!fs.existsSync(srcDir)) continue
    for (const f of walkRs(srcDir)) {
      // 测试面不判：`*.test.rs` 是插件的**内部验证**，不是它对外承诺的依赖面。
      // 容器测试要扮演一次「轮次派发」才能验证路由转发，那批符号属于控制面的容器
      // 概念（`TurnSpec` / `TurnDispatch` / …），门面白名单里没有也不该有（D-253）。
      // 这与 EX-008 对 `*.test.rs` 不判是同一条纪律：测试可以把宿主词汇摆上桌。
      if (f.endsWith('.test.rs')) continue
      scannedFiles++
      const code = stripRustCode(fs.readFileSync(f, 'utf8'))
      const lines = code.split('\n')
      for (const [idx, line] of lines.entries()) {
        for (const pfx of FORBIDDEN_PREFIXES) {
          if (!line.includes(pfx)) continue
          errors.push(
            `${path.relative(root, f).split(path.sep).join('/')}:${idx + 1}  \`${pfx}\`——` +
              '扩展层插件源码只经 `symbio_plugin_api::…` 取宿主面（HS-001 / D-418）：' +
              '`symbio-sdk` / `symbio-deploy` 是应用侧门面，插件依赖它们等于「插件 → 应用」反向耦合；' +
              '`symbio_kernel` 的共享语法由门面的语法臂转出，绕开它等于绕开门面的计件与消费方核对',
          )
        }
      }
    }
  }

  const arms = readFacadeArms(root)
  if (arms === null) {
    console.log(
      `${red('[ERROR]')} plugin-surface-audit：读不出门面白名单（${FACADE}）——` +
        '一条 `pub use …::` 都没读到。门面文件在、但读法失效时，HS-002 会读到 0 条并恒绿（D-280）',
    )
    return 1
  }
  const wl = arms.other
  const gram = readGrammar(root) ?? []
  const sdk = readSdkArm(root) ?? []
  const n = wl.length
  if (n > WHITELIST_BASELINE) {
    errors.push(
      `门面机制臂 ${n} 条 > 基线 ${WHITELIST_BASELINE}——HS-002 只许降（D-253 / D-255）；` +
        '新增一条要把「谁消费、为什么非给插件不可」写成决策理由，并同步上调 `WHITELIST_BASELINE`',
    )
  }
  const ns = sdk.length
  if (ns > SDK_BASELINE) {
    errors.push(
      `门面机制臂（SDK）${ns} 条 > 基线 ${SDK_BASELINE}——HS-002 只许降，两臂同一把尺子（D-269）；` +
        '新增一条要过 §5.1 的两条准入（不提能力名 ∧ ≥2 条能力要用），写成决策并同步上调 `SDK_BASELINE`',
    )
  }

  // 语法臂与模块级转发：两条零代价通道，同一把尺子（D-278）。
  if (gram.length > GRAMMAR_BASELINE) {
    errors.push(
      `门面语法臂 ${gram.length} 条 > 基线 ${GRAMMAR_BASELINE}——HS-006 只许不涨（D-278）：` +
        '语法臂不计债，但计件；新增一条要写成决策并同步上调 `GRAMMAR_BASELINE`',
    )
  }
  const moduleEntries = [...collectModuleReexports(root)]
  if (moduleEntries.length > MODULE_REEXPORT_BASELINE) {
    errors.push(
      `门面模块级转发 ${moduleEntries.length} 条 > 基线 ${MODULE_REEXPORT_BASELINE}——HS-006 (D-278)：` +
        `\`${moduleEntries.join('\` / \`')}\` 各自放出一整个子模块，一行换 N 条；` +
        '要加就把「为什么整块给」写成决策并同步上调基线',
    )
  }

  const used = collectConsumers(root, plugins.filter((p) => !CONTROL_PLANE.includes(p)))
  // 两臂都判：语法臂不计债，但同样不许有空转条目——它也是门面对插件的承诺。
  for (const s of [...wl, ...gram, ...sdk]) {
    if (used.has(s)) continue
    errors.push(
      `门面白名单 \`${s}\` 在扩展层插件代码里零引用——HS-003：白名单每一条都要报得出消费方` +
        '（与 C-003 同一把尺子），报不出的就是空转条目，删掉它',
    )
  }

  // HS-005：旧 core 名不得回到活跃区（门面 manifest + 任一活跃 crate 的源码 / manifest）。
  //
  // 前半（D-276）：门面不得依赖旧 core。机制臂归零之后，这一条把「下次再为一条控制面
  // 符号整块依赖实现层」的路堵死——否则 21 个插件的编译图会重新背上控制面。
  // 后半（D-279）：旧 core 整棵并入 sdk 之后，任何 `use` / 依赖键写它都是编译不了的
  // 死引用。**这条判据不是「名字不存在所以永远绿」**：被禁名以常量拼出，但扫描面是
  // 真实源码；回归用例把两种拼写写进夹具，各自必须红。
  const retiredCoreHits = []
  const nameRe = (name) => new RegExp(`\\b${name.replace(/[-/\\^$*+?.()|[\]{}]/g, '\\$&')}\\b`)
  const allRs = walkRs(cratesDir)
  const allManifests = walkManifests(cratesDir)
  for (const f of allRs) {
    const code = stripRustCode(fs.readFileSync(f, 'utf8'))
    for (const name of [RETIRED_CORE_MODULE, RETIRED_CORE_CRATE]) {
      if (nameRe(name).test(code)) {
        retiredCoreHits.push(`${path.relative(root, f).split(path.sep).join('/')}（\`${name}\`）`)
      }
    }
  }
  for (const manifest of allManifests) {
    const lines = fs.readFileSync(manifest, 'utf8').split(/\r?\n/)
    for (const line of lines) {
      if (line.trim().startsWith('#')) continue
      for (const name of [RETIRED_CORE_MODULE, RETIRED_CORE_CRATE]) {
        if (nameRe(name).test(line)) {
          retiredCoreHits.push(
            `${path.relative(root, manifest).split(path.sep).join('/')}（\`${name}\`）`,
          )
        }
      }
    }
  }
  if (retiredCoreHits.length) {
    errors.push(
      `活跃区仍有已退役的旧 core 名：${[...new Set(retiredCoreHits)].join(' / ')}——HS-005 (D-276/D-279)：` +
        '旧 core 整棵并入 `symbio-sdk`；源码里的 `use` 改成 `symbio_sdk::…` / `symbio_kernel::…`，' +
        'manifest 里的依赖键整条删掉。同一条判据也挡门面重新依赖实现层',
    )
  }
  const facadeManifest = path.join(root, 'crates', 'symbio-plugin-api', 'Cargo.toml')
  if (fs.existsSync(facadeManifest)) {
    const toml = fs.readFileSync(facadeManifest, 'utf8')
    if (dependenciesHas(toml, RETIRED_CORE_CRATE)) {
      errors.push(
        `crates/symbio-plugin-api/Cargo.toml  [dependencies] 里有 \`${RETIRED_CORE_CRATE}\`——HS-005 (D-276)：` +
          '门面只依赖 kernel + sdk + deploy；要一条控制面符号，先论证它属于扩展面还是该换成宿主调用口',
      )
    }
  }

  // HS-004：插件胶水层的重臂默认必须是关的。文件不存在 ⇒ 这个仓没有那层胶水，无此风险。
  const heavyP = path.join(root, HEAVY_ARM.manifest)
  const heavy = fs.existsSync(heavyP) ? heavyArmViolations(fs.readFileSync(heavyP, 'utf8')) : []
  errors.push(...heavy)

  if (errors.length) {
    for (const e of errors) console.log(`${red('[ERROR]')} ${e}`)
    console.log(`\n扩展层宿主面：${errors.length} 处违规（HS-001 / HS-002 / HS-004 / HS-005）`)
    return 1
  }

  if (n < WHITELIST_BASELINE) {
    console.log(
      yellow(
        `△ HS-002 门面机制臂 ${n} 条 < 基线 ${WHITELIST_BASELINE}：请把 ` +
          '`WHITELIST_BASELINE` 同步下调（账本只许降，基线跟着降才不是空转）',
      ),
    )
  }
  if (ns < SDK_BASELINE) {
    console.log(
      yellow(
        `△ HS-002 门面机制臂（SDK）${ns} 条 < 基线 ${SDK_BASELINE}：请把 ` +
          '`SDK_BASELINE` 同步下调（账本只许降，基线跟着降才不是空转）',
      ),
    )
  }
  console.log(
    green(
      `✓ HS-001 扩展层零直接依赖：${scannedPlugins} 个非控制面插件的 ${scannedFiles} 个源文件` +
        `没有 \`symbio_sdk::\` / \`symbio_deploy::\` / \`symbio_kernel::\` 前缀（控制面 ${CONTROL_PLANE.length} 个源码前缀豁免：` +
        `${CONTROL_PLANE.join(' / ')}；manifest 依赖边归 E-009，控制面不豁免）· HS-002 机制臂 ${n} 条（基线 ${WHITELIST_BASELINE}，` +
        `读数来自 ${arms.read} 条 \`pub use\` 语句：` +
        `${FACADE_ARMS.map((c) => `${c} ${arms.counts[c] ?? 0}`).join(' · ')}）` +
        `+ SDK 臂 ${ns} 条（基线 ${SDK_BASELINE}，D-269 起同进棘轮）` +
        `· 语法臂 ${gram.length} 条（基线 ${GRAMMAR_BASELINE}，不计债但计件）` +
        `· 模块级转发 ${moduleEntries.length} 条（基线 ${MODULE_REEXPORT_BASELINE}）` +
        `· HS-003 每条都有扩展层消费方（0 空转）· HS-004 胶水层重臂默认关闭（${HEAVY_ARM.feature}）` +
        `· HS-005 活跃区无旧 core 名（扫 ${allRs.length} 个源码 / ${allManifests.length} 个 manifest）`,
    ),
  )
  return 0
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const arg = process.argv.find((a) => a.startsWith('--root='))
  const root = arg ? path.resolve(arg.slice('--root='.length)) : path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
  process.exit(run(root))
}
