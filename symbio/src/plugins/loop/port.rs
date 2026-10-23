//! 端口调用的**执行半边**（T-223）：协议事实的唯一写方。
//!
//! ## 它解决什么
//!
//! PL-014 §2.1 的「意图」此前只活在内存里：「我要调模型」「我要调工具」在日志上
//! 没有任何痕迹，于是崩溃后不晓得上次发到哪一次，重试与审计都无从下手。本模块把
//! 一次对外调用落成四条事实：`request` → `sent` → `response` / `failed`，外加
//! 租约侧的一条 `retry`。
//!
//! ## 谁写哪一条（判据 2 / 3）
//!
//! | 事实 | 谁写 | 为什么 |
//! |---|---|---|
//! | `request` | **决策者**（[`announce`]） | 「这一轮欠一次调用」是判断，不是执行 |
//! | `sent` | **引擎**（[`dispatch`]，跑执行体**之前**） | 搬运者的 `run` 可能不产事实就返回；依赖它自觉 ⇒ 派发行永不还清，而**没人报错** |
//! | `response` | **引擎**（执行体返回 `Ok`） | 同上：不让「有没有记账」依赖搬运者的纪律 |
//! | `failed` | **引擎**（执行体返回 `Err`） | 失败也是事实；义务环里没有异常分支（PL-016 §2.6 契约 3） |
//! | `retry` | **租约看护者**（[`reap`]） | 一次尝试作废、换一次——只有看护者知道这笔卡了多久 |
//!
//! 「搬运者只做活、引擎记账」是对 PL-016 §2.3 的一处**收紧**：草稿把 `response`
//! 划给搬运者，理由与 `sent` 完全相同（忘了写就静默挂起）。同一处收紧一次，
//! 不拆成两个写方。
//!
//! ## 派发前先问表
//!
//! [`dispatch`] 收一张「这一行还欠不欠」的断言而不是自己去算：调用点刚写完
//! `request`，只有它手里有新鲜的事实切片。不欠 ⇒ [`DispatchError::NotOwed`]，
//! 而不是让一次「没人承认的调用」静默跑完——那正是这张表存在的理由。
//!
//! ## 幂等键
//!
//! `sent` / `response` / `retry` 的键是 `(port, id, attempt)`，`failed` 的键是
//! `(port, id)`（它关的是**逻辑 id**——所有尝试）。事件 id 按同一形状拼出，
//! 重复派发撞上 `AppendError::Duplicate` 即「这一格已经落过了」，不算失败。
//!
//! ## 租约看护为什么在「开一幕」时跑
//!
//! 时间不推进就没有到期（PL-016 §7.2 #10）——一条 `sent` 之后进程死掉，重启时它
//! 一定已经超时。所以看护点就是**恢复点**：[`reap`] 在
//! [`super::chat_loop`] 的 `begin_drive` 里跑，把上一段生命里没回来的调用重试或
//! 判死，再开新的一幕。它的 `now` 取本幕那条唤醒事实的 `ts`（判据 12），所以看护
//! 排在唤醒格**之后**：停机多久都由那条事实带进日志，不靠判决现读墙钟。
//!
//! **诚实的边界**：这一批的看护只覆盖「重启」这一个时机，不做进程内的定时看护
//! （那需要一个定时器源，而时间只在事件上推进）。

use std::fmt;

use crate::symbio_core::ring::{self, PortDecl};
use crate::symbio_core::{
    AppendError, Entity, Event, Frozen, Store, Verb, EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE,
    EVENT_RETRY,
};

/// 一笔端口调用：端口名 + 逻辑 id + 尝试号。
///
/// 逻辑 id 在一次调用的各次尝试间不变（幂等键按它去重），尝试号随重试 +1。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Call {
    pub(crate) port: &'static str,
    pub(crate) id: String,
    pub(crate) attempt: u64,
}

impl Call {
    pub(crate) fn new(port: &'static str, id: impl Into<String>, attempt: u64) -> Self {
        Self {
            port,
            id: id.into(),
            attempt,
        }
    }

    /// 尝试级事实的事件 id（`(port, id, attempt)` 的形状）。
    fn attempt_key(&self, kind: &str) -> String {
        format!("{kind}-{}-{}-{}", self.port, self.id, self.attempt)
    }

    /// 逻辑级事实的事件 id（`(port, id)` 的形状）。
    fn call_key(&self, kind: &str) -> String {
        format!("{kind}-{}-{}", self.port, self.id)
    }
}

/// 一次派发的**上下文**：这笔账属于哪一轮、溯源到哪一格、行还欠不欠。
///
/// 打包不是为了少写参数，而是因为它们四个一起构成「这一次派发是哪一笔账」；
/// 分开传时调用点很容易把 `turn` 与 `produced_by` 写反（两者都是 `u64`）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Dispatch<'a> {
    pub(crate) call: &'a Call,
    /// 这笔调用属于哪一轮。
    pub(crate) turn: u64,
    /// 溯源：本轮用户格的 `seq`（I2 要求断言类事实带 `produced_by`）。
    pub(crate) produced_by: u64,
    /// 调用点刚问过的结论：`dispatch-<port>` 行还欠不欠。
    pub(crate) owed: bool,
}

/// 派发失败：事实没落成 / 执行体自己失败 / 行不欠 / 没有这个端口的声明。
#[derive(Debug)]
pub(crate) enum DispatchError<E> {
    /// 协议事实落格失败（`request` / `sent` / `failed`）。
    Store(AppendError),
    /// 执行体失败——`failed` 已落，这是它自己的错误。
    Actor(E),
    /// `dispatch-<port>` 行不欠：`request` 没落成，或行表装载失败。
    NotOwed(String),
    /// 没有这个端口的 `PortDecl`（漏声明，不是默认值）。
    NoPort(&'static str),
}

impl<E: fmt::Debug> fmt::Display for DispatchError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(e) => write!(f, "协议事实落格失败：{e:?}"),
            Self::Actor(e) => write!(f, "执行体失败：{e:?}"),
            Self::NotOwed(row) => {
                write!(f, "行 `{row}` 不欠——`request` 未落或行表装载失败")
            }
            Self::NoPort(port) => write!(f, "端口 `{port}` 没有声明（缺少 `PortDecl`）"),
        }
    }
}

/// 决策者写 `request`：这一轮欠一次 `<port>` 调用。
///
/// 幂等：同 `(port, id, attempt)` 重跑 ⇒ `Duplicate`，视为已记录。
pub(crate) fn announce<S>(
    store: &S,
    call: &Call,
    turn: u64,
    produced_by: u64,
) -> Result<(), AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    announce_with(store, call, turn, produced_by, serde_json::Value::Null)
}

/// 与 [`announce`] 同一条 `request`，但允许端口带上**判定所需的载荷**。
///
/// 工具端口的授权级联需要「这次调用请求了什么、风险多大、用户批过没有」——
/// 它们必须和 `request` 同一条事实落盘，否则授权行只能读内存，回放就不可判。
/// `extra` 是 JSON 对象时逐字段并入载荷；`Null` / 非对象 ⇒ 与 `announce` 逐字同形。
///
/// **不把 `extra` 并进 `sent`**：派发事实的载荷是端口声明 + 尝试号，判定输入
/// 属于「请求」这一半；同一条事实写两遍输入会让两个写方各说一套。
pub(crate) fn announce_with<S>(
    store: &S,
    call: &Call,
    turn: u64,
    produced_by: u64,
    extra: serde_json::Value,
) -> Result<(), AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let mut payload = serde_json::json!({
        "port": call.port,
        "id": call.id,
        "attempt": call.attempt,
    });
    if let (Some(dst), Some(src)) = (payload.as_object_mut(), extra.as_object()) {
        for (key, value) in src {
            dst.insert(key.clone(), value.clone());
        }
    }
    append(
        store,
        Event::pending(
            call.attempt_key(EVENT_REQUEST),
            EVENT_REQUEST,
            Entity::Turn,
            Verb::Asserted,
            turn,
            "system",
        )
        .with_produced_by(produced_by)
        .with_payload(payload),
    )
}

/// 引擎派发：落 `before` → 跑执行体 → 落应答或失败。
///
/// `before` 取自 [`PortDecl::before`]（不是写死的 `sent`）：端口的协议事实由声明
/// 决定，加一个端口只加一条 `PortDecl`。
///
/// `owed` 是调用点刚问过的结论（`dispatch-<port>` 行还欠不欠）。不欠 ⇒ 立刻返回
/// [`DispatchError::NotOwed`]，**执行体不会被跑**——没有账的调用不许发出。
pub(crate) async fn dispatch<S, T, E, F, Fut, W>(
    store: &S,
    ports: &[PortDecl],
    plan: Dispatch<'_>,
    run: F,
    why: W,
) -> Result<T, DispatchError<E>>
where
    S: Store<Event = Event> + ?Sized,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    W: FnOnce(&E) -> String,
{
    dispatch_with(store, ports, plan, run, why, |_| serde_json::Value::Null).await
}

/// 与 [`dispatch`] 同一条派发，但允许把**应答载荷**并进 `response`。
///
/// 模型端口的应答要携带本次交回的工具调用（`calls`，ADR-117）：创建行
/// `open-tools` 的集合展开读的就是它。载荷只能由**跑完执行体之后**才知道，
/// 所以以闭包形式传入，而不是让调用点先跑一遍再补写（那会变成第二个写方）。
pub(crate) async fn dispatch_with<S, T, E, F, Fut, W, X>(
    store: &S,
    ports: &[PortDecl],
    plan: Dispatch<'_>,
    run: F,
    why: W,
    extra: X,
) -> Result<T, DispatchError<E>>
where
    S: Store<Event = Event> + ?Sized,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    W: FnOnce(&E) -> String,
    X: FnOnce(&T) -> serde_json::Value,
{
    let Dispatch {
        call,
        turn,
        produced_by,
        ..
    } = plan;
    begin(store, ports, plan).map_err(|e| match e {
        DispatchError::NoPort(port) => DispatchError::NoPort(port),
        DispatchError::NotOwed(row) => DispatchError::NotOwed(row),
        DispatchError::Store(e) => DispatchError::Store(e),
        // `begin` 不跑执行体，构造不出 `Actor`。
        DispatchError::Actor(never) => match never {},
    })?;
    match run().await {
        Ok(value) => {
            succeed_with(store, call, turn, produced_by, extra(&value))
                .map_err(DispatchError::Store)?;
            Ok(value)
        }
        Err(e) => {
            // 失败也要留痕：写不进去不掩盖执行体自己的错误（它更有信息量）。
            let _ = failed(store, call, turn, produced_by, &why(&e));
            Err(DispatchError::Actor(e))
        }
    }
}

/// 派发的**前半段**：确认这一行还欠，然后落 `before`（通常是 `sent`）。
///
/// 与 [`dispatch`] 拆开是为了「先派发 N 笔、再等整批」的形态（PL-017 §3.3 的
/// 工具批次）：每笔子调用各自落一条 `sent`，执行体在整批 `sent` 之后才跑。
/// 记账仍然是引擎的一半——调用点只交出「这一行还欠不欠」的事实结论。
pub(crate) fn begin<S>(
    store: &S,
    ports: &[PortDecl],
    plan: Dispatch<'_>,
) -> Result<(), DispatchError<std::convert::Infallible>>
where
    S: Store<Event = Event> + ?Sized,
{
    let Dispatch {
        call,
        turn,
        produced_by,
        owed,
    } = plan;
    let decl = ring::port_of(ports, call.port).ok_or(DispatchError::NoPort(call.port))?;
    if !owed {
        return Err(DispatchError::NotOwed(format!("dispatch-{}", call.port)));
    }
    append(
        store,
        Event::pending(
            call.attempt_key(decl.before),
            decl.before,
            Entity::Turn,
            Verb::Asserted,
            turn,
            "system",
        )
        // 协议事实是断言类（`turn × asserted`）：I2 要求带溯源。
        // 溯源 = 本轮用户格（“这次调用是为这一轮发的”）。
        .with_produced_by(produced_by)
        .with_payload(serde_json::json!({
            "port": call.port,
            "id": call.id,
            "attempt": call.attempt,
            // 派发那一刻约定的租约：`stale_sent` 视图算「卡了多久」要用它，
            // 而视图读不到代码里的 `PortDecl`（入参只有事实与 `now`）。
            "lease_ms": decl.lease_ms,
            "attempt_cap": decl.max_attempts,
        })),
    )
    .map_err(DispatchError::Store)
}

/// 派发的**后半段**：执行体正常返回 ⇒ 落 `response`（同一次尝试）。
pub(crate) fn succeed<S>(
    store: &S,
    call: &Call,
    turn: u64,
    produced_by: u64,
) -> Result<(), AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    succeed_with(store, call, turn, produced_by, serde_json::Value::Null)
}

/// 与 [`succeed`] 同一条 `response`，但允许并进**应答载荷**（模型端口的 `calls`）。
///
/// 与 [`announce_with`] 对称：输入属于请求那一半，输出属于应答这一半；两半各写
/// 一次，不互相覆盖。
pub(crate) fn succeed_with<S>(
    store: &S,
    call: &Call,
    turn: u64,
    produced_by: u64,
    extra: serde_json::Value,
) -> Result<(), AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let mut payload = serde_json::json!({
        "port": call.port,
        "id": call.id,
        "attempt": call.attempt,
    });
    if let (Some(dst), Some(src)) = (payload.as_object_mut(), extra.as_object()) {
        for (key, value) in src {
            dst.insert(key.clone(), value.clone());
        }
    }
    append(
        store,
        Event::pending(
            call.attempt_key(EVENT_RESPONSE),
            EVENT_RESPONSE,
            Entity::Turn,
            Verb::Asserted,
            turn,
            "system",
        )
        .with_produced_by(produced_by)
        .with_payload(payload),
    )
}

/// 落 `failed`：这笔调用不再重试（关**逻辑 id**，所有尝试）。
pub(crate) fn failed<S>(
    store: &S,
    call: &Call,
    turn: u64,
    produced_by: u64,
    why: &str,
) -> Result<(), AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    append(
        store,
        Event::pending(
            call.call_key(EVENT_FAILED),
            EVENT_FAILED,
            Entity::Turn,
            Verb::Asserted,
            turn,
            "system",
        )
        .with_produced_by(produced_by)
        .with_payload(serde_json::json!({
            "port": call.port,
            "id": call.id,
            "why": why,
        })),
    )
}

/// 租约看护的结果（每笔卡住的调用一个）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reaped {
    /// 这次尝试作废、已发起下一次（`retry` + `request`）。
    Retried { id: String, attempt: u64 },
    /// 尝试到顶，判死（`failed`）。
    Killed { id: String, attempt: u64 },
}

/// **租约看护**：把 `sent` 之后超时无应答的调用重试或判死（判据 7）。
///
/// 输入只有事实切片、端口声明与 `now`——不给内存状态，所以「崩溃后重放」与
/// 「正常跑」走同一条路（判据 4 的另一半：`in_flight` 从日志重建）。
///
/// 到顶判死的判据是 `stale_sent.exhausted`（`attempt >= max_attempts`），
/// **不是**行上的 `unless`：行只负责「这笔卡住了、有人要管」，怎么收归这里。
/// 两种收尾都写一条事实（`retry` / `failed`），所以下一轮问表时那笔账已经关掉。
pub(crate) fn reap<S>(
    store: &S,
    frozen: &Frozen,
    ports: &[PortDecl],
    facts: &[Event],
    turn: u64,
    produced_by: u64,
    now: i64,
) -> Result<Vec<Reaped>, AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let mut out = Vec::new();
    // 只取 `lease` 行开着的那几笔——「卡住」的唯一口径在视图里，不在这里重算。
    for debt in ring::owed(super::loop_rows::ROWS, frozen, facts, now) {
        if debt.decl.name != "lease" {
            continue;
        }
        let Some(port) = key_of(&debt.key, "port") else {
            continue;
        };
        let Some(id) = key_of(&debt.key, "id") else {
            continue;
        };
        let Some(decl) = ring::port_of(ports, &port) else {
            continue;
        };
        // 这一笔目前卡在第几次尝试：取 `stale_sent` 里同 `(port, id)` 的那条实例。
        let Some((attempt, exhausted)) = stale_state(frozen, facts, now, &port, &id) else {
            continue;
        };
        let call = Call::new(decl.port, id.clone(), attempt);
        if exhausted {
            failed(store, &call, turn, produced_by, "lease_expired")?;
            out.push(Reaped::Killed { id, attempt });
            continue;
        }
        // 作废这次尝试，并立刻举起下一次。**这句「就欠了」今天是假的**：三行派发债的
        // 键是 `(port, id)`（不含 `attempt`，见 `loop_rows.rs` 的键口径注释），
        // 上一 attempt 的出口格已经把那一行关掉 ⇒ 这笔 `request{attempt+1}` 在行表里
        // 不欠任何东西，没有任何执行体会派它。键要不要含 attempt 是一条机制裁决，
        // 见 T-252；在它落地之前，真表上 `max_attempts` 全为 1（重试分支不可达，
        // 用例 `no_port_in_the_real_table_retries_today` 钉着）。
        append(
            store,
            Event::pending(
                call.attempt_key(EVENT_RETRY),
                EVENT_RETRY,
                Entity::Turn,
                Verb::Asserted,
                turn,
                "lease_keeper",
            )
            .with_produced_by(produced_by)
            .with_payload(serde_json::json!({
                "port": call.port,
                "id": call.id,
                "attempt": call.attempt,
            })),
        )?;
        let next = Call::new(decl.port, id.clone(), attempt + 1);
        announce(store, &next, turn, produced_by)?;
        out.push(Reaped::Retried { id, attempt });
    }
    Ok(out)
}

/// 一笔债务的键里取一个字段。
fn key_of(key: &ring::Key, field: &str) -> Option<String> {
    key.0
        .iter()
        .find(|(k, _)| k == field)
        .map(|(_, v)| v.clone())
}

/// 同 `(port, id)` 的卡住实例的 `(attempt, exhausted)`。
fn stale_state(
    frozen: &Frozen,
    facts: &[Event],
    now: i64,
    port: &str,
    id: &str,
) -> Option<(u64, bool)> {
    let value = frozen.eval("stale_sent", facts, now)?;
    value.as_array()?.iter().find_map(|v| {
        let same = v.get("port").and_then(|x| x.as_str()) == Some(port)
            && v.get("id").and_then(|x| x.as_str()) == Some(id)
            && v.get("stale").and_then(|x| x.as_bool()) == Some(true);
        if !same {
            return None;
        }
        Some((
            v.get("attempt").and_then(|x| x.as_u64()).unwrap_or(0),
            v.get("exhausted")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
        ))
    })
}

/// 落一条协议事实；`Duplicate` 是「已经落过了」，不是失败。
fn append<S>(store: &S, event: Event) -> Result<(), AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    match store.append(event) {
        Ok(_) => Ok(()),
        Err(AppendError::Duplicate) => Ok(()),
        Err(e) => Err(e),
    }
}

/// 一次**内部请求**的端口往返（端口 `model`）。
///
/// ## 它管的是「编排层替别人打的模型电话」
///
/// 会话轮的那一次模型调用住在 v2 运行器里，由 `dispatch-model` 的执行体派发。
/// 但一轮里还有别的模型往返，它们由编排层经容器 `route` 交给别的插件去跑
/// （当前唯一一条：`Answered{from_context}` 的答话生成）。PL-017 判据 2 的口径是
/// **每一次**模型调用都有 `sent` + 应答，内部请求不是例外——它同样是一次对外调用，
/// 同样要能在崩溃后被租约看护收走。
///
/// ## 调用点为什么能断定「这一笔真会打模型」
///
/// [`crate::symbio_core::Verdict::needs_wording_generation`] 是这条判据的唯一 owner：
/// 措辞插件按它决定是否生成，编排层按它决定要不要记账。两处各判一遍会漂移成
/// 「记了账却没打模型」（三格从此不代表一次模型往返，日志不能再用来数调用）或
/// 「打了模型没账」（本任务要关的那个缺口）。
///
/// ## 失败方向：告警 + 「无措辞」，不重试也不补打
///
/// 派发被拒（`NotOwed`）时执行体**根本没跑**，落格失败时它已经跑完——两种 `Err`
/// 都不该换来第二次模型调用。调用方按「没有措辞」处置（`Answered` 那条已知是
/// 降级进工具循环，不是沉默）。
pub(crate) struct Round<'a> {
    /// 这笔账属于哪一轮。
    pub(crate) turn: u64,
    /// 溯源锚：本轮用户格的 `seq`。
    pub(crate) produced_by: u64,
    /// 本幕锚——逻辑 id 必须带它，否则续写幕会与首幕撞成同一笔账。
    pub(crate) drive_seq: u64,
    /// 本幕的 `now`（唤醒事实的 `ts`，判据 12）：问表要用它。
    pub(crate) now: i64,
    /// 逻辑 id 的步名段：`model-{turn}-d{drive}-{step}`。
    pub(crate) step: &'a str,
    /// 这一笔是谁的往返（并进 `request` / `response` 载荷，让日志说得出用途）。
    pub(crate) via: &'a str,
}

impl Round<'_> {
    fn id(&self) -> String {
        format!("model-{}-d{}-{}", self.turn, self.drive_seq, self.step)
    }
}

/// 记账并跑一次内部请求（见 [`Round`]）。
pub(crate) async fn round_trip<S, T, F, Fut>(
    store: &S,
    round: Round<'_>,
    run: F,
) -> Result<T, DispatchError<std::convert::Infallible>>
where
    S: Store<Event = Event> + ?Sized,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let call = Call::new("model", round.id(), 1);
    let via = serde_json::json!({ "via": round.via });
    announce_with(store, &call, round.turn, round.produced_by, via.clone())
        .map_err(DispatchError::Store)?;
    // 派发前切片必须是**刚写完 `request`** 的那一份：行欠不欠看它。
    let facts = store.range(crate::symbio_core::Seq::new(0));
    let owed = super::loop_rows::owes("dispatch-model", &facts, round.turn, round.now);
    dispatch_with(
        store,
        super::loop_rows::PORTS,
        Dispatch {
            call: &call,
            turn: round.turn,
            produced_by: round.produced_by,
            owed,
        },
        || async move { Ok(run().await) },
        |never| match *never {},
        move |_| via,
    )
    .await
}

#[cfg(test)]
#[path = "port.test.rs"]
mod tests;
