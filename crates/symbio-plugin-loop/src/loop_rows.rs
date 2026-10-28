//! 义务环的行表：一条绑定表上的全部行（策略 / 接缝 / 执行决策 / 端口与授权级联）。
//!
//! ## 一行只写「什么时候欠、什么算还」
//!
//! 判据全部是数据：`owes` / `until` / `unless` 三个模式，`prio` 是**全局**裁决序
//! （`(prio, name, key)` 升序）。执行体只拿到一笔 `Debt`，不读任何请求作用域状态——
//! 「什么时候欠」对应的是一份视图读出的事实，不是内存里的阶段枚举。
//!
//! ## 一表一行序（`D-222` 裁决 5）
//!
//! ```text
//! 0  authorize-tool─ 授权级联：一笔工具请求先判「放行 / 拒绝」
//! 10 classify       ── 轮首判决（本幕 round=0 且有发言）
//! 30 compress      ── 自动压缩（开关开着且本幕还没压过）
//! 40 recall        ── 记忆 / 技能 / 任务段（有持久事实源且本幕还没召回）
//! 50 reflex        ── 技能快路（非续写幕且本幕召回命中）
//! 52 prepare-turn  ── 接缝：轮首那几行跑完、任何请求之前的一次性装配
//! 53 report        ── 中途汇报（静默到期 ∧ 配额未满 ∧ 停在已关闭的批次边界）
//! 55 close-turn ── 收束：本幕最近一次模型应答没有工具调用
//! 60 execute       ── 执行决策：本幕未收束且手上没有开着的工具批 ⇒ 宣布一次模型往返
//! 66 dispatch-model─ 端口派发：模型往返（在轮内发起）
//! 67 open-tools ── 创建：模型响应的 `calls` 集合展开成 N 笔工具调用（D-117）
//! 68 dispatch-tools─ 端口派发：工具批次里已放行的单次调用（只交出去，出口 `sent`）
//! 69 deny-tools ── 拒绝收尾：被拒的那几笔按原因落 `failed`（b）
//! 71 await-model   ── 等应答（模型端口）
//! 72 await-tools   ── 等应答（工具端口）
//! ```
//!
//! `await-*` 与它的 `dispatch-*` 成对（`D-222` 订正 2 / `D-124` 订正 1）：派出与收应答
//! 分处两个环步，两个动作之间那一圈「已派出、未收清」就是 `waiting` 视图读的形状。
//!
//! 顺序进表而不是进调用点：同一点上多行竞争时，`drive` 取最小的一笔。`prio` 是跨行
//! 可比的**全局**序，不是格内序（判据 10）。轮首那几行、批次边界那一行与轮内各行
//! 在同一张表上竞争，同处一个 `loop`——`prio` 因此是唯一的先后（`D-222` 裁决 5）。
//!
//! ## 汇报的轮次前提也在表里
//!
//! 「跑过至少一轮工具」是 `report_due` v6 的 `due` 的一格（[D-163]）：本幕完成了几轮
//! 工具调用 = 本幕落了几条**已关闭**的带 `calls` 的 `response(port=model)`——日志上
//! 现成，不需要触发点在内存里数。于是这一行与其它行同形，派发点只选债。
//!
//! ## 查询口：事实是输入，`now` 也是输入
//!
//! [`owes`] / [`view`] 只收事实切片、轮号与 `now`——不读配置、不读时钟、不碰内存
//! 缓存（D-115 决策 1）。切片由 [`turn_slice`] 收窄到本轮：视图声明的扫描成本
//! 上界针对**一轮**，不是整份 WAL。
//!
//! ## 装载期检查
//!
//! 首次查询时经 [`load`] 把六张注册表经 `Builder::declare` 一次交齐并 `seal`，**任何缺口都是
//! 拒绝**（DE-015 §3.4 末句「装载失败即拒绝启动，不许部分成功」）：缺口逐条记 error 后
//! 装载返回 `None`，[`table`] 随之恒 `Err`。开幕（`chat_loop::drive::begin_drive`）先要
//! 表再干活，拿不到表就带着错误退出——不存在「半张表继续跑」的形态。

use std::sync::{Arc, LazyLock};

use serde_json::Value;

use symbio_core::ring::{owed, Decl, Gap, Lit, Pattern, PortDecl};
use symbio_core::{
    ActorDecl, Builder, Event, Frozen, LatencyTier, PluginDeclarations, EVENT_FAILED,
    EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY, EVENT_SENT,
};
use symbio_plugin_api::events::{
    EVENT_ARTIFACT_ADDED, EVENT_AUTHZ_DENIED, EVENT_CHAT_ASSISTANT_FALLBACK,
    EVENT_CHAT_ASSISTANT_FINAL, EVENT_CHAT_ASSISTANT_OPENING, EVENT_CHAT_ASSISTANT_PROGRESS,
    EVENT_CHAT_ASSISTANT_REPORTED, EVENT_MESSAGE_ADDED, EVENT_SESSION_CONSOLIDATED,
    EVENT_TURN_BRANCH, EVENT_TURN_DISPATCH, EVENT_TURN_SUPPLEMENTED,
};

use crate::control_decl::{EVENT_AUTHZ_GRANTED, EVENT_TURN_STAGE};

/// 这张表里**接了线的执行体**：`execute`（宣布一次模型往返）、`close`（收束）、
/// `opener` / `authorizer` / `courier`（工具批三笔债， 步 5b）、`relay` / `waiter`
/// （模型往返的派出与收回）、轮首那四条 `classify` / `compress` / `recall` /
/// `reflex`、接缝 `seam`，以及 `report`——**一张表就是全部**（`D-222` 裁决 5），
/// 它们借的 `Ctx` 面不同，名字面上没有任何区别。
///
/// `by` 只有两种取值（DE-015 §2.2）：`Some(名字)` ⇒ 引擎跑它，`None` ⇒ 引擎什么都不做。
/// 「还没接线」不是第三种取值，也不是求解器的第四个过滤条件（D-118 订正 1）——
/// 未接线的行显式写 `None`，进表、可 diff，接线时把 `Some(名字)` 与
/// [`ACTORS`] 里那条 `ActorEntry` 同批改回。
/// 检查方在装载期：`by = Some(a)` 而 `a` 没有声明 ⇒
/// `Gap::UnmannedRow` ⇒ 装载红。
///
/// 声明本身（DE-015 §2.3）与 `turn_exec::ACTORS` 的
/// 绑定项一起构造，这里只放静态可查的四样。
///
/// `writes` 是这个执行体**直接落格**的领域事实（账本语义：单写者是 store，不是引擎）。
/// 引擎独占的四格（`sent` / `response` / `failed` / `retry`）不在任何执行体的
/// `writes` 里；`request` 例外——它是**决策者**写的（§2.3），工具批的决策者就是
/// `opener`，所以它出现在 [`ACTOR_OPENER`] 的名单里（运行期账本仍把它当协议格豁免，
/// 两份名单在这一点上不冲突：豁免的是「不必声明」，声明的是「确由我写」）。
const EXECUTE: &str = "execute";
/// 收束执行体：把寄存的那一次应答落成收束格。
const CLOSE: &str = "close";
/// 创建执行体（b）：模型交回的调用逐条落 `request(port=tool)`。
const OPENER: &str = "opener";
/// 授权执行体（b）：对已开账的工具调用落放行 / 拒绝事实。
const AUTHORIZER: &str = "authorizer";
/// 派发执行体（b）：放行的落 `sent` 交给执行层（只交出去，不等结果——`D-124` 订正 1）。
const COURIER: &str = "courier";
/// 派发的收尾执行体（`D-124` 订正 1）：`await-tools` / `deny-tools` 两行选中的动作——
/// 收这一批的结果、关账，被拒的落 `failed`。
///
/// 与 [`COURIER`] 分开是「一个 `by` 一个动作」的同一条理由（D-118 决策 1–2）：
/// `courier` 把一批**交出去**（出口 `sent`），`waiter-tools` 把它们**收回来**
/// （出口 `response` / `failed` / `retry`）。共名就等于让一个动作按「现在是第几个
/// 环步」分支——那正是绑定表要消灭的东西。
///
/// 名字与 model 端口的 [`RELAY`] / [`WAITER`] 同族：**派出半用信使词**
/// （`relay` / `courier`），**收回半用等待词**（`waiter` / `waiter-tools`）。
const WAITER_TOOLS: &str = "waiter-tools";
/// 中转执行体（D-166）：`dispatch-model` 行选中的动作——跑一次**已宣布**的模型往返。
///
/// 与 [`COURIER`] 分开是 D-118 决策 1–2 的直接后果：一个 `by` 一个动作，而两件事
/// 不是同一件——`courier` 派发的是**一批**工具调用（批尾统一关账、落等待格与轮内
/// 注入、批次边界还借它的窗口落汇报格），这一条只是「把手上那一笔模型往返发出去」。
/// 共用一个名字，动作就只能按行名分支，那正是绑定表要消灭的东西。
const RELAY: &str = "relay";
/// 等待执行体（`D-222` 订正 2）：`await-model` 行选中的动作——取走寄存的那一次往返、
/// 等它落地、落 `response` / `failed`。
///
/// 与 [`RELAY`] 分开是同一个理由（一个 `by` 一个动作）：`relay` 把这一笔**发出去**
/// （出口 `sent`），`waiter` 把它**收回来**（出口 `response` / `failed`）。共名就等于
/// 让一个动作按「现在是第几个环步」分支——那正是绑定表要消灭的东西。
const WAITER: &str = "waiter";
/// 轮首准备执行体：`policy.recall` 行选中的动作。
///
/// 它借的是 [`super::turn_exec::Ctx::head`] 那一片（本轮状态 + 会话上下文 + 请求句柄）。
/// 「这行归谁跑」只写在行上，绑定表回答它跑哪个动作（D-118 决策 2）；同一张表上的
/// 轮内各行与它只差「`Ctx` 里哪一片有值」。
const RECALL: &str = "recall";
/// 汇报执行体：`policy.report` 行选中的动作。
///
/// 落格与上线在 `Ctx::head` 的 [`super::turn_exec::HeadCtx::report_landing`] 那一片；
/// 「该不该说」全由 `report_due` 判，派发点只选债——它因此与其它行同处一个环，
/// 不需要在批次边界的注入闭包里另开一个问点。
const REPORT: &str = "report";
/// 轮首压缩执行体（D-159）：`policy.compress` 行选中的动作。
const COMPRESS: &str = "compress";
/// 轮首判决执行体（D-159）：`policy.classify` 行选中的动作。
///
/// 名字与旧行 `effects.classify` 的那个能力名同源：这一行跑的是「问一次 `classify`
/// 并**执行**它判成的枚举」，判决本身住在能力插件里，执行体只做编排与入账。
const CLASSIFY: &str = "classify";
/// 快路执行体（D-163）：`policy.reflex` 行选中的动作。
///
/// 它只落「反射档已答」那一格，并把技能正文寄进本轮状态；收束格由
/// [`ACTOR_CLOSE`] 统一落——两个执行体都写 `final` 会让依赖图成环
/// （`reflex → close → execute → reflex`，DE-015 §3.4 第 1 条）。
const REFLEX: &str = "reflex";
/// 接缝执行体：`prepare-turn` 行选中的动作（`D-223` 订正 1）。
///
/// 它是「轮首那几行跑完、任何请求之前」的那一次装配：压缩入格、水位与根骨架、
/// 请求视图、定档、首响、基线 prompt。这些都不是一次模型往返，也不该由幕首的直线
/// 代码按行序表达——它们是一行债，出口是 `turn.stage{stage:"inputs"}` 那一格。
const SEAM: &str = "seam";

/// `execute` 执行体的声明（名单 = 运行期对账的**唯一依据**）。
///
/// 对外的用处不止 [`ACTORS`]：判据 2 / 判据 4 的**夹具绑定表**在同一个 `by` 上换动作时，
/// 照抄的就是这一份声明——名单一改，夹具与生产一起改，不会各拿一份名单。
pub(crate) const ACTOR_EXECUTE: ActorDecl = ActorDecl {
    name: EXECUTE,
    // 整轮执行体调模型：深度档。
    mode: LatencyTier::Deep,
    // 它不读视图：视图面由 `announce-model` 行的 `owes` / `until` 承担（行已查存在）。
    reads: &[],
    // **收束格不在这里**（c）：`chat.assistant.final` / `chat.assistant.fallback`
    // 归 `close` 执行体。两个执行体写同一批格 ⇒ 依赖图成环（`close → execute` 与
    // `execute → close`），装载红（DE-015 §3.4 第 1 条）。
    writes: &[
        EVENT_CHAT_ASSISTANT_OPENING,
        EVENT_TURN_STAGE,
        EVENT_TURN_DISPATCH,
        EVENT_ARTIFACT_ADDED,
        EVENT_SESSION_CONSOLIDATED,
        EVENT_AUTHZ_DENIED,
    ],
};

/// 创建执行体（b）：一个模型响应的 `calls` 逐条落 `request(port=tool)`。
///
/// `request` 是**决策者**的事实（§2.3），批次里做这个决定的就是它；`sent` 之后
/// 的协议格仍只由引擎写。装载期「出口全是引擎格时名单不许空」（D-124）在
/// `open-tools` 上不走那一档——它的出口里有可声明的 `request`，判交集。
const ACTOR_OPENER: ActorDecl = ActorDecl {
    name: OPENER,
    // 只写请求、不调模型、不跑工具：快速档。
    mode: LatencyTier::Fast,
    reads: &[],
    writes: &[EVENT_REQUEST],
};

/// 授权执行体（b）：风险档 vs 会话阈值的一次纯比较。
const ACTOR_AUTHORIZER: ActorDecl = ActorDecl {
    name: AUTHORIZER,
    mode: LatencyTier::Fast,
    reads: &[],
    writes: &[EVENT_AUTHZ_GRANTED, EVENT_AUTHZ_DENIED],
};

/// 派发执行体（b）：`dispatch-tools` 行选中的动作——已放行的落 `sent` 并交给执行层。
///
/// ## 它为什么一条领域格都不写（`D-124` 订正 1）
///
/// 派出去与收回来分处两个环步之后，这一半的事只有一件：把这一笔交出去。它落的
/// `sent` 是**引擎格**（端口契约的 `before`，`port::begin` 写），第 5 条禁止执行体
/// 把它写进名单；而批次真正落地的东西（产物 / 等待进展 / 轮内注入 / 批次边界汇报）
/// 全在**收尾半**，归 [`ACTOR_WAITER_TOOLS`]。
///
/// 装载期第 6 条的后半因此改判「有没有交出去的凭据」：出口里有某个端口的 `before`
/// 格即可（`check_ports` 已经要求那条端口有应答出口，关门链条完整）。这不是豁免——
/// 出口不是任何端口的 `before` 且名单为空的行照样红。
const ACTOR_COURIER: ActorDecl = ActorDecl {
    name: COURIER,
    // 派发本身不调模型（模型在执行层之外）。
    mode: LatencyTier::Fast,
    reads: &[],
    writes: &[],
};

/// 派发的**收尾半**（`D-124` 订正 1）：`await-tools` / `deny-tools` 两行选中的动作——
/// 收这一批的结果、关账、落产物与等待格。
///
/// 与 [`ACTOR_COURIER`] 分开是「一个 `by` 一个动作」的直接后果（D-118 决策 1–2）：
/// 派出（出口 `sent`）与收应答（出口 `response` / `failed` / `retry`）分处两个环步，
/// 两个动作之间那一圈「已派出、未收清」因此在事实面上可见。
///
/// 名单上三格都在**收尾**这一侧：`artifact.added`（2c 产物）、
/// `chat.assistant.progress`（2d 等待进展）、`turn.supplemented`（2f 轮内注入）——
/// 它们的写点在批尾。
///
/// 批次边界汇报（`chat.assistant.reported`）不在名单上：它的写点归 [`ACTOR_REPORT`]，
/// 因为那一行由同一个环按 `prio` 选中、在自己的动作窗口里落格——名单与写点因此一一对上。
///
/// `deny-tools` 为什么也归它：被拒的那几笔的终局（`failed{why}`）由批尾关账写出，
/// 而批尾就在这一半。派发半在那条路上什么都不落（`begin_calls` 只写放行的那些），
/// 把它挂在收尾半才与「谁写的」对得上。
const ACTOR_WAITER_TOOLS: ActorDecl = ActorDecl {
    name: WAITER_TOOLS,
    // 收一批工具结果：不调模型，快速档。
    mode: LatencyTier::Fast,
    reads: &[],
    writes: &[
        EVENT_ARTIFACT_ADDED,
        EVENT_CHAT_ASSISTANT_PROGRESS,
        EVENT_TURN_SUPPLEMENTED,
    ],
};

/// 中转执行体（D-166）：`dispatch-model` 行选中的动作——把**已宣布**的那一次模型
/// 往返发出去，并把它的结局归位。
///
/// ## 为什么它写得出兜底格
///
/// 名单里那两格只在**控制面失败**时落：模型往返派不出去（行不欠 / 端口没声明）或
/// 模型要了工具而本轮没有分发通道。两条路在日志上都没有模型的终局事实
/// （`failed(port=model)` 落不下来）⇒ 收束行开不出账 ⇒ 兜底格只能由**手上拿着这一笔
/// 往返的那一方**就地落。D-166 之前那一方是 `execute`（走它的过渡豁免，因为它的出口
/// 与 `close` 的出口同族 ⇒ 声明就是一条假环）；写方搬到这里之后，本行的出口是引擎
/// 独占的 `sent` / `failed`，与 `close` 的出口不同族，于是**声明得了**——过渡豁免
/// 随之清空（`ACTORS` 上不再有任何一条）。
///
/// ## 它不交回产出
///
/// 结局寄存在本轮草稿（`closing` / `pending_model`），由 `close-turn` / `open-tools`
/// 两行各自的执行体取走——执行体不判断「本轮结束了没有」（D-123 决策 5）。
const ACTOR_RELAY: ActorDecl = ActorDecl {
    name: RELAY,
    // 一次模型往返本身就是深度档的活。
    mode: LatencyTier::Deep,
    reads: &[],
    writes: &[EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_AUTHZ_DENIED],
};

/// 收束执行体（步 5c 起在**执行体之间**独占收束格）：它不调模型，只把寄存的那一次
/// 模型往返的**结局**落成收束格。
/// 「独占」说的是**执行体之间**：两个执行体写同一批格 ⇒ 依赖图成环（`close → execute`
/// 与 `execute → close`），表装不上（DE-015 §3.4 第 1 条）——所以收束格只能归一个执行体。
/// 它**不**覆盖轮首准备的 1b 分支（定稿答话轮）：那一轮一次模型往返都不发，收束格由
/// `turn_runner/prepare.rs` 就地落（缺口与归属见 ）。快路（技能命中）同样零往返，
/// 但它进驱动循环：`policy.reflex` 只落「已答」，收束格仍归这一格（D-163）。
///
/// 两条收束格都在它名下：`chat.assistant.final`（答完了 / 中止）与
/// `chat.assistant.fallback`（模型没答成 / 空答）。独占不是洁癖，是装载期的硬约束：
/// 两个执行体写同一批格 ⇒ 依赖图成环（`close → execute` 与 `execute → close`），
/// 表装不上（DE-015 §3.4 第 1 条）。`authz.denied` 是写侧闸拒那一条路留下的否定事实
/// （C4），它和收束格同一次动作里落，所以也在名单上。
/// 等待执行体的声明。
///
/// 行出口（`response` / `failed`）是**引擎写**的协议格——动作只把 future 取走并等它
/// 落地，两条事实由 `port::succeed_with` / `failed_with_extra` 写。名单里那一格是它
/// 唯一自己写的领域格：**模型要了工具而本轮没有分发通道**（配置缺口）时，日志上没有
/// 模型的终局事实（应答是好的，缺的是通道），收束行判不出「该收了」，兜底格只能由
/// 手上拿着这一笔往返的那一方就地落（D-166 的同一理由）。
const ACTOR_WAITER: ActorDecl = ActorDecl {
    name: WAITER,
    // 等的是模型往返的应答：与那次往返同档。
    mode: LatencyTier::Deep,
    reads: &[],
    writes: &[EVENT_CHAT_ASSISTANT_FALLBACK],
};

const ACTOR_CLOSE: ActorDecl = ActorDecl {
    name: CLOSE,
    // 收束就是「把已经拿到手的应答入格」：反射档，不调模型。
    mode: LatencyTier::Reflex,
    reads: &[],
    writes: &[
        EVENT_CHAT_ASSISTANT_FINAL,
        EVENT_CHAT_ASSISTANT_FALLBACK,
        EVENT_AUTHZ_DENIED,
    ],
};

/// 轮首准备执行体的声明（D-159）。
///
/// 名单只有 `turn.stage` 一格：召回这一步的**出口**就是那一格（`stage:"recall"`
/// 带 `result`），装载期第 6 条判的正是「执行体走不走得到自己那一行的收口」。
/// 它的三份产物（记忆视图 / 调度段 / 技能命中）寄存在本轮状态里交编排层取——
/// 执行体不交回产出（D-123 决策 5 在轮首照用，D-159 裁决 4）。
const ACTOR_RECALL: ActorDecl = ActorDecl {
    name: RECALL,
    // 事实投影的纯读 + 一次技能路由判定，不调模型：与行同档（更慢就装载红）。
    mode: LatencyTier::Reflex,
    reads: &[],
    writes: &[EVENT_TURN_STAGE],
};

/// 轮首压缩执行体的声明（D-159）。
///
/// 名单两格。`turn.stage` 是它的出口；摘要那一次模型往返的三格（`request` /
/// `sent` / `failed`，`port=model`）由**端口记账层**落，属协议格，不在执行体的名单里
/// （D-122 决策 2；运行期对账按 `PROTOCOL_CELLS` 豁免同一件事）。
///
/// `message.added` 是压缩**结果**入账：产线把压过的那份历史写回会话
/// （`REPLACE_ALL`），快照消息因此以 `message.added` 进日志。写它的决定是这一行动作
/// 做的（缩不压、压成什么），所以它是这一行的领域格，不是会话插件替它记的。
const ACTOR_COMPRESS: ActorDecl = ActorDecl {
    name: COMPRESS,
    // 语义压缩调模型：与行同档（深度）。
    mode: LatencyTier::Deep,
    reads: &[],
    writes: &[EVENT_TURN_STAGE, EVENT_MESSAGE_ADDED],
};

/// 轮首判决执行体的声明（D-159 / D-162）。
///
/// 名单同样只有 `turn.stage`：判决那一步的**出口**就是 `stage:"decide"` 那一格
/// （带 `result`，它是 `turn_head` 折出 `answered` 的唯一来源）。措辞那次内部往返
/// 的三格（`request` / `sent` / `response`，`via=compose/wording`）由端口记账层落，
/// 属协议格，不在执行体名单里（同 [`ACTOR_COMPRESS`]）。
const ACTOR_CLASSIFY: ActorDecl = ActorDecl {
    name: CLASSIFY,
    // 判决可能落到模型，但不生成：快速档（与行的 `tier` 同值，更慢就装载红）。
    mode: LatencyTier::Fast,
    reads: &[],
    writes: &[EVENT_TURN_STAGE],
};

/// 快路执行体的声明（D-163）。
///
/// 名单只有 `turn.stage` 一格：快路的**出口**就是 `stage:"reflex"` 那一格，
/// 装载期第 6 条判的正是「执行体走不走得到自己那一行的收口」。技能正文随本轮
/// 状态交给收束动作，**不进**事实网格（正文属于收束格的载荷，D-163）。
const ACTOR_REFLEX: ActorDecl = ActorDecl {
    name: REFLEX,
    // 技能命中即出话、零模型往返：反射档（与行同档，更慢就装载红）。
    mode: LatencyTier::Reflex,
    reads: &[],
    writes: &[EVENT_TURN_STAGE],
};

/// 汇报执行体的声明。
///
/// 两格：`chat.assistant.reported` 是这一行的**出口**（`reported_anchors` 视图按它逐段
/// 关账），`turn.stage` 是它自己落的**尝试格**（`stage:"report"`，`report_due` 的开账
/// 前件之一——「这一段静默试过没有」）。尝试格刻意不作出口：作出口就等于让本行的执行
/// 体同时是本行的关门方，依赖图成环。
///
/// 汇报的对话面节点随本轮的落库镜像一起走（`persist_messages`），**不进**事实网格。
const ACTOR_REPORT: ActorDecl = ActorDecl {
    name: REPORT,
    // 模板产线，零模型往返：与行同档（反射档，更慢就装载红）。
    mode: LatencyTier::Reflex,
    reads: &[],
    writes: &[EVENT_CHAT_ASSISTANT_REPORTED, EVENT_TURN_STAGE],
};

/// 接缝执行体的声明（`D-223` 订正 1）。
///
/// 名单是这一次装配落下的**全部**格子：`turn.stage`（出口那一格 `stage:"inputs"`，它
/// 同时翻转 `turn_head.prepared`）、`session.consolidated`（压缩入格）、
/// `turn.dispatch`（定档）、`chat.assistant.opening`（首响）、`turn.branch`（重试新支
/// 的登记）。它们不是四件事而是**一件事的四个落点**——「轮首那几行跑完之后、请求之前
/// 的这一次装配」，所以归一个执行体；拆成四行会让「谁先落」重新变成代码里的行序。
const ACTOR_SEAM: ActorDecl = ActorDecl {
    name: SEAM,
    // 装配不调模型（压缩与判决的模型往返在它们自己那一行已经跑完）：快速档。
    mode: LatencyTier::Fast,
    reads: &[],
    writes: &[
        EVENT_TURN_STAGE,
        EVENT_SESSION_CONSOLIDATED,
        EVENT_TURN_DISPATCH,
        EVENT_CHAT_ASSISTANT_OPENING,
        EVENT_TURN_BRANCH,
    ],
};

/// **执行体绑定表**（DE-015 §3.2 的 `actor` 口 + D-118 决策 2）：一条 = 一个 `by`。
///
/// 声明与动作在**同一条**里：装载期查得到的名字（[`actor_decls`] 是这张表的 `decl`
/// 投影，经 `Builder::actor` 入口冻结）与派发器跑得动的动作（[`next`] 查出的
/// [`Next::action`]）因此不可能一个有一个没有。接线一个执行体 = 在这里加一条 +
/// 把那一行的 `by` 改回 `Some(名字)`，同一个提交。
///
/// 动作住在 [`turn_exec`]（它要本轮草稿与七个端口），这张表只把它指到名字上。
///
/// `transitional` 是这条执行体**今天确实写得出、但 `decl.writes` 声明不了**的领域格。
/// 声明不了不是遗漏，是装载期会红：把收束格写进一个出口与 `close` 同族的执行体名单，
/// `close-turn` 的出口格就同时是两个执行体的出口格 ⇒ `Gap::DependencyCycle`（注①）。
/// 账本对账因此需要一个**有名有姓、会自己变红**的过渡豁免：它只在这条表项上生效，且由
/// `the_transitional_exemptions_are_pinned` 钉死内容——多一条就红，
/// 而不是将错就错地放宽名单。
///
/// **今天这张表上一条都没有**（D-166）：唯一那条（`execute` 写兜底格）随模型往返的
/// 派发搬到了 [`ACTOR_RELAY`]——它的出口是引擎独占的 `sent` / `failed`，与 `close`
/// 的出口不同族，那两格因此**声明得了**。这一格保留为空列表而不是删字段：它是装载期
/// 的判据面（「声明不了的一次写出」必须有名有姓地登记），删掉它等于下次有人需要时
/// 只能偷偷放宽 `decl.writes`。
///
/// 这张表是**派发器的入参**（[`super::turn_exec::drive`] 的 `actors`，D-123
/// 决策 4）：生产传 [`ACTORS`]，测试在同一张表的形状上注册别的东西。判据 14 那条
/// 「动作写了零格 ⇒ `exhausted{no_progress}`」的用例就靠这条接缝——没有它只能永远说
/// 那条分支「按构造不可达」。
pub(crate) struct ActorEntry {
    pub(crate) decl: ActorDecl,
    pub(crate) run: super::turn_exec::Action,
    pub(crate) transitional: &'static [&'static str],
}

pub(crate) static ACTORS: &[ActorEntry] = &[
    ActorEntry {
        decl: ACTOR_EXECUTE,
        run: super::turn_exec::run_execute,
        // 豁免已清空（D-166）：那两格兜底写随模型往返的派发搬到了 [`ACTOR_RELAY`]，
        // 而它的出口与 `close` 不同族 ⇒ 声明得了，不必再借豁免。
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_CLOSE,
        run: super::turn_exec::run_close,
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_OPENER,
        run: super::turn_exec::run_open,
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_AUTHORIZER,
        run: super::turn_exec::run_authorize,
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_COURIER,
        run: super::turn_exec::run_begin_tools,
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_WAITER_TOOLS,
        run: super::turn_exec::run_finish_tools,
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_WAITER,
        run: super::turn_exec::run_await,
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_RELAY,
        run: super::turn_exec::run_model,
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_CLASSIFY,
        run: super::turn_exec::run_classify,
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_COMPRESS,
        run: super::turn_exec::run_compress,
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_RECALL,
        run: super::turn_exec::run_recall,
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_REFLEX,
        run: super::turn_exec::run_reflex,
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_SEAM,
        run: super::turn_exec::run_seam,
        transitional: &[],
    },
    ActorEntry {
        decl: ACTOR_REPORT,
        run: super::turn_exec::run_report,
        transitional: &[],
    },
];

/// 一台派发点的**行子集**：`by` 落在绑定表里的那些行。
///
/// 为什么不向 `ring::drive` 传整张行表：未接线的行（`by: None`）不在任何派发点的子集
/// 里，它照旧由外层代码直问 [`owes`]（调度旁路，归零对象），而 [`owes`] 读的是**整张**
/// 冻结表，两者互不影响。
///
/// [`super::turn_exec::drive`] 因此拿的是整张 [`ACTORS`]：`prio` 是全局序，一个循环
/// 看得见全部行才是它的目的（`D-222` 裁决 5）。
fn bound_rows(frozen: &Frozen, names: &[&'static str]) -> Vec<Decl> {
    frozen
        .rows()
        .iter()
        .filter(|d| d.by.is_some_and(|by| names.contains(&by)))
        .cloned()
        .collect()
}

/// 绑定表的名字集合（[`bound_rows`] 的入参）。
fn actor_names(actors: &[ActorEntry]) -> Vec<&'static str> {
    actors.iter().map(|e| e.decl.name).collect()
}

/// 交给装载期冻结的执行体声明（`loop_rows::assemble` 的唯一用处）。
///
/// 一张绑定表一次交齐：装载面判的是「名字有没有人认领、认领的人走不走得到出口」，
/// 与执行体借走哪片可变状态无关。分开注册会冻出两份 `Frozen`，行表就有了两个真源。
pub(crate) fn actor_decls() -> Vec<ActorDecl> {
    ACTORS.iter().map(|e| e.decl).collect()
}

/// `by` → 绑定表里的那一条。`None` = 表里没有这个名字（装载期的 `UnmannedRow` 已拦，
/// 这里是派发面把「表被改坏」显形，不猜动作、也不猜它写了什么）。
///
/// 动作与声明**必须一次查全**：派发器读账本时要豁免的协议五格、以及
/// 「这个执行体声明过哪些领域格」，都来自同一条 `ActorEntry`。分两次查就是
/// 给「声明与动作漂移」留缝。
///
/// 查的是**传进来的那张表**（D-123 决策 4）：`ACTORS` 是生产表，接缝让夹具能在
/// 同名 `by` 上注册别的动作。
fn entry_of(by: &'static str, actors: &'static [ActorEntry]) -> Option<&'static ActorEntry> {
    actors.iter().find(|e| e.decl.name == by)
}

/// 协议五格：运行期写入封闭的**豁免名单**。
///
/// 它们由引擎写（`port.rs` 那一个文件，PTS-005 钉着），因此**不该**出现在任何
/// `ActorDecl::writes` 里——装载期 [`symbio_core::decl`] 的
/// `ENGINE_ONLY_CELLS` 检查拦的就是「执行体声明了引擎独占的格」。运行期读账本时
/// 它们当然在：派发的本体住在 `port` 里，执行体的动作调它（`relay` → `dispatch_full`，
/// `courier` / `turn_relay` → `dispatch`），四格因此落在动作窗口内——写点在执行体里，
/// 写方仍是引擎，两句话不冲突。
const PROTOCOL_CELLS: &[&str] = &[
    EVENT_REQUEST,
    EVENT_SENT,
    EVENT_RESPONSE,
    EVENT_FAILED,
    EVENT_RETRY,
];

/// [`Next`] 上的运行期对账口（DE-015 §3.4 第 5 条的**运行期那一半**，D-123 决策 2）。
///
/// 装载期只能查声明面（名字存在、不与引擎独占的格相撞），「执行体实际只写它声明过的
/// 领域格」只能拿**实际写入**对账——而实际写入的唯一可信取法是日志尾段（决策 1）。
impl Next {
    /// 账本上的这一格收不收得拢：每一格都要**在这张表里对得上一个名字**——协议五格
    /// （引擎写）、`decl.writes`（执行体声明）、`transitional`（过渡豁免）。
    /// 对不上就是越界，派发器不静默。
    pub(crate) fn writes_closed(&self, kind: &str) -> bool {
        PROTOCOL_CELLS.contains(&kind)
            || self.writes.contains(&kind)
            || self.tolerated.contains(&kind)
    }
}

/// 端口声明——**加一个端口只加这一条**，不加事实名、不改求解器。
///
/// 当前两个端口：`model`（一次模型往返， 步 2）、`tool`（批次里的单次工具调用，
/// 步 3）。
///
/// ## 这一张表凭什么叫「端口」（`D-222` §一）
///
/// 端口是**跨边界往返**：控制面在 `sent` 与 `response` 之间不持有对方的状态。这个
/// 定义是可判的，也因此砍掉了此前那条 `turn` 声明——它三条都不成立：应答载荷为空
/// （`extra = |_| Null`）、适配器就是内环自己（`run_turn → execute_turn →
/// drive`）、`awaited = false` 且 `max_attempts = 1`（没有任何等待方）。
/// 那是**重入**，不是往返；为它造的 `request` / `sent` / `response` 三格事实，
/// 三格都不带信息。删掉之后控制面只剩两个真端口。
///
/// ## 租约与尝试上限怎么定的
///
/// `lease_ms` 是「派出去多久没应答算卡住」。`max_attempts = 1` 是今天的实测值：
/// 租约到期只发生在进程重启之后（时间只在事件上推进），而等这笔应答的 future
/// 已经随进程一起没了。重试只会留下一条**永不派发**的 `request`，让「欠一次
/// 派发」变成一句谎。到顶即判死（`failed{why:"lease_expired"}`），恢复走 resume
/// 幕（新的逻辑 id，旧那笔作为「没回来」留在日志里）。
///
/// `retry` 因此只给**有活着的等待方**的尝试用（`D-138`）。上限是端口级的天花板，
/// 逐次尝试是否真能重试由 `sent.awaited` 决定（见 [`super::port::reap`]）。
///
/// ## `model` 端口
///
/// 一次**模型往返**。它在轮内发起（`D-115` 决策 5：两者之间没有双调度器）。
pub(crate) static PORTS: &[PortDecl] = &[
    PortDecl {
        port: "model",
        before: EVENT_SENT,
        lease_ms: 600_000,
        max_attempts: 1,
        // 一次模型往返是一条调用；同批并发在它下面的工具端口。
        concurrent: false,
    },
    // 工具批次里的**单次调用**。一次模型响应可带 N 个调用 ⇒
    // 同一批写 N 条 `request`，各自一个 id。
    //
    // ## 为什么上限是 2，而普通批次仍然不重试
    //
    // 上限是端口级的天花板，**不是**「每个调用都会重试」的承诺（D-138）。工具端口
    // 里有两类尝试：
    //
    // - 普通批次调用：结果等在当前草稿里，进程重启后等待方随草稿消失 ⇒
    //   `sent.awaited = false` ⇒ 到顶/到期即判死，恢复走 `resume` 幕（新 id）；
    // - 唤醒调用：`wake_batch` 能从事实重建那一笔（`resume` / `tool` / `args` 都在
    //   `request` 载荷里）⇒ `awaited = true` ⇒ 租约看护可以重试一次。
    //
    // 上限 2 = 首次 + 一次重试；再抬就需要新的等待方形态，而不是改数字。
    PortDecl {
        port: "tool",
        before: EVENT_SENT,
        lease_ms: 600_000,
        max_attempts: 2,
        // 一次模型响应可带 N 个工具调用：同批子债允许并发，
        // 具体哪几条同时在途仍由风险档决定（`dispatch/parallel.rs`）。
        concurrent: true,
    },
];

/// 内建行表。注册顺序**不参与**裁决——同 `prio` 由 `(name, key)` 决出。
pub(crate) static ROWS: &[Decl] = &[
    // ── 授权级联（⇒ 步 3）────────────────────────────────
    //
    // 两行，不是一行：
    //
    //   authorize-tool   owes: request(port=tool)      until: granted | denied
    //   dispatch-tools   owes: granted(port=tool)      until: sent | failed
    //
    // ## 为什么派发行 `owes` 写 `granted` 而不是 `request ∧ granted`
    //
    // `Decl.owes` 是**一个** `Pattern`（单源等值），跨源合取写不出来；要写就得
    // 改求解器，而求解器的字节是 PTS-003 的判据面（改它必须挂 ADR）。这里用
    // **蕴含**代替合取：`granted` 只可能由 `authorize-tool` 从一笔 `request`
    // 派生，所以 `granted ⇒ request` 在数据上成立；`dispatch-tools` 只欠
    // `granted`，与 `request ∧ granted` 的可达债集合相同。
    //
    // 这不是近似：`denied` ⇒ 没有 `granted` ⇒ 派发行**永不**成立（fail-closed），
    // 而「还没判」⇒ `authorize-tool` 仍开着、`dispatch-tools` 不欠——两种状态在
    // 匹配上可分，正是判据 8b 要的那条区分。全程没有否定原子。
    //
    // ## 为什么两行的 `key` 都是 `(port, id, attempt)`
    //
    // `authorize-tool` 的出口是 `authz.granted` / `authz.denied`，`dispatch-tools`
    // 的出口是 `sent` / `failed`：它们必须与协议事实关同一**次尝试**（D-138）。
    // 逻辑键会让上一 attempt 的出口把行提前关掉，重试出来的 `request{attempt+1}`
    // 就再也欠不动授权与派发。`authz.denied` 的**收束格拒绝**那条载荷只有 `key`
    // （目标格名），没有 `port` / `id` / `attempt`——按三元组绑定自然取不到实例，
    // 不会误关一笔工具授权。
    Decl {
        name: "authorize-tool",
        // 的口径：判在派发之前。0 是这一环里最早的一档。
        prio: 0,
        // 判定是纯比较（风险档 vs 会话阈值），不调模型（b 接线）。
        by: Some(AUTHORIZER),
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: EVENT_REQUEST,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("tool"))],
        },
        until: &[
            Pattern {
                source: EVENT_AUTHZ_GRANTED,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("tool"))],
            },
            Pattern {
                source: EVENT_AUTHZ_DENIED,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("tool"))],
            },
        ],
        unless: &[],
    },
    // ── 轮首判决 ─────────────────────────────────────────────────────────
    Decl {
        name: "policy.classify",
        prio: 10,
        // `prio` 10 让它排在压缩 / 召回之前——它落的 `result` 正是那两条的
        // 前件（[D-162] 裁决 1）。
        by: Some(CLASSIFY),
        // 判决可能落到模型，但不生成：快速档（与旧行 `effects.tier` 同值）。
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: "turn_head",
            key: &["turn", "round"],
            eq: &[
                ("round", Lit::Uint(0)),
                ("has_utterance", Lit::Bool(true)),
                ("classify_enabled", Lit::Bool(true)),
                ("classify_done", Lit::Bool(false)),
            ],
        },
        until: &[Pattern {
            source: "turn_stages",
            key: &["turn", "round"],
            eq: &[("stage", Lit::Str("decide"))],
        }],
        unless: &[],
    },
    // ── 自动语义压缩 ─────────────────────────────────────────────────────
    Decl {
        name: "policy.compress",
        prio: 30,
        // 水位要的那份请求级开销寄存进本轮状态，产线本身不动；它的入格归接缝行。
        by: Some(COMPRESS),
        // 语义压缩调模型：深度档。
        tier: LatencyTier::Deep,
        expand: None,
        owes: Pattern {
            source: "turn_head",
            key: &["turn", "round"],
            eq: &[
                ("auto_compress", Lit::Bool(true)),
                ("compress_done", Lit::Bool(false)),
                // 判决判成「就此答话」的一轮不进驱动循环（§5.1 / [D-162] 裁决 1）：
                // 压缩与召回都不该开账。同一视图的一格，不是跨键的 `unless`——后者
                // 与债键逐字比对，跨键时静默不命中（§3.4 第 2 条）。
                ("answered", Lit::Bool(false)),
            ],
        },
        until: &[Pattern {
            source: "turn_stages",
            key: &["turn", "round"],
            eq: &[("stage", Lit::Str("compress"))],
        }],
        unless: &[],
    },
    // ── 轮首准备（记忆 / 技能 / 任务段）─────────────────────────────────
    Decl {
        name: "policy.recall",
        prio: 40,
        // 出口那一格由执行体自己落——它不交回产出，但「已做」这件事照旧只写在日志上。
        by: Some(RECALL),
        // 事实投影的纯读 + 技能路由，不调模型。
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: "turn_head",
            key: &["turn", "round"],
            eq: &[
                ("round", Lit::Uint(0)),
                ("is_persistent", Lit::Bool(true)),
                ("recall_done", Lit::Bool(false)),
                // 同 [`policy.compress`] 的那一条：定稿答话轮不进驱动循环。
                ("answered", Lit::Bool(false)),
            ],
        },
        until: &[Pattern {
            source: "turn_stages",
            key: &["turn", "round"],
            eq: &[("stage", Lit::Str("recall"))],
        }],
        unless: &[],
    },
    // ── 反射档（技能快路）────────────────────────────────────────────────
    //
    // 续写幕不走快路（落格形状会撞幂等键）；`skill_hit` 来自本幕召回阶段的
    // `result`，所以「命中」这件事第一次有了事实形态。
    //
    // 出口是 `reflex` 阶段格（D-163）：执行体只落「已答」并把技能正文寄进本轮
    // 状态，收束格由 `close-turn` 统一落——快路自身再写 `final` 会让依赖图成环。
    // 收束行据此在**零模型往返**的轮上也能开出账（`turn_head.reflex_answered`）。
    Decl {
        name: "policy.reflex",
        prio: 50,
        by: Some(REFLEX),
        // 快路就是「不调模型」：反射档是它的定义。
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: "turn_head",
            key: &["turn", "round"],
            eq: &[
                ("resumed", Lit::Bool(false)),
                ("skill_hit", Lit::Bool(true)),
            ],
        },
        until: &[Pattern {
            source: "turn_stages",
            key: &["turn", "round"],
            eq: &[("stage", Lit::Str("reflex"))],
        }],
        unless: &[],
    },
    // ── 接缝（轮首那几行 → 请求之间的一次性装配）───────────────────────────
    //
    // 这一行是「一幕的开头」在行表上的形状：压缩入格、水位与根骨架、请求视图、定档、
    // 首响、基线 prompt 都要等轮首那几行寄存的值，因此不能住在幕首的直线代码里；
    // 它们又不是四次模型往返，所以也不必拆成四行。一行 = 一笔债，出口 = 那一格
    // `turn.stage{stage:"inputs"}`（它同时翻转 `turn_head.prepared`）。
    //
    // `prio` 52：轮首四行之后（它们的产物是它的前件）、收束与执行之前（还没定档、
    // 没渲染 prompt 的幕不欠一次往返）。轮内各行因此在 `owes` 上都带 `prepared: true`
    // ——让「接缝没跑」在事实面上是**开着的债**，而不是一个靠行序维持的假设。
    Decl {
        name: "prepare-turn",
        prio: 52,
        by: Some(SEAM),
        // 装配不调模型：快速档。
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: "turn_head",
            key: &["turn", "round"],
            eq: &[("prepared", Lit::Bool(false))],
        },
        until: &[Pattern {
            source: "turn_stages",
            key: &["turn", "round"],
            eq: &[("stage", Lit::Str("inputs"))],
        }],
        unless: &[],
    },
    // ── 中途汇报 ─────────────────────────────────────────────────────────
    //
    // 一段静默 = 一个 `(turn, anchor)`。汇报事实落格后静默锚前移，下一段静默
    // 是另一笔账——配额之内可以汇报多次，逐段关账而不是「一说就全关」。
    //
    // `prio` 53 把它排在接缝之后、收束之前：`report_due` 的 `at_boundary` 要求最近
    // 一条模型应答是一批**已关闭**的工具调用，那正是「下一笔债是再跑一次往返」的
    // 时刻——先报一句，再出发。它与其余行同处一个循环，先后只由 `prio` 说。
    Decl {
        name: "policy.report",
        prio: 53,
        by: Some(REPORT),
        // 模板产线，零模型往返。
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: "report_due",
            key: &["turn", "anchor"],
            eq: &[("due", Lit::Bool(true))],
        },
        until: &[Pattern {
            source: "reported_anchors",
            key: &["turn", "anchor"],
            eq: &[],
        }],
        unless: &[],
    },
    // ── 收束（/ 步 5c）────────────────────────────────────────
    //
    // 「这一轮该收束了、还没入格」是一笔独立的账：前件是日志上的 `turn.closeable`
    // （`has_final`〔本幕最近一条 `response(port=model)` 的 `calls` 为空 ∧ 本幕未收束，
    // D-121 决策 2〕∨「本幕派发过的模型往返以失败收场」），执行体是 [`CLOSE`]——
    // 方案（DE-015 §3.1）要的就是一个独立的 `closer`，不是整轮执行体里的第四个内部动作。
    // 它排在 `announce-model` **之前**（55 < 60）：应答一落地，下一个该还的债就是入格，
    // 而不是再跑一次模型往返——两行在同一切片上竞争，`drive` 取最小的一笔。
    //
    // ## 出口是 `final | fallback`（DE-015 §3.1 的字面形状，步 5c 复原）
    //
    // 步 5 第一批照抄 §3.1 装不上表：`announce-model` 与 `close-turn` 共用同一个出口
    // 事实集时，两个执行体互为对方的出口写方（`execute` 写错误路径的 `fallback`，
    // `close` 写 `final`），依赖图于是有 `close → execute` 与 `execute → close` 两条边
    // ——两节点环，装载红。环本身是**假阳性**（收束一旦入格两行同时关门），但第 1 条是
    // 字节判据面，改它要挂 ADR。
    //
    // 步 5c 断环的方式是**让两个执行体写的格不相交**，不是改判据：收束格
    // （`final` 与 `fallback`）全部归 `close` 执行体——`execute` 那一步只跑模型往返，
    // 收束格由 `close-turn` 行的执行体落（中止 / 适配器失败两条路径的收束格因此
    // 从执行体内部挪到了收束动作里）。断环之后 `close-turn` 没有出边（只有 `close`
    // 写它的出口，而自环不算），`announce-model → close-turn` 单独一条边，表装得上。
    //
    // 两条**控制面**失败（模型往返派不出去 / 工具批开不出来）仍在 `execute` 那一步
    // 就地落兜底格：它们的模型端口**没有终局事实**（`failed(port=model)` 落不下来）
    // ⇒ 收束行开不出账 ⇒ 那两格由 `execute` 写，并走它的**过渡豁免**（见
    // [`ACTOR_EXECUTE`] 的 `transitional`）。
    Decl {
        name: "close-turn",
        prio: 55,
        by: Some(CLOSE),
        // 入格不调模型：反射档（比执行体的 `mode` 更慢就装载红，第 6 条的 mode 闸门）。
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: "turn_head",
            key: &["turn"],
            eq: &[("closeable", Lit::Bool(true))],
        },
        until: &[
            Pattern {
                source: EVENT_CHAT_ASSISTANT_FINAL,
                key: &["turn"],
                eq: &[],
            },
            Pattern {
                source: EVENT_CHAT_ASSISTANT_FALLBACK,
                key: &["turn"],
                eq: &[],
            },
        ],
        unless: &[],
    },
    // ── 执行（决策行）────────────────────────────────────────────────────
    //
    // 名字不带幕（`D-222` 裁决 7）：这一行做的事一直是「宣布**这一次**模型往返」——
    // 落一条 `request(port=model)`，派出归 `dispatch-model`。名字里的 `turn` 是幕的
    // 残留，它会让读表的人以为这一行按幕执行。改名之后行表里不再有以幕命名的行。
    //
    // 只要本幕还开着、且手上没有一批没关掉的工具调用，就欠一次执行。它的**后果**是
    // 写一条 `request`（决策者的动作），派发本身归 `dispatch-model`。没把它并进协议行：
    // 两者的粒度不同（本幕 vs 一次请求往返）——一幕可以有多次往返（auto 续写），
    // 而「本幕还欠不欠」只有一个答案。
    //
    // `tools_open = false`（b）：模型交回了工具调用、其中还有没关账的，
    // 这一步就让位——否则它每次都比创建 / 授权 / 派发（67 / 68 / 69）先生效，
    // 工具批永远等不到人跑（下一轮模型往返会带着没有结果的交换再次出发）。
    //
    // `reflex_answered = false`（D-163）：快路已作答的轮**没有要答的东西**，本行不欠。
    // 这条前件替代了「按优先级让 close 先跑」的时序约定——零模型往返因此是行表上的
    // 判据，不是两个数字（55 / 60）碰巧凑出来的次序。
    //
    // `model_open = false`（D-166）：本行只**宣布**一次往返（落 `request(port=model)`），
    // 派出归 `dispatch-model`。宣布之后那笔账还开着，本行必须让位——否则 `prio` 60
    // 会压过 66 再宣布一笔，同一幕两条请求并存，「一次模型往返」这个粒度当场失真。
    Decl {
        name: "announce-model",
        prio: 60,
        by: Some(EXECUTE),
        tier: LatencyTier::Deep,
        expand: None,
        owes: Pattern {
            source: "turn_head",
            key: &["turn"],
            eq: &[
                ("tools_open", Lit::Bool(false)),
                ("reflex_answered", Lit::Bool(false)),
                ("model_open", Lit::Bool(false)),
            ],
        },
        until: &[Pattern {
            source: "turn_closed",
            key: &["turn"],
            eq: &[],
        }],
        unless: &[],
    },
    // ── 端口 `model` 的两行──────────────────────────
    //
    // 每端口固定两行：`dispatch-<port>` 与 `await-<port>`，差异只有 `by`。第三行
    // （租约）**不在表上**：看护读的是 `stale_sent` 视图（哪些尝试过期了），不经过
    // 行表——「该还哪一笔」只有环能回答，看护问的不是那个问题（`TC-1b`）。
    // 模型调用是轮内的一次往返。
    //
    // 键同样取 `(port, id, attempt)`（D-138）：逻辑 id 在一次调用的各次尝试间不变，
    // 尝试号随重试 +1；按逻辑键开账会让上一 attempt 的 `sent` 把重试后的请求提前关掉。
    //
    // ## 端口粒度为什么是「一次模型往返」而不是「一次工具批次」
    //
    // 模型调用是**最内层**的单次对外调用（它下面没有别的调用了），因此它是最小
    // 的可恢复单位：崩溃后重放到「上一次模型请求」即可，不必重放整轮。工具批次
    // 在它**外面**（一个模型响应可带多个工具调用），所以工具端口的 `expand` 是
    // 下一层的事（DE-015 步 3）。
    //
    // ## 谁宣布、谁派出（D-166）
    //
    // `announce-model`（60）宣布这一笔（落 `request`），由本行的 [`RELAY`] 执行体
    // 跑出 `sent` + `response` / `failed`。两个时刻分开的理由不是「拆得好看」：
    // 宣布之后那一笔在日志上是一笔**开着**的债，派发由选债触发（不是执行体内部
    // 顺手调一次往返），所以「这一笔该谁跑」在事实面上可判、在崩溃后可接。
    //
    // 与 `dispatch-tools` 不共用一个执行体名：一个 `by` 一个动作（D-118 决策 1–2），
    // 而 `courier` 的动作派发的是**一批**（批尾关账、落等待格与轮内注入），
    // 与「跑一次往返」不是同一件事——共名会让动作只能按行名分支。
    Decl {
        name: "dispatch-model",
        prio: 66,
        by: Some(RELAY),
        // 派发本身不调模型（模型在 [`ACTOR_RELAY`] 的动作里，协议格仍由引擎写）。
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: EVENT_REQUEST,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("model"))],
        },
        // 出口**只有 `sent`**（`D-222` 订正 2）：落完它这一行的账就还清了，「在等」
        // 由 `await-model` 认领。此前出口是 `response|failed`，于是派出与收应答在同一个
        // 动作窗口里，两次选债之间「已派出、未收清」在正常跑里读不出来。
        until: &[Pattern {
            source: EVENT_SENT,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("model"))],
        }],
        unless: &[],
    },
    // 工具调用的**创建**（D-117）：一个模型响应的 `calls` 集合逐元素开债。
    //
    // 与派发行分开：创建行只回答「这次响应里的调用有没有落成 request」，派发行
    // 才回答「已放行的那一笔有没有 sent」。把两者合成一行会让授权拒绝失去
    // 作用点——被拒绝的调用同样有模型响应，派发会绕过 `authz.denied`。
    //
    // `expand` 只展开 `owes`；`until` 按键命中。元素键 `id` 由模型响应携带
    // （`tool-{turn}-d{drive}-{模型调用 id}`），与随后的 `request(port=tool)` 同 id。
    Decl {
        name: "open-tools",
        prio: 67,
        // 创建动作（b）：逐条落 `request(port=tool)`，不调模型、不跑工具。
        by: Some(OPENER),
        tier: LatencyTier::Fast,
        expand: Some(("calls", "id")),
        owes: Pattern {
            source: EVENT_RESPONSE,
            key: &["port", "id"],
            eq: &[("port", Lit::Str("model"))],
        },
        until: &[
            Pattern {
                source: EVENT_REQUEST,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("tool"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("tool"))],
            },
        ],
        unless: &[],
    },
    // 授权级联的第二行（第一行 `authorize-tool` 在表首）：只对已放行的那笔
    // 落 `sent`。`granted` 已蕴含 `request`，所以这里不再复写请求前件。
    //
    // **本行只到 `sent` 为止**（`D-124` 订正 1）：交出去就关账，收应答是 `await-tools`。
    // 第二格 `failed` 是控制面失败的出口（`port::begin` 拒了这一笔：行不欠 / 端口没声明）
    // ——那条路上没有可等的往返，就地判死；留着它才不会让这一行永远开账。
    Decl {
        name: "dispatch-tools",
        prio: 68,
        // 派发动作（b）：放行的落 `sent` 交给执行层。批尾关账归 `waiter-tools`。
        by: Some(COURIER),
        // 派发本身不调工具（执行在批次里）。
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: EVENT_AUTHZ_GRANTED,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("tool"))],
        },
        until: &[
            Pattern {
                source: EVENT_SENT,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("tool"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("tool"))],
            },
        ],
        unless: &[],
    },
    // 被拒的调用也要有人收尾（b）：`authz.denied` 关掉了授权行，但
    // `dispatch-tools` 只欠 `granted`——一批**全被拒**（或全是非法调用）时它不欠，
    // 执行层就永远跑不到，`failed{why}` 也永远不落。这一行把「拒绝之后还必须做一次
    // 收尾」说成债务：出口与派发行同一格（`failed`），动作归**收尾半**
    // （混合批次里那一笔由 68 起批、由本行与 `await-tools` 一起收掉，收完本行自然不欠）。
    //
    // 为什么 `by` 是 `waiter-tools` 而不是 `courier`（`D-124` 订正 1）：这一格是
    // 批尾关账写出来的，而批尾在收尾半；派发半在拒绝路径上一格都不落
    // （`begin_calls` 只写放行的那些），把它挂在派发半，名单与写点就对不上。
    //
    // 键取 `(port, id, attempt)` 与 `authz.denied` 同形（D-138）；不带 `port`
    // 的收束格拒绝（`deny()` 写的那一条）按 eq 取不到实例，不会误开一笔工具债。
    Decl {
        name: "deny-tools",
        prio: 69,
        by: Some(WAITER_TOOLS),
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: EVENT_AUTHZ_DENIED,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("tool"))],
        },
        until: &[Pattern {
            source: EVENT_FAILED,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("tool"))],
        }],
        unless: &[],
    },
    // 「已派出、等应答」：**可还**（`D-222` 订正 2）。动作取走寄存的那一次往返并等它
    // 落地——`by: None` 时代它恒 blocked、永不选中，环选不到它就停（`exhausted{waiting}`），
    // 而模型已经发出去了：那一笔应答永远收不回来。`by = None` 从此只表示「这一笔不由
    // 引擎执行」，「在等」的唯一表达是 `waiting` 视图（`sent` 有 `response` 无）。
    //
    // 键取 `(port, id, attempt)`（D-138）：`sent` / `response` / `failed` / `retry`
    // 都是同一次尝试的事实。
    Decl {
        name: "await-model",
        prio: 71,
        by: Some(WAITER),
        tier: LatencyTier::Deep,
        expand: None,
        owes: Pattern {
            source: EVENT_SENT,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("model"))],
        },
        until: &[
            Pattern {
                source: EVENT_RESPONSE,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("model"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("model"))],
            },
            Pattern {
                source: EVENT_RETRY,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("model"))],
            },
        ],
        unless: &[],
    },
    // 一批 N 笔，一笔一条 `sent`；`await-tools` 与其它端口的 await 同形。
    //
    // 键取 `(port, id, attempt)`：重试后新 attempt 的 `sent` 要重新开一笔等待，
    // 而不是被旧 attempt 的应答关掉（D-138）。
    //
    // `by` 从 `None` 变成 `waiter-tools`（`D-124` 订正 1）：与 `D-222` 订正 2 对
    // `await-model` 做的事逐字相同——`by = None` 时代它恒 blocked、永不选中，
    // 「在等」只在崩溃恢复的输入上才有值；接上执行体之后，派出与应答分处两个环步，
    // 中间那一圈「`sent` 有、终局无」在**正常跑**里读得到。
    Decl {
        name: "await-tools",
        prio: 72,
        by: Some(WAITER_TOOLS),
        // 等的是工具执行层，不调模型：这一行的时延档与 `dispatch-tools` 同档
        // （`tier` 判的是「这一行要不要模型句柄」，不是「会占多久」）。
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: EVENT_SENT,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str("tool"))],
        },
        until: &[
            Pattern {
                source: EVENT_RESPONSE,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("tool"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("tool"))],
            },
            Pattern {
                source: EVENT_RETRY,
                key: &["port", "id", "attempt"],
                eq: &[("port", Lit::Str("tool"))],
            },
        ],
        unless: &[],
    },
];

/// 装载：六张注册表经 DE-015 §3.2 的注册口进 `Builder`，`seal` 跑完全部装载期检查。
///
/// 行切片与端口切片从这里进 `row` / `port` 两个入口——检查跟着表走，不由调用方
/// 自己想起来再跑一遍：`seal` 是装载的唯一出口。
///
/// **任何缺口都是装载失败**——`seal` 一次报全的每一格（声明侧重名 / 撞名 / 视图引用
/// 未声明的事实或字段 / 执行体声明了引擎独占的格，行侧无出口 / 键不一致 / 未声明源 /
/// 未声明字段 / 展开键不匹配 / 孤儿事实名 / 无人认领的 `by` / 收不了口的执行体 /
/// 依赖成环 / 端口没有应答出口）都在同一个 `Vec<Gap>` 里，不降级成告警：一个没人
/// 消费的事实名就是判据 7 拦的那条「拼错的名字静默永不被消费」，它在装载期就该挡住启动。
///
/// 执行体声明与绑定项一起交（[`actor_decls`] 就是绑定表的 `decl` 投影），所以装载期
/// 能看到的名字与派发器跑得动的名字**同源**。
/// 控制面声明集的 owner（DE-023 D-178）：这张表由 `plugins/loop` 交进装载口，
/// 事实与视图的 owner 清单因此都指向这里。
pub(crate) const LOOP_OWNER: &str = "plugins/loop";

pub(crate) fn load(rows: &[Decl]) -> Result<Arc<Frozen>, Vec<Gap>> {
    let mut b = Builder::new();
    b.declare(LOOP_OWNER, declarations(rows));
    let frozen = b.seal()?;
    // **每一档守卫角色都得有人住**（DE-024 D-203）。判据读的是角色，角色来自声明：
    // owner 删掉一行 `roles` 的表现不是「拒绝」而是「看不见」——守卫从拦人变成空转，
    // 装载还是全绿。所以钉在装载口，与其余缺口同一个 `Vec<Gap>`，不降级成告警。
    let unmanned: Vec<Gap> = frozen
        .unmanned_roles()
        .into_iter()
        .map(|role| Gap::UnmannedRole {
            role: role.as_str(),
        })
        .collect();
    if !unmanned.is_empty() {
        return Err(unmanned);
    }
    Ok(frozen)
}

/// 本插件交给装配口的完整声明（DE-024 D-198）。`load` 与链接期提交共用这一份，
/// 避免「私有单例」与「装配表」两份形状各写一遍。
pub(crate) fn declarations<'a>(rows: &'a [Decl]) -> PluginDeclarations<'a> {
    PluginDeclarations {
        events: super::control_decl::EVENTS,
        facts: super::control_decl::FACTS,
        views: super::control_decl::VIEWS,
        ports: PORTS,
        actors: actor_decls(),
        rows,
    }
}

// 装配口的提交：宿主链接本 crate 即生效，不需要谁记得调用一次。
// 装配方（`symbio_plugin_api::sealed_declarations`）按 owner 排序后逐个 `declare`，
// 本插件不认识也不依赖任何兄弟插件。
symbio_plugin_api::submit_declarations!(LOOP_OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    declarations(ROWS)
}

/// 真表装载，并把 §3.4 第 4 条的报告打出来。
///
/// 第 4 条是**报告**不是缺口：静态只能判「可能同时命中」，谁先派发归 `drive` 的
/// `(prio, name, key)`。名单随表冻结（[`Frozen::overlaps`]），这里只负责让人看见。
fn assemble() -> Result<Arc<Frozen>, Vec<Gap>> {
    // 装配期冻结表（DE-024 D-198）：本插件经 `submit_declarations!` 交表，兄弟插件
    // 的事实 / 事件也在这张表里。只剩「链接期提交没生效」这一种情况会得到缺行表
    // （例如某个测试二进制用链接器参数裁掉了未引用的提交段）——此时退回本插件
    // 自己的 `load(ROWS)`，行为与迁移前一致；生产装配不走这条退路。
    let frozen = symbio_plugin_api::sealed_declarations()?;
    let frozen = if frozen.rows().is_empty() || frozen.actors().is_empty() {
        load(ROWS)?
    } else {
        frozen
    };
    let overlapping = frozen.overlaps();
    if !overlapping.is_empty() {
        symbio_core::plugin_info!(
            "session",
            "[Loop] 相交行对 {} 组（可能同时开账，裁决归 prio）：{}",
            overlapping.len(),
            overlapping
                .iter()
                .map(|o| format!("{}×{}@{}", o.rows[0], o.rows[1], o.facts.join("+")))
                .collect::<Vec<_>>()
                .join(" / ")
        );
    }
    Ok(frozen)
}

/// 冻结的声明表（进程内唯一）。装载有任何缺口 ⇒ `None`，[`table`] 恒 `Err`。
///
/// ## 为什么进程级单例 ≡ 按会话注入
///
/// 六张表灌进 `Builder` 的全是 `&'static` 数据（`Decl` / `PortDecl` / `ActorDecl` 的
/// 每个字段都是 `&'static str` / `&'static [_]` / `Copy`），`seal(self)` 又是**消耗**
/// `self` 的：同一份程序文本只能冻出同一张表，按会话构造得到的就是这个值。于是
/// `LazyLock` 不是「一条全局可变注册通道」，而是一个纯函数的记忆化——它的
/// `Option` 缓存的是**失败**（缺口已逐条进 error 日志），不是可写的状态。
///
/// 与按会话注入真正不同的只有**开幕时机**：这里首次查询即装载，失败缓存后每一次开幕
/// 都带同一个 `Err` 退出（[`table`]），不存在「这一届会话拿到了半张表」。类型面上
/// `Session` 只持 `Arc<Frozen>`，`Frozen` 没有 `&mut self` 方法、没有内部可变性。
static FROZEN: LazyLock<Option<Arc<Frozen>>> = LazyLock::new(|| match assemble() {
    Ok(frozen) => Some(frozen),
    Err(gaps) => {
        symbio_core::plugin_error!(
            "session",
            "[Loop] 行表装载失败，义务环拒绝启动（开幕带错退出）：{:?}",
            gaps
        );
        None
    }
});

/// 冻结的声明表（租约看护者自己也要求它——它不重复装载一份）。
///
/// 装载有任何缺口 ⇒ `Err`：调用方（开幕）带着它退出，不存在「表没装好但轮照常跑」
/// 这种部分成功。
pub(crate) fn table() -> Result<Arc<Frozen>, symbio_core::PluginError> {
    frozen().ok_or_else(|| {
        symbio_core::PluginError::InternalError(
            "义务环行表装载失败（装载期检查有缺口），本轮拒绝开幕；详见启动日志 [Loop] 行表装载失败".into(),
        )
    })
}

fn frozen() -> Option<Arc<Frozen>> {
    FROZEN.clone()
}

/// 一份切片最多能有多长（[`Frozen::scan_bound`] 的转发）。
///
/// 存在的理由是**分层**：调用方（`turn_slice`、`chat_loop::lease_tail`）不该自己去
/// 遍历视图表算最小值，也不该各自抄一个常数——切点只能有一个真源。表还没装好时
/// 回 `None`，由调用方按「不切」处理（那一轮的打开本来就会先被 [`table`] 拦下）。
pub(crate) fn scan_bound() -> Option<usize> {
    frozen().and_then(|f| f.scan_bound())
}

/// 本轮的事实切片：视图只扫**一轮**（声明的成本上界针对轮，不针对整份 WAL）。
///
/// 轮号单调，因此「本轮第一条事件」就是本轮窗口的起点；续写幕复用轮号时，
/// 首幕的事件也在切片里，而视图的 `drive_of` 会按**最新幕锚**收窄。
///
/// ## 长度也要切，而且切点不在这里
///
/// 只切起点是不够的：一轮的事实条数没有上界（工具批次、多步往返都会往同一轮里
/// 追加），而 `Frozen::eval` 在条数超过视图声明的 `cost` 时回 `None`——`ring::owed`
/// 把那个 `None` 折成空集（`_ => Vec::new()`，字节冻结，`PTS-003`）。于是**整轮
/// 所有行**在那一轮里都不开账：环看起来「没账可还」，实际是「算不出来」。
///
/// 修法不在求解器，就在这两行之间：先按 [`Frozen::scan_bound`]（全表最小的 `cost`，
/// 唯一能同时喂给每一条视图的上界）切到窗口内，超界时**留一条告警**——静默降级
/// 正是本条要收的那一格。切掉的是一轮里更早的事实：它们的 `request` 会缺席（那一
/// 笔债还不上，环空转），但绝不会让「已应答」缺席（`response` 总在尾部），所以失效
/// 方向是「多开账」而不是「漏还账」。
///
/// `scan_bound` 为 `None`（表还没装好）时原样交出：这一轮的打开本来就会先被
/// [`table`] 拦下，这里不替它判。
fn turn_slice(events: &[Event], turn: u64) -> &[Event] {
    let slice = match events.iter().position(|e| e.turn == turn) {
        Some(start) => &events[start..],
        None => &[],
    };
    let Some(bound) = frozen().and_then(|f| f.scan_bound()) else {
        return slice;
    };
    if slice.len() <= bound {
        return slice;
    }
    tracing::warn!(
        "本轮事实 {} 条超过视图声明的扫描上界 {}（`Frozen::scan_bound`）⇒ 只按尾部 {} 条算账；\
         超界部分对 `owed` 不可见，本轮可能多开账。要么调大 `control_decl` 的视图 `cost`，\
         要么把这一轮拆短",
        slice.len(),
        bound,
        bound
    );
    &slice[slice.len() - bound..]
}

/// 本次 `drive` 的 `now`：**幕锚那条事实的 `ts`**（判据 12，DE-015 §2.1）。
///
/// 幕锚 = `drive_seq`：新轮是用户格，续写幕是恢复阶段事实——两者都是**触发本次
/// drive 的那条事实**。墙钟进入日志的唯一合法通道是落这条事实时 `Store::append`
/// 补的落格时刻，此后本次 drive 的所有时间条件（静默、租约）都只读它的 `ts`。
/// 于是同一个日志重放两次，`now` 相同，`owed` 逐字相同；而现取墙钟会让「回放可
/// 重建」在这些路径上成为假话。
///
/// 切片里没有这一条 ⇒ `0`（切片比幕旧，或幕锚填的是占位值）：没有任何时间条件
/// 可以满足，宁可判「还不该做」也不拿一个来路不明的时刻去凑。
pub(crate) fn wake_now_ms(events: &[Event], drive_seq: u64) -> i64 {
    events
        .iter()
        .find(|e| e.seq.is_some_and(|s| s.value() == drive_seq))
        .map(|e| e.ts)
        .unwrap_or(0)
}

/// 这一行此刻**欠着**吗（`owes` 命中且未被 `until` 关掉）。
///
/// 这是 §2.2 的 `open`，不是 `actionable`：`by = None` 的行照样欠着。「在等」与
/// 「没事」可区分正是 `by = None` 存在的理由，所以点问口不能把它折掉——调用点
/// （`port.rs` 的往返、`tool_dispatch` 的三段、`chat_loop` 的策略段）问的都是
/// 「这笔账在不在」，而「引擎该不该现在去还」只由 [`next`] 回答。
///
/// 返回布尔而不是 `Debt`：调用点只决定「这一步
/// 做不做」。要裁决顺序（同一点多行竞争）时用 `ring::drive`，本函数不代劳。
pub(crate) fn owes(name: &str, events: &[Event], turn: u64, now: i64) -> bool {
    let Some(frozen) = frozen() else {
        return false;
    };
    let debt = owed(frozen.rows(), &frozen, turn_slice(events, turn), now)
        .into_iter()
        .find(|d| d.decl.name == name);
    if let Some(debt) = &debt {
        trace(&frozen, debt);
    }
    debt.is_some()
}

/// 一段连续幕的上限与「还有没有下一次」：定义搬到视图那一侧
/// （`control_decl::MAX_STAGE_ROUNDS` / `has_next_round`），这里只留路径——
/// 上界是那一笔债的前件（`D-222` 裁决 3），而前件住在视图与行表那一层。
pub(crate) use super::control_decl::{has_next_round, MAX_STAGE_ROUNDS};

/// 幕间判据：本幕停下之后**还开不开下一幕**——开的话，下一幕的轮次是多少。
///
/// `None` = 收工；`Some(round)` = 再开一幕，轮次取 `round`。
///
/// ## 它只读一格
///
/// 两条判据（`closure = exhausted` ∧ 轮次未到上限）折在视图 `turn_next_round` 里
/// （`D-222` 裁决 2），这里只把那一格取出来。判据留在代码里的时候，流向就是
/// 「按步骤号决定下一步」——那正是判据 2 的最后一格；折进声明之后，同一个问题
/// 在事实面上有一个答案，`continue-turn` 那一行立起来时直接把它当 `owes`。
///
/// 轮次从事实里读，不从内存里数：`round` 是本幕开账时落下的
/// `turn.stage{stage:"round"}` 那一格的值，`round + 1` 就是下一幕的轮次，
/// 上界比对的是同一个数（`TurnState::continuation_count` 因此退场）。
pub(crate) fn next_stage_round(events: &[Event], turn: u64, now: i64) -> Option<u64> {
    let frozen = frozen()?;
    let next = frozen.eval("turn_next_round", turn_slice(events, turn), now)?;
    next.as_array()?.first()?.get("round")?.as_u64()
}

/// 这一刻环上有没有**已派出、未收清**的账——「在等」的可观测形态。
///
/// ## 为什么按形状问，不按行名 / 不按 `blocked` 问
///
/// 判据就是 [`open_calls`]：有一条 `sent` 没有被同 `(id, attempt)` 的
/// `response` / `failed` / `retry` 关掉。**刻意不按行名前缀匹配**，也刻意不看
/// 「哪一行 blocked」：派发器不认行名（D-118 决策 1），而 `await-*` 行在
/// `D-222` 订正 2 之后是**可还的债**（`by = Some(..)`），`blocked` 这个位不再
/// 携带「在等」——「在等」剩下的唯一写法就是这条形状。换了行名、换了 `by`、
/// 甚至删掉整张 `await-*` 行，本函数照样成立。
///
/// ## 它存在的理由：`await-*` 行不是装饰行
///
/// [`super::turn_exec::drive`] 选不到可还债时，有两种**完全不同**的处境：真的没事
/// 可做，与有一笔往返还在路上。停机理由此前一律写 `no_progress`——于是「这一轮为什么停」在事实面
/// 上不可区分，而「在等」恰恰是恢复路径最需要看见的那一种（进程在等应答时被打断，
/// 重启后这笔账要靠 `port::reap` 判死，前提是它当时确实被记成「在等」）。
///
/// 第二个读者是步数上界（同一个 `drive`）：上界是**收工的判据**，不是「半路撂下」的
/// 许可——有一笔在飞时先把它收回来，否则日志上留下一条 `sent` 无终局，重启后
/// `reap` 要判死一笔**其实已经答了**的往返。
pub(crate) fn waiting(events: &[Event], turn: u64, _now: i64) -> bool {
    !super::control_decl::open_calls(turn_slice(events, turn)).is_empty()
}

/// 引擎的**选债口**：本幕当前最小的一笔可还债，连带它的动作。
///
/// 与 [`owes`] 的分工：`owes` 回答「某一行欠不欠」，本类型回答「下一步谁欠、
/// 跑什么」。[`next`] 是 [`super::turn_exec::drive`] 的唯一选债依据（D-118 决策 1–2）——
/// 派发器不按行名 / 端口名 / 业务语义分支，只调 `Next.action` 这**一个**动作。
pub(crate) struct Next {
    /// 行名（日志与停机理由用）。
    pub(crate) row: &'static str,
    /// 执行体名。`drive` 已滤掉 `by: None`，这里必有值。
    pub(crate) by: &'static str,
    /// 债键的人读形状（日志用）。
    pub(crate) key: String,
    /// 按 `by` 在绑定表里查出的动作。查不到就不会有这一项（见 [`next`] 的 `None`）。
    pub(crate) action: super::turn_exec::Action,
    /// 同一个动作的**声明面**：它登记过会写哪些领域格。
    ///
    /// 派发器拿账本（日志尾段）对它对账（[`Next::writes_closed`]）。放在 `Next` 上而不是
    /// 让派发器再查一次表，是因为「跑哪个动作」与「它写了什么」必须同源——
    /// 分两次查就能查出两份答案（D-123 决策 2）。
    pub(crate) writes: &'static [&'static str],
    /// 这条执行体的**过渡豁免名单**（[`ActorEntry::transitional`] 的同一份，同源）。
    pub(crate) tolerated: &'static [&'static str],
}

/// 取最小可还债；没有可还的债 ⇒ `None`（收束 / 装载失败 / 只剩 blocked 行）。
///
/// 这是 `ring::drive` 的生产包装：切片只含**本轮**事件（视图成本上界按轮），
/// 并复用与 [`owes`] 同一份冻结声明表。装载失败返回 `None` 而不是 panic——
/// 派发器据此停住，不拿半张表继续走。
///
/// 可还性只有一个声明位：`owed` 的 `blocked`（`by = None ∨ unless`）。
/// 「注册表没覆盖的 `by`」不是第四个过滤条件——那种行直接写 `by = None`，
/// 接回时与它的 `ActorEntry` 同批改（D-118 订正 1）。
///
/// `actors` 是派发器手里的那张绑定表（生产 = [`ACTORS`]，D-123 决策 4）：行表决定
/// 「谁欠什么」，这张表决定「这个名字跑哪个动作、它登记写哪些格」，读账本对账用的
/// 名单也来自同一处。
///
/// 送给求解器的是**由这张表派生的行子集**（[`bound_rows`]）：求解器字节一字未动，
/// 而「这一行有没有人跑」始终只有一个读法。
pub(crate) fn next(
    events: &[Event],
    turn: u64,
    now: i64,
    actors: &'static [ActorEntry],
) -> Option<Next> {
    let frozen = frozen()?;
    let rows = bound_rows(&frozen, &actor_names(actors));
    let debt = symbio_core::ring::drive(&rows, &frozen, turn_slice(events, turn), now)?;
    trace(&frozen, &debt);
    // 动作与声明**一次查全**：查不到动作就按「没有可还的债」显形并出声（不猜动作、
    // 也不猜它写了什么）——那是行表与绑定表被改坏的形状，不是第四种可还性。
    let Some(entry) = debt.decl.by.and_then(|by| entry_of(by, actors)) else {
        symbio_core::plugin_warn!(
            "session",
            "[Loop] 行表与执行体绑定表脱节（行 {} 的 `by={:?}` 没有动作），本步不派发",
            debt.decl.name,
            debt.decl.by
        );
        return None;
    };
    Some(Next {
        row: debt.decl.name,
        by: debt.decl.by?,
        key: debt.key.trace(),
        action: entry.run,
        writes: entry.decl.writes,
        tolerated: entry.transitional,
    })
}

/// 按声明求一个视图值（本轮切片）。未声明 / 扫描超界 ⇒ `None`。
pub(crate) fn view(name: &str, events: &[Event], turn: u64, now: i64) -> Option<Value> {
    frozen()?.eval(name, turn_slice(events, turn), now)
}

/// 这一行携带的档位。未登记 ⇒ 深度档（fail-safe：不静默提速）。
///
/// 读的是**冻结表**里的那一行（`frozen.rows()`），不是 `static ROWS`：装载入口用
/// `static` 灌表，读侧只认表——否则「改了 static 但表没重装」这种漂移就没有出口。
pub(crate) fn tier(name: &str) -> LatencyTier {
    frozen()
        .and_then(|frozen| {
            frozen
                .rows()
                .iter()
                .find(|d| d.name == name)
                .map(|d| d.tier)
        })
        .unwrap_or(LatencyTier::Deep)
}

/// 本幕的**服务档**：快路作答 ⇒ `policy.reflex` 那一行携带的档，否则深度档。
///
/// ## 为什么读 `reflex_answered` 而不是 `skill_hit`
///
/// `skill_hit` 是 `policy.reflex` 的**开账前件**（召回命中），`reflex_answered` 是
/// **已作答**（`stage:"reflex"` 且 `result` 非 `miss`）。两者在正常路径等价，但
/// `miss` 那一支（命中集为空、行自己关账）只有后者分得出来——照 `skill_hit` 定档会给
/// 一个要跑模型往返的幕发 80ms 的预算，`I3` 当场误报（[`D-223`] 订正 1）。
///
/// ## 为什么读视图而不是读执行体的寄存物
///
/// 寄存物（`TurnState::head_reflex`）是**执行侧**的副本，视图是事实的公开形态。档位要
/// 入的是幕级事实（`turn.dispatch`），写它的那一侧（接缝）手里只有事实源——读视图让
/// 「档位」与「快路行欠不欠」逐字同源，不由「两边记得同步」保证。
///
/// [`D-223`]: ../../../../docs/003-design/022-行表与派发面.md "DE-022-行表与派发面"
pub(crate) fn turn_tier(events: &[Event], turn: u64, now: i64) -> LatencyTier {
    let answered = view("turn_head", events, turn, now)
        .and_then(|v| v.get("reflex_answered").and_then(|b| b.as_bool()))
        .unwrap_or(false);
    if answered {
        tier("policy.reflex")
    } else {
        LatencyTier::Deep
    }
}

/// 一笔可还的**可观测面**：行 / 债标识 / 档位 / 视图口径。与旧 `Hit::trace` 同一
/// 目的——「某一步怎么发生了」在日志里要能指认到行与那一笔债，而不只是行名本身。
///
/// 视图版本一并记下（`ViewDecl.ver`）：投影读法一变，同一份事实折出的结论就变，
/// 回放时「这一轮用了哪一版」必须在日志里有答案。
fn trace(frozen: &Frozen, debt: &symbio_core::ring::Debt<'_>) {
    let decl = debt.decl;
    let key = debt.key.trace();
    let view = frozen.view(decl.owes.source);
    symbio_core::plugin_debug!(
        "session",
        "[Loop] 行 {} 可还（{} / {}ms；债 {}；视图 {} v{}）",
        decl.name,
        decl.tier.name(),
        decl.tier.budget_ms(),
        if key.is_empty() { "-" } else { key.as_str() },
        decl.owes.source,
        view.map(|v| v.ver).unwrap_or(0)
    );
}

#[cfg(test)]
#[path = "loop_rows.test.rs"]
mod tests;
