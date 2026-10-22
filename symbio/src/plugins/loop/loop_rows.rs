//! 义务环的行表（T-222）：五条策略行 + 一条执行决策行 + 端口的三条行（T-223）。
//!
//! ## 一行只写「什么时候欠、什么算还」
//!
//! 判据全部是数据：`owes` / `until` / `unless` 三个模式，`prio` 是**全局**裁决序
//! （`(prio, name, key)` 升序）。执行体只拿到一笔 [`Debt`](crate::symbio_core::ring::Debt)，
//! 不再读任何请求作用域状态——旧 `LoopState` 的五个变体在这里对应五个视图读到的事实。
//!
//! ## 五条策略行的前后关系
//!
//! ```text
//!  0 authorize-close ── 收束授权：有 `turn.closing` ⇒ 交回 granted / denied（ADR-116）
//!  5 close-turn      ── 收束：`granted` ⇒ 落收束格（`denied` ⇒ 本行永不成立）
//! 10 classify      ── 轮首判决（本幕 round=0 且有发言）
//! 20 report        ── 中途汇报（一段静默到期，未用完配额）
//! 30 compress      ── 自动压缩（开关开着且本幕还没压过）
//! 40 recall        ── 记忆 / 技能 / 任务段（有持久事实源且本幕还没召回）
//! 50 reflex        ── 技能快路（非续写幕且本幕召回命中）
//! 60 execute       ── 执行决策：本幕未收束 ⇒ 写一条 `request{port:turn}`
//! 65 dispatch-turn ── 端口派发：有 `request` ⇒ 引擎落 `sent` 并跑执行体
//! 66 dispatch-model─ 端口派发：模型往返（T-227 步 2，在 `turn` 执行体内部）
//! 67 dispatch-tool ─ 端口派发：工具批（T-227 步 3，同上）
//! 70 await-turn    ── 等应答：`by = None`，唯一的「在等」声明位
//! 71 await-model   ── 等应答（模型端口）
//! 72 await-tool    ── 等应答（工具端口）
//! 80 lease         ── 租约看护：`sent` 超时无应答（一个行管所有端口）
//! ```
//!
//! 收束的两行排在**最前**（`prio` 0 / 5）：一轮走到收束点时 `execute-turn`（60）
//! 仍然欠着（它的 `until` 是 `turn_closed`），收束若排在它后面，单环驱动会先再执行
//! 一次而永远收不了尾。数字不取 ADR-116 写的 0 / 10，是因为 10 已被 `policy.classify`
//! 占着，而表要求 `prio` 严格递增——为凑一个整数把十二条行全重编号，是给读者
//! 制造一次假 diff。
//!
//! 顺序进表而不是进调用点：同一点上多行竞争时，`drive` 取最小的一笔。旧表的
//! `prio` 只是格内序，这里第一次跨行可比（T-222 判据 10）。
//!
//! ## 一行**不在表里**的条件（写在这里，不写在事后）
//!
//! **汇报的轮次前提**：`轮数 > 0 ∧ 轮数 ≥ progress_min_rounds`。日志里没有
//! 「当前工具轮数」这一格（`chat.assistant.reported.tool_rounds` 只在说出口之后
//! 才有），所以它归触发点——`round_report_hook` 自己数批次边界，表判「静默够久
//! ∧ 配额未满」。`report_due` 视图把 `min_rounds` 一并交出，触发点因此也不用读配置。
//!
//! ## 查询口：事实是输入，`now` 也是输入
//!
//! [`owes`] / [`view`] 只收事实切片、轮号与 `now`——不读配置、不读时钟、不碰内存
//! 缓存（ADR-115 决策 1）。切片由 [`turn_slice`] 收窄到本轮：视图声明的扫描成本
//! 上界针对**一轮**，不是整份 WAL。
//!
//! ## 装载期检查
//!
//! 首次查询时冻结声明表并跑 [`check_rows`]：`OrphanFact` 是**已知**的后续任务
//! （协议四名 / `turn.dispatch` / `authz.denied`），记 info；其余缺口（无出口、
//! 键不一致、未声明源）记 error。装载失败 ⇒ 表空 ⇒ 永不前进，但系统其余部分
//! 照常运行（T-222 判据 6）。

use std::sync::{Arc, LazyLock};

use serde_json::Value;

use crate::symbio_core::ring::{check_rows, owed, Decl, Gap, Lit, Pattern, PortDecl};
use crate::symbio_core::{
    builtin, Event, Frozen, LatencyTier, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL,
    EVENT_AUTHZ_DENIED, EVENT_AUTHZ_GRANTED, EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE,
    EVENT_RETRY, EVENT_SENT, EVENT_TURN_CLOSING,
};

/// 端口声明（T-223）——**加一个端口只加这一条**，不加事实名、不改求解器。
///
/// 当前三个端口：`turn`（一轮的一次请求往返）、`model`（一次模型往返，T-227 步 2）、
/// `tool`（一次工具批，T-227 步 3）。
///
/// ## 租约与尝试上限怎么定的
///
/// `lease_ms` 是「派出去多久没应答算卡住」：`turn` 是一次模型往返，且含工具轮，
/// 慢的那一头可能是模型排队——太短会把正在进行的一轮误判成卡住（重试会再调一次
/// 模型，代价真实）。取 10 分钟：既比最慢的可接受往返长，又比「用户已经以为坏了」短。
///
/// `max_attempts = 1`——**`turn` 调用不重试**。理由不是成本，是**没人 await**：
/// 租约到期只发生在进程重启之后（时间只在事件上推进），而等这笔应答的 future
/// 已经随进程一起没了。重试只会留下一条**永不派发**的 `request`，让「欠一次
/// 派发」变成一句谎。到顶即判死（`failed{why:"lease_expired"}`），恢复走
/// resume 幕（新的逻辑 id，旧那笔作为「没回来」留在日志里）。
///
/// `retry` 因此是**有活着的等待方**的端口才用的机制（PL-017 的 `tool`：
/// 它的内环还在，看护者重试后由同一幕接着 await）。
///
/// ## `model` 端口（T-227 步 2）
///
/// 一次**模型往返**。它在 `turn` 执行体**内部**发起（ADR-115 决策 5：两者之间
/// 没有双调度器——`turn` 是普通 `by=Some` 执行体，PL-017 只是把它的内部拆成
/// 更多行）。
///
/// 租约与 `turn` 同量级（慢的那一头同样是模型排队）；`max_attempts = 1` 同理：
/// 进程重启后等这笔应答的 future 已经不在了，重试只会留下一条永不派发的
/// `request`。等步 4 的唤醒事实接上（有活着的等待方）时，这个上限才可以 > 1。
///
/// ## `tool` 端口（T-227 步 3）
///
/// 一次**工具批**：一次模型响应带来的全部调用，作为一个派发单元。
///
/// **调用单元为什么是「批」而不是「单次调用」**：批才是执行体的原子单位——
/// 批首闸门读数一次、交互模式下一条待用户动作就收掉批尾、批尾未执行的调用统一
/// 收口（`tools/dispatch/{gates,batch,unfinished}.rs`）。把债切到单次调用会得到
/// N 笔**无法独立派发**的账（执行体一次只吃一批），那是记账比执行更细的假象。
/// 单次调用的**结果**本来就逐条是事实（`artifact.added`，带 `tool_call_id`），
/// 所以「哪一次调用跑完了」不从协议面上丢。
///
/// 租约与 `turn` 同值：批再长也长不过它所属的那一轮，轮租约先到期就整轮判死——
/// 工具租约设得更大只是让「谁先判死」变得不可预期。`max_attempts = 1` 同
/// `turn`：重跑一次工具批会重放副作用（写文件 / 派活），那不是重试，是第二次执行。
pub(crate) static PORTS: &[PortDecl] = &[
    PortDecl {
        port: "turn",
        // 引擎在跑执行体**之前**落它（判据 2）：搬运者可能不产事实就返回。
        before: EVENT_SENT,
        lease_ms: 600_000,
        max_attempts: 1,
    },
    PortDecl {
        port: "model",
        before: EVENT_SENT,
        lease_ms: 600_000,
        max_attempts: 1,
    },
    PortDecl {
        port: "tool",
        before: EVENT_SENT,
        lease_ms: 600_000,
        max_attempts: 1,
    },
];

/// 内建行表。注册顺序**不参与**裁决——同 `prio` 由 `(name, key)` 决出。
pub(crate) static ROWS: &[Decl] = &[
    // ── 收束的授权级联（ADR-116）──────────────────────────────────────────
    //
    // 两条行，`prio` 0 / 5：**收束一旦该发生，它先于一切**。一轮走到收束点时
    // `execute-turn`（60）仍然欠着（`until` 是 `turn_closed`），若收束行排在它后面，
    // 单环驱动会先再执行一次而永远收不了尾。
    //
    // ## 为什么触发是 `turn.closing` 而不是「模型回来了」
    //
    // ADR-116 初稿把 `authorize-close` 的开账条件写成 `turn.has_final`（视图字段，
    // 从 `response{port:model}` 的载荷折出）。施工时逐点核对六条收束路径，发现
    // **三条根本没有模型事实可折**：首请求前中止（`prepare` 1c）、定稿答话
    // （`prepare` 1b）、反射档（`run_reflex`）。以模型事实为前件，这三条会永不
    // 收束——而它们正是「零模型往返」的三条，不是边角。
    //
    // 所以前件是**决策本身**：收束点先落 `turn.closing`（与 `request` 同一条纪律
    // ——决策者写决策），授权行再据它开账。六条路径同一个前件，没有例外。
    //
    // ## `until` 为什么带 `kind`
    //
    // 闸按种类分两臂（`speech` / `artifact`）。收束只问 `speech`：产物臂的拒绝
    // 落的是 `artifact.added` 那一格，它不结束一轮，不在本行管的范围里。
    Decl {
        name: "authorize-close",
        prio: 0,
        by: Some("authorizer"),
        // 纯矩阵查表（`ActorSpec::can_land_speech` + 作用域上限），不调模型。
        tier: LatencyTier::Fast,
        owes: Pattern {
            source: EVENT_TURN_CLOSING,
            key: &["turn"],
            eq: &[],
        },
        until: &[
            Pattern {
                source: EVENT_AUTHZ_GRANTED,
                key: &["turn"],
                eq: &[("kind", Lit::Str("speech"))],
            },
            Pattern {
                source: EVENT_AUTHZ_DENIED,
                key: &["turn"],
                eq: &[("kind", Lit::Str("speech"))],
            },
        ],
        unless: &[],
    },
    // 收束本身：**只有 `granted` 才开账**（ADR-115 决策 6 的正事实级联）。
    //
    // `denied` ⇒ 本行永不成立 ⇒ 收束格不落——「被拒的一轮没有收束格」从此是
    // **结构**，不是某个 `if` 的结论（T-227 判据 8b）。`owes` 里没有否定原子。
    //
    // 出口是两格收束（`final` / `fallback`）：落哪一种由收束点的 `turn.closing.key`
    // 说了算，本行只管「这一轮有没有收完」。
    Decl {
        name: "close-turn",
        prio: 5,
        by: Some("closer"),
        // 落格是记账：正文与工具事实在收束点之前已经产出，收束本身不再调模型。
        tier: LatencyTier::Fast,
        owes: Pattern {
            source: EVENT_AUTHZ_GRANTED,
            key: &["turn"],
            eq: &[("kind", Lit::Str("speech"))],
        },
        until: &[
            Pattern {
                source: EVENT_ASSISTANT_FINAL,
                key: &["turn"],
                eq: &[],
            },
            Pattern {
                source: EVENT_ASSISTANT_FALLBACK,
                key: &["turn"],
                eq: &[],
            },
        ],
        unless: &[],
    },
    // ── 轮首判决 ─────────────────────────────────────────────────────────
    Decl {
        name: "policy.classify",
        prio: 10,
        by: Some("decide"),
        // 判决可能落到模型，但不生成：快速档（与旧行 `effects.tier` 同值）。
        tier: LatencyTier::Fast,
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
    // ── 中途汇报 ─────────────────────────────────────────────────────────
    //
    // 一段静默 = 一个 `(turn, anchor)`。汇报事实落格后静默锚前移，下一段静默
    // 是另一笔账——配额之内可以汇报多次，逐段关账而不是「一说就全关」。
    Decl {
        name: "policy.report",
        prio: 20,
        by: Some("report"),
        // 模板产线，零模型往返。
        tier: LatencyTier::Reflex,
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
    // ── 自动语义压缩 ─────────────────────────────────────────────────────
    Decl {
        name: "policy.compress",
        prio: 30,
        by: Some("compress"),
        // 语义压缩调模型：深度档。
        tier: LatencyTier::Deep,
        owes: Pattern {
            source: "turn_head",
            key: &["turn", "round"],
            eq: &[
                ("auto_compress", Lit::Bool(true)),
                ("compress_done", Lit::Bool(false)),
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
        by: Some("recall"),
        // 事实投影的纯读 + 技能路由，不调模型。
        tier: LatencyTier::Reflex,
        owes: Pattern {
            source: "turn_head",
            key: &["turn", "round"],
            eq: &[
                ("round", Lit::Uint(0)),
                ("is_persistent", Lit::Bool(true)),
                ("recall_done", Lit::Bool(false)),
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
    Decl {
        name: "policy.reflex",
        prio: 50,
        by: Some("reflex"),
        // 快路就是「不调模型」：反射档是它的定义。
        tier: LatencyTier::Reflex,
        owes: Pattern {
            source: "turn_head",
            key: &["turn"],
            eq: &[
                ("resumed", Lit::Bool(false)),
                ("skill_hit", Lit::Bool(true)),
            ],
        },
        until: &[Pattern {
            source: "turn_closed",
            key: &["turn"],
            eq: &[],
        }],
        unless: &[],
    },
    // ── 执行（决策行）────────────────────────────────────────────────────
    //
    // 只要本幕还开着，就欠一次执行。它的**后果**是写一条 `request`（决策者的
    // 动作），派发本身归 `dispatch-turn`。没把它并进协议行：两者的粒度不同
    // （本幕 vs 一次请求往返）——一幕可以有多次往返（auto 续写），
    // 而「本幕还欠不欠」只有一个答案。
    Decl {
        name: "execute-turn",
        prio: 60,
        by: Some("execute"),
        tier: LatencyTier::Deep,
        owes: Pattern {
            source: "turn_head",
            key: &["turn"],
            eq: &[],
        },
        until: &[Pattern {
            source: "turn_closed",
            key: &["turn"],
            eq: &[],
        }],
        unless: &[],
    },
    // ── 端口 `turn` 的两行主模板（T-223）───────────────────────────
    //
    // 每端口固定两行：`dispatch-<port>` 与 `await-<port>`，差异只有 `by`。
    // 第三行（租约）**每端口不写**——一个 `lease` 行管所有端口，它按
    // `stale_sent` 的实例而不是按端口开账。
    //
    // 键取 **逻辑 id**（`(port, id)`）而不是 `(port, id, attempt)`：`failed`
    // 关的是逻辑 id（所有尝试），带 attempt 的键永远匹配不上它。尝试级口径
    // 归视图（`in_flight` / `stale_sent`），行层是逻辑账。
    Decl {
        name: "dispatch-turn",
        prio: 65,
        by: Some("courier"),
        // 派发本身不调模型（模型在执行体里）。
        tier: LatencyTier::Fast,
        owes: Pattern {
            source: EVENT_REQUEST,
            key: &["port", "id"],
            eq: &[("port", Lit::Str("turn"))],
        },
        until: &[
            Pattern {
                source: EVENT_SENT,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("turn"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("turn"))],
            },
        ],
        unless: &[],
    },
    // ── 端口 `model` 的两行（T-227 步 2）──────────────────────────
    //
    // 与 `turn` 的两行**同模板**，只差端口取值：模型调用是轮内的一次往返，它在
    // `turn` 执行体内部发起（ADR-115 决策 5：没有第二个调度器）。
    //
    // 键仍取逻辑 id：一次模型调用 = 一个 id，重试另计尝试号。
    //
    // ## 端口粒度为什么是「一次模型往返」而不是「一次工具批次」
    //
    // 模型调用是**最内层**的单次对外调用（它下面没有别的调用了），因此它是最小
    // 的可恢复单位：崩溃后重放到「上一次模型请求」即可，不必重放整轮。工具批次
    // 在它**外面**（一个模型响应可带多个工具调用），所以工具端口的 `expand` 是
    // 下一层的事（PL-017 步 3）。
    Decl {
        name: "dispatch-model",
        prio: 66,
        by: Some("courier"),
        // 派发本身不调模型（模型在执行体里）。
        tier: LatencyTier::Fast,
        owes: Pattern {
            source: EVENT_REQUEST,
            key: &["port", "id"],
            eq: &[("port", Lit::Str("model"))],
        },
        until: &[
            Pattern {
                source: EVENT_SENT,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("model"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("model"))],
            },
        ],
        unless: &[],
    },
    // ── 端口 `tool` 的派发行（T-227 步 3）──────────────────────────
    //
    // 与前两个端口**同模板**，只差端口取值。工具批的调用单元是「一次响应带来的
    // 全部调用」（理由见 `PORTS` 的 `tool` 一节）：批内并发与批尾收口都在执行体
    // 内部，协议面只回答「这一批派出去了没有、回来了没有」。
    //
    // `prio` 取 67：在 `dispatch-model` 之后、两个 `await` 之前——同一点上若模型
    // 与工具都有账要还，模型那一笔先（工具批是它的**下游**，没有响应就没有批）。
    // `await-tool` 与其余 `await` 行一起排在 72（见下）。
    Decl {
        name: "dispatch-tool",
        prio: 67,
        by: Some("courier"),
        // 派发本身不调模型（工具在执行体里）。
        tier: LatencyTier::Fast,
        owes: Pattern {
            source: EVENT_REQUEST,
            key: &["port", "id"],
            eq: &[("port", Lit::Str("tool"))],
        },
        until: &[
            Pattern {
                source: EVENT_SENT,
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
    // 「已派出、等应答」——`by: None` 就是 `Wait` 的折叠：对引擎透明（恒 blocked、
    // 永不选中），但它让「在等」与「没事」可区分（PL-016 §2.3）。
    Decl {
        name: "await-turn",
        prio: 70,
        by: None,
        tier: LatencyTier::Deep,
        owes: Pattern {
            source: EVENT_SENT,
            key: &["port", "id"],
            eq: &[("port", Lit::Str("turn"))],
        },
        until: &[
            Pattern {
                source: EVENT_RESPONSE,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("turn"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("turn"))],
            },
            Pattern {
                source: EVENT_RETRY,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("turn"))],
            },
        ],
        unless: &[],
    },
    // 「已派出、等应答」——与 `await-turn` 同形：`by: None` 就是 `Wait` 的折叠。
    Decl {
        name: "await-model",
        prio: 71,
        by: None,
        tier: LatencyTier::Deep,
        owes: Pattern {
            source: EVENT_SENT,
            key: &["port", "id"],
            eq: &[("port", Lit::Str("model"))],
        },
        until: &[
            Pattern {
                source: EVENT_RESPONSE,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("model"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("model"))],
            },
            Pattern {
                source: EVENT_RETRY,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("model"))],
            },
        ],
        unless: &[],
    },
    // 「已派出、等应答」——`by: None` 就是 `Wait` 的折叠（工具端口：等这一批回来）。
    Decl {
        name: "await-tool",
        prio: 72,
        by: None,
        tier: LatencyTier::Deep,
        owes: Pattern {
            source: EVENT_SENT,
            key: &["port", "id"],
            eq: &[("port", Lit::Str("tool"))],
        },
        until: &[
            Pattern {
                source: EVENT_RESPONSE,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("tool"))],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("tool"))],
            },
            Pattern {
                source: EVENT_RETRY,
                key: &["port", "id"],
                eq: &[("port", Lit::Str("tool"))],
            },
        ],
        unless: &[],
    },
    // ── 租约（**每端口不写**，一个行管所有端口）───────────────────
    //
    // 开账条件是 `stale_sent.stale`（视图把 `now` 与 `lease_ms` 折成布尔），
    // 所以时间不进欠账公式。
    //
    // ## 为什么没有 `unless: attempt ≥ max_attempts`
    //
    // PL-016 §2.3 的草稿把「尝试到顶」写成 `unless`——那会让行在到顶时挂起，
    // 而「判死」（落 `failed`）恰恰要在到顶时发生。到顶是**看护者的判断**，不是
    // 行的出口：行只负责「这笔卡住了，有人要管」，重试还是判死归执行体。
    Decl {
        name: "lease",
        prio: 80,
        by: Some("lease_keeper"),
        // 看护本身是记账，不调模型。
        tier: LatencyTier::Fast,
        owes: Pattern {
            source: "stale_sent",
            key: &["port", "id"],
            eq: &[("stale", Lit::Bool(true))],
        },
        until: &[
            Pattern {
                source: EVENT_RESPONSE,
                key: &["port", "id"],
                eq: &[],
            },
            Pattern {
                source: EVENT_FAILED,
                key: &["port", "id"],
                eq: &[],
            },
            Pattern {
                source: EVENT_RETRY,
                key: &["port", "id"],
                eq: &[],
            },
        ],
        unless: &[],
    },
];

/// 冻结的声明表（进程内唯一）。装载失败 ⇒ `None`：所有 `owes` 恒假、环停在原处。
static FROZEN: LazyLock<Option<Arc<Frozen>>> = LazyLock::new(|| {
    let frozen = match builtin() {
        Ok(frozen) => frozen,
        Err(errors) => {
            crate::plugin_error!(
                "session",
                "[Loop] 事实 / 视图声明装载失败，义务环不再前进：{:?}",
                errors
            );
            return None;
        }
    };
    for gap in check_rows(ROWS, &frozen) {
        match gap {
            // 已知缺口：`turn.dispatch` 的消费行在 T-225 读侧。授权族的两个事实
            // （`authz.denied` / `authz.granted`）自 ADR-116 起由收束行消费，不再点名。
            Gap::OrphanFact { fact } => {
                crate::plugin_info!("session", "[Loop] 声明的事实暂无行消费：{}", fact);
            }
            other => crate::plugin_error!("session", "[Loop] 行表装载期缺口：{:?}", other),
        }
    }
    Some(frozen)
});

/// 冻结的声明表（租约看护者自己也要求它——它不重复装载一份）。
pub(crate) fn table() -> Option<Arc<Frozen>> {
    FROZEN.clone()
}

fn frozen() -> Option<Arc<Frozen>> {
    FROZEN.clone()
}

/// 本轮的事实切片：视图只扫**一轮**（声明的成本上界针对轮，不针对整份 WAL）。
///
/// 轮号单调，因此「本轮第一条事件」就是本轮窗口的起点；续写幕复用轮号时，
/// 首幕的事件也在切片里，而视图的 `drive_of` 会按**最新幕锚**收窄。
fn turn_slice(events: &[Event], turn: u64) -> &[Event] {
    match events.iter().position(|e| e.turn == turn) {
        Some(start) => &events[start..],
        None => &[],
    }
}

/// 这一行此刻是否欠着且可还（`owes` 命中、未被 `until` / `unless` 关掉）。
///
/// 返回布尔而不是 [`Debt`](crate::symbio_core::ring::Debt)：调用点只决定「这一步
/// 做不做」。要裁决顺序（同一点多行竞争）时用 `ring::drive`，本函数不代劳。
pub(crate) fn owes(name: &str, events: &[Event], turn: u64, now: i64) -> bool {
    let Some(frozen) = frozen() else {
        return false;
    };
    let debt = owed(ROWS, &frozen, turn_slice(events, turn), now)
        .into_iter()
        .find(|d| d.decl.name == name && !d.blocked);
    if let Some(debt) = &debt {
        trace(&frozen, debt);
    }
    debt.is_some()
}

/// 按声明求一个视图值（本轮切片）。未声明 / 扫描超界 ⇒ `None`。
pub(crate) fn view(name: &str, events: &[Event], turn: u64, now: i64) -> Option<Value> {
    frozen()?.eval(name, turn_slice(events, turn), now)
}

/// **视图口径的「欠」**：视图回一条 `[{..}]` = 欠，`[]` / `Null` / 未声明 = 不欠。
///
/// ## 与 [`owes`] 的分工
///
/// [`owes`] 问的是**行**：它把 `owes` / `until` / `unless` / `blocked` 一起算了，
/// 回答「这一行现在可不可还」。本函数问的是**视图**：视图自己就是判据
/// （`turn_model_due` 已经把「收束 / 在途 / 待办批」三件事折完了），调用点只需要
/// 一个布尔。
///
/// ## 为什么两个口都要有
///
/// 行名与视图名共用一张名字空间，但**语义不同**：拿视图名去问 [`owes`] 会一个
/// 字都匹配不上，静默回 `false`——一个「永远不欠」的驱动器，没有任何报错。
/// 把两种问法分成两个函数，误用会变成编译期的名字错误而不是运行期的静默。
pub(crate) fn due(name: &str, events: &[Event], turn: u64, now: i64) -> bool {
    matches!(view(name, events, turn, now), Some(Value::Array(ref a)) if !a.is_empty())
}

/// 这一行携带的档位。未登记 ⇒ 深度档（fail-safe：不静默提速）。
pub(crate) fn tier(name: &str) -> LatencyTier {
    ROWS.iter()
        .find(|d| d.name == name)
        .map(|d| d.tier)
        .unwrap_or(LatencyTier::Deep)
}

/// 一笔可还的**可观测面**：行 / 债标识 / 档位 / 视图口径。与旧 `Hit::trace` 同一
/// 目的——「某一步怎么发生了」在日志里要能指认到行与那一笔债，而不只是行名本身。
///
/// 视图版本一并记下（`ViewDecl.ver`）：投影读法一变，同一份事实折出的结论就变，
/// 回放时「这一轮用了哪一版」必须在日志里有答案。
fn trace(frozen: &Frozen, debt: &crate::symbio_core::ring::Debt<'_>) {
    let decl = debt.decl;
    let key = debt.key.trace();
    let view = frozen.view(decl.owes.source);
    crate::plugin_debug!(
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
