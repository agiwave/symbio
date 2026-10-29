//! 端口调用的**执行半边**：协议事实的唯一写方。
//!
//! ## 它解决什么
//!
//! DE-014 §2.1 的「意图」此前只活在内存里：「我要调模型」「我要调工具」在日志上
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
//! | `failed` | **引擎**（执行体返回 `Err`） | 失败也是事实；义务环里没有异常分支（DE-015 §2.6 契约 3） |
//! | `retry` | **租约看护者**（[`reap`]） | 一次尝试作废、换一次——只有看护者知道这笔卡了多久 |
//!
//! 「搬运者只做活、引擎记账」是对 DE-015 §2.3 的一处**收紧**：草稿把 `response`
//! 划给搬运者，理由与 `sent` 完全相同（忘了写就静默挂起）。同一处收紧一次，
//! 不拆成两个写方。
//!
//! ## 派发前先问表
//!
//! [`dispatch`] 在落 `sent` **之前**自己读一次行表：调用点交出的只有「授权这次派发
//! 的行名」与读账用的 `now`，不交结论。让它交结论等于给每个调用点一次自己算的机会
//! ——切片问旧了、行名写错了，都不会有人发现（D-136）。不欠 ⇒
//! [`DispatchError::NotOwed`]，而不是让一次「没人承认的调用」静默跑完——那正是这张
//! 表存在的理由。
//!
//! ## 幂等键
//!
//! **五条协议事实的键都是 `(port, id, attempt)`**（D-138）：`failed` 也收窄到
//! 这一次尝试，「这笔调用不再重试」由「没有后继 `request`」表达。事件 id 按同一
//! 形状拼出，重复派发撞上 `AppendError::Duplicate` 即「这一格已经落过了」，不算失败。
//!
//! 行层因此能按尝试开账：重试写出的 `request{attempt+1}` 会重新打开
//! `dispatch-<port>` / `await-<port>` / `authorize-tool` / `dispatch-tools`，而不是
//! 被上一 attempt 的出口格提前关掉。
//!
//! ## 租约看护为什么在「开一幕」时跑
//!
//! 时间不推进就没有到期（DE-015 §7.2 #10）——一条 `sent` 之后进程死掉，重启时它
//! 一定已经超时。所以看护点就是**恢复点**：[`reap`] 在
//! [`super::chat_loop`] 的 `begin_drive` 里跑，把上一段生命里没回来的调用重试或
//! 判死，再开新的一幕。它的 `now` 取本幕那条唤醒事实的 `ts`（判据 12），所以看护
//! 排在唤醒格**之后**：停机多久都由那条事实带进日志，不靠判决现读墙钟。
//!
//! **诚实的边界**：这一批的看护只覆盖「重启」这一个时机，不做进程内的定时看护
//! （那需要一个定时器源，而时间只在事件上推进）。

use std::fmt;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use symbio_core::ring::{self, PortDecl};
use symbio_core::{
    AppendError, Entity, Event, Frozen, Store, Verb, EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE,
    EVENT_RETRY, EVENT_SENT,
};

/// 一笔端口调用：端口名 + 逻辑 id + 尝试号。
///
/// 逻辑 id 在一次调用的各次尝试间不变（幂等键按它去重），尝试号随重试 +1。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Call {
    pub(crate) port: &'static str,
    pub(crate) id: String,
    pub(crate) attempt: u64,
    /// 这笔往返的**身份**（`via`）：它是「这一轮自己的」还是一条**内部**往返。
    ///
    /// 缺席 = 这一轮自己的模型往返（D-122 决策 2）。请求 / 已发 / 应答 / 失败
    /// 四格都从**这一处**取它：身份写在四个地方就要写对四次，而漏一次的后果是
    /// 判据侧把内部往返读成「这一轮答完了」（提前收束）或「这一轮没答成」
    /// （永不收束）。
    pub(crate) via: Option<String>,
}

impl Call {
    pub(crate) fn new(port: &'static str, id: impl Into<String>, attempt: u64) -> Self {
        Self {
            port,
            id: id.into(),
            attempt,
            via: None,
        }
    }

    /// 一笔**内部**往返（判决 / 措辞 / 摘要）：带身份。
    pub(crate) fn with_via(
        port: &'static str,
        id: impl Into<String>,
        attempt: u64,
        via: &str,
    ) -> Self {
        Self {
            port,
            id: id.into(),
            attempt,
            via: Some(via.to_string()),
        }
    }

    /// 带上一个**可能有**的身份（租约看护从请求格里取回来的那种）。
    pub(crate) fn with_via_opt(mut self, via: Option<String>) -> Self {
        self.via = via;
        self
    }

    /// 协议事实的事件 id（`(port, id, attempt)` 的形状）。
    fn attempt_key(&self, kind: &str) -> String {
        format!("{kind}-{}-{}-{}", self.port, self.id, self.attempt)
    }
}

/// 一次派发的**上下文**：这笔账属于哪一轮、溯源到哪一格、由哪一行授权。
///
/// 打包不是为了少写参数，而是因为它们一起构成「这一次派发是哪一笔账」；
/// 分开传时调用点很容易把 `turn` 与 `produced_by` 写反（两者都是 `u64`）。
///
/// **不传「还欠不欠」的布尔值**：那等于让每个调用点自己问一次表，问旧了或问错了
/// 都没有人发现。行名是声明，欠不欠由引擎在落派发事实前现读（D-136）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct Dispatch<'a> {
    pub(crate) call: &'a Call,
    /// 这笔调用属于哪一轮。
    pub(crate) turn: u64,
    /// 溯源：本轮用户格的 `seq`（I2 要求断言类事实带 `produced_by`）。
    pub(crate) produced_by: u64,
    /// 授权这次派发的行（`dispatch-<port>` 两行模板之一）。
    ///
    /// 行名与「还欠不欠」必须同源：调用点只声明自己站在哪一行上，结论由引擎读。
    pub(crate) row: &'a str,
    /// 读账的 `now`（与 [`super::loop_rows::owes`] 同形；派发行模式本身不读时间）。
    pub(crate) now: i64,
    /// 这次尝试**有没有活着的等待方**（D-138）。
    ///
    /// 只有调用点知道答案：普通工具批次把结果等在当前草稿里，进程一死等待方就没了；
    /// 唤醒调用能由 `wake_batch` 从事实重建，所以它算「有人等」。租约看护只对
    /// `true` 的尝试重试——给没人等的调用写重试，只会造出一条永不派发的 `request`。
    pub(crate) awaited: bool,
}

impl Dispatch<'_> {
    /// 这一行此刻还欠不欠——**唯一的现读点**。
    fn row_owed<S>(&self, store: &S) -> bool
    where
        S: Store<Event = Event> + ?Sized,
    {
        let facts = store.range(symbio_core::Seq::new(0));
        super::loop_rows::owes(self.row, &facts, self.turn, self.now)
    }
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
///
/// 身份（`via`）由 [`Call`] 自带，四格同源——见 [`Call::via`]。
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
    with_via(&mut payload, call.via.as_deref());
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

/// 引擎派发：读行 → 落 `before` → 跑执行体 → 落应答或失败。
///
/// `before` 取自 [`PortDecl::before`]（不是写死的 `sent`）：端口的协议事实由声明
/// 决定，加一个端口只加一条 `PortDecl`。
///
/// 行不欠 ⇒ 立刻返回 [`DispatchError::NotOwed`]，**执行体不会被跑**——没有账的
/// 调用不许发出。这个结论由 [`Dispatch::row_owed`] 现读，不由调用方传入。
///
/// ## 应答载荷由闭包补（`extra`）
///
/// 模型端口的应答要携带本次交回的工具调用（`calls`，D-117）：创建行
/// `open-tools` 的集合展开读的就是它。载荷只能由**跑完执行体之后**才知道，
/// 所以以闭包形式传入，而不是让调用点先跑一遍再补写（那会变成第二个写方）。
///
/// 身份（`via`）不在这条参数里：它由 [`Call`] 自带，请求 / 已发 / 应答 / 失败
/// 四格同源（D-122 决策 2）。
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
    dispatch_full(store, ports, plan, run, why, extra, |_| {
        serde_json::Value::Null
    })
    .await
}

/// 与 [`dispatch_with`] 同一条派发，但**失败载荷**也由调用点补字段。
///
/// 应答载荷与失败载荷是两条事实的两个写方，但**同一批内容**：模型端口的半截
/// 正文 / 思考必须随失败一起入格（否则中止之后那段话只活在内存里），而它们只能
/// 在**执行体返回之后**从流式桥取。`fail_extra` 就是那半个闭包；不关心失败载荷的
/// 端口走 [`dispatch_with`] 传 `|_| Null`。
pub(crate) async fn dispatch_full<S, T, E, F, Fut, W, X, Y>(
    store: &S,
    ports: &[PortDecl],
    plan: Dispatch<'_>,
    run: F,
    why: W,
    extra: X,
    fail_extra: Y,
) -> Result<T, DispatchError<E>>
where
    S: Store<Event = Event> + ?Sized,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    W: FnOnce(&E) -> String,
    X: FnOnce(&T) -> serde_json::Value,
    Y: FnOnce(&E) -> serde_json::Value,
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
            let _ = failed_with_extra(store, call, turn, produced_by, &why(&e), fail_extra(&e));
            Err(DispatchError::Actor(e))
        }
    }
}

/// 派发的**前半段**：确认这一行还欠，然后落 `before`（通常是 `sent`）。
///
/// 与 [`dispatch`] 拆开是为了「先派发 N 笔、再等整批」的形态（DE-015 §3.3 的
/// 工具批次）：每笔子调用各自落一条 `sent`，执行体在整批 `sent` 之后才跑。
/// 记账仍然是引擎的一半——调用点只声明「这笔调用站的是哪一行」。
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
        row,
        ..
    } = plan;
    let decl = ring::port_of(ports, call.port).ok_or(DispatchError::NoPort(call.port))?;
    if !plan.row_owed(store) {
        return Err(DispatchError::NotOwed(row.to_string()));
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
        .with_payload({
            let mut payload = serde_json::json!({
                "port": call.port,
                "id": call.id,
                "attempt": call.attempt,
                // 派发那一刻约定的租约：`stale_sent` 视图算「卡了多久」要用它，
                // 而视图读不到代码里的 `PortDecl`（入参只有事实与 `now`）。
                "lease_ms": decl.lease_ms,
                "attempt_cap": decl.max_attempts,
                // 这一次尝试有没有等待方：租约看护据此决定「重试」还是「判死」。
                "awaited": plan.awaited,
            });
            with_via(&mut payload, call.via.as_deref());
            payload
        }),
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
    with_via(&mut payload, call.via.as_deref());
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

/// 落 `failed`：**这一次尝试**失败（D-138）。
///
/// 「这笔调用不再重试」不再由键的粒度表达：没有后继 `request`，就没有下一次尝试。
/// 逻辑级读者（`open-tools` / `deny-tools` / `turn_head`）仍按 `(port, id)` 读它，
/// 因为模式只取自己声明的键字段。
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
    failed_with(store, call, turn, produced_by, why)
}

/// 与 [`failed`] 同一条 `failed`，身份（`via`）由 [`Call`] 自带。
///
/// `via` 是「这笔往返是谁的」（D-122 决策 2）：失败也要说得出它属于**这一轮
/// 自己**的模型往返，还是一条内部往返（判决 / 措辞 / 摘要）。少了它，判据侧读到
/// 一条内部往返的失败会以为「这一轮模型没答成」⇒ 提前收束。
pub(crate) fn failed_with<S>(
    store: &S,
    call: &Call,
    turn: u64,
    produced_by: u64,
    why: &str,
) -> Result<(), AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    failed_with_extra(store, call, turn, produced_by, why, serde_json::Value::Null)
}

/// 把一笔**没有等待方**的请求判死（`failed`，`why` 由调用方给）。
///
/// ## 什么时候会有这种请求
///
/// 上一段生命在「宣布」与「派出」之间被杀：两步之间还没有 `sent`，租约看护看不见
/// 它（`stale_sent` 只看派出之后卡住的那些），于是它作为一笔开着的债留在日志里，
/// 新的一幕照旧会把它选出来。等它的 future 已经没了，重试也没有对象——判死是唯一
/// 如实的处置（与 [`reap`] 对 `awaited = false` 的口径同源：没人等的调用不留着占账）。
///
/// ## 为什么按端口取最老的一笔
///
/// 与 `ring::owed` 的遍历序一致：判死之后下一问问表就轮到下一笔。**不按幕锚收窄**
/// ——遗留请求恰恰落在本幕锚之前，按锚收窄会让它既看不见又关不掉，派发行于是永远
/// 欠着同一笔（D-166 给 `model` 端口立下的那条处置，本条是它的端口无关版本）。
pub(crate) fn kill_dangling<S>(
    store: &S,
    port: &'static str,
    why: &str,
) -> Result<bool, AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let facts = store.range(symbio_core::Seq::new(0));
    // 不按 `via` 收窄：能力行与内部往返的请求同样「宣布之后没有等待方」时，处置
    // 逐字相同——它们等的是那一刻的栈帧，栈帧没了就没有可重试的对象。按 `via`
    // 收窄会让这类请求既不进 `reap`（还没有 `sent`）也不被本函数关掉，`dispatch-model`
    // 于是永远欠着同一笔。
    let open = facts.iter().find(|r| {
        r.kind == EVENT_REQUEST
            && r.payload.get("port").and_then(|v| v.as_str()) == Some(port)
            && !facts.iter().any(|e| {
                matches!(e.kind.as_str(), EVENT_SENT | EVENT_FAILED)
                    && e.payload.get("port").and_then(|v| v.as_str()) == Some(port)
                    && e.payload.get("id") == r.payload.get("id")
                    && e.payload.get("attempt") == r.payload.get("attempt")
            })
    });
    let Some(request) = open else {
        return Ok(false);
    };
    let Some(id) = request.payload.get("id").and_then(|v| v.as_str()) else {
        return Ok(false);
    };
    let attempt = request
        .payload
        .get("attempt")
        .and_then(|v| v.as_u64())
        .unwrap_or(1);
    let call = Call::new(port, id, attempt);
    failed_with(
        store,
        &call,
        request.turn,
        request.produced_by.unwrap_or(request.turn),
        why,
    )?;
    symbio_core::plugin_warn!(
        "session",
        "[Loop] `{port}` 请求 `{}` 没有等待方（上一段生命在宣布与派出之间结束）——判死",
        call.id
    );
    Ok(true)
}

/// 把一笔**已经派出、没有等待方**的调用判死（`failed`，`why` 由调用方给）。
///
/// 与 [`kill_dangling`] 是同一条处置的两半：那一半管「宣布了还没派出」（日志上只有
/// `request`），这一半管「派出去了、等它的那一段栈帧随进程没了」（日志上有 `sent`、
/// 没有终局）。第二种在 `model` 端口由 `stream::kill_dangling_round` 逐笔处理，工具端口
/// 是一批 N 笔，所以这里按端口把**所有**开着的 `sent` 一次关掉。
///
/// 不重试、不重放：等待方是那一刻的栈帧，它已经不存在了。租约看护（[`reap`]）对同一
/// 形态的处理是「等到期再判死」——本函数给的是**同一个结论的更早时点**：派发器手里
/// 已经没有这一批了，再等下去只是空转。
pub(crate) fn kill_dangling_sent<S>(
    store: &S,
    port: &'static str,
    why: &str,
) -> Result<usize, AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let facts = store.range(symbio_core::Seq::new(0));
    let mut killed = 0usize;
    for sent in facts.iter().filter(|e| {
        e.kind == EVENT_SENT && e.payload.get("port").and_then(|v| v.as_str()) == Some(port)
    }) {
        let closed = facts.iter().any(|e| {
            matches!(e.kind.as_str(), EVENT_RESPONSE | EVENT_FAILED | EVENT_RETRY)
                && e.payload.get("port").and_then(|v| v.as_str()) == Some(port)
                && e.payload.get("id") == sent.payload.get("id")
                && e.payload.get("attempt") == sent.payload.get("attempt")
        });
        if closed {
            continue;
        }
        let Some(id) = sent.payload.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let attempt = sent
            .payload
            .get("attempt")
            .and_then(|v| v.as_u64())
            .unwrap_or(1);
        let call = Call::new(port, id, attempt);
        failed_with(
            store,
            &call,
            sent.turn,
            sent.produced_by.unwrap_or(sent.turn),
            why,
        )?;
        killed += 1;
        symbio_core::plugin_warn!(
            "session",
            "[Loop] `{port}` 调用 `{}` 已派出但没有等待方（草稿里没有这一批）——判死",
            call.id
        );
    }
    Ok(killed)
}

/// 与 [`failed_with`] 同一条 `failed`，但允许并进**失败载荷**（模型端口已产出的
/// 半截正文 / 思考，见 [`dispatch_full`]）。
pub(crate) fn failed_with_extra<S>(
    store: &S,
    call: &Call,
    turn: u64,
    produced_by: u64,
    why: &str,
    extra: serde_json::Value,
) -> Result<(), AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let mut payload = serde_json::json!({
        "port": call.port,
        "id": call.id,
        "attempt": call.attempt,
        "why": why,
    });
    if let (Some(dst), Some(src)) = (payload.as_object_mut(), extra.as_object()) {
        for (key, value) in src {
            dst.insert(key.clone(), value.clone());
        }
    }
    with_via(&mut payload, call.via.as_deref());
    append(
        store,
        Event::pending(
            call.attempt_key(EVENT_FAILED),
            EVENT_FAILED,
            Entity::Turn,
            Verb::Asserted,
            turn,
            "system",
        )
        .with_produced_by(produced_by)
        .with_payload(payload),
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
/// 重试的两个前提（D-138）：
///
/// 1. 这次尝试**有活着的等待方**（`sent.awaited`）——否则重试只会造出一条永远
///    没人派发的 `request`；
/// 2. 还没到 `max_attempts`（`stale_sent.exhausted` 为假）。
///
/// 两个前提缺一就判死（落 `failed{why:"lease_expired"}`），与旧行为逐字相同：
/// 普通批次调用 `awaited = false`、真表里没有等待方的端口 `max_attempts = 1`，
/// 都走这条平凡路径。
///
/// 输入只有事实切片、端口声明与 `now`——不给内存状态，所以「崩溃后重放」与
/// 「正常跑」走同一条路（判据 4 的另一半：`in_flight` 从日志重建）。
///
/// 到顶判死的判据是 `stale_sent.exhausted`（`attempt >= max_attempts`）。
/// 两种收尾都写一条事实（`retry` / `failed`），所以下一次求值时这笔在途已经关掉。
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
    // 「卡住」的唯一口径在视图里（`stale_sent`），不在这里重算——**也不经过行表**。
    //
    // 看护不是还债：它问的是「哪些已派出的尝试过期了」，那是 `sent` 与 `lease_ms`
    // 的一份事实投影；「现在该还哪一笔」才是选债，而那个问题只有环能回答
    // （`TC-1b`：选债口唯一）。所以 `ROWS` 里没有 `lease` 这一行——它此前是
    // `by: None` 的未接线行，唯一的派发点就是本函数对 `ring::owed` 的那次直问。
    let Some(value) = frozen.eval("stale_sent", facts, now) else {
        return Ok(out);
    };
    for item in value.as_array().map(Vec::as_slice).unwrap_or(&[]) {
        if item.get("stale").and_then(|v| v.as_bool()) != Some(true) {
            continue;
        }
        let Some(port) = item.get("port").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(id) = item.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(attempt) = item.get("attempt").and_then(|v| v.as_u64()) else {
            continue;
        };
        let id = id.to_string();
        let Some(decl) = ring::port_of(ports, port) else {
            continue;
        };
        // 这一笔目前卡在第几次尝试：`stale_sent` 已经把「到顶没有」折成 `exhausted`。
        let exhausted = item
            .get("exhausted")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        // 身份写在它自己的 `request` 上；重试写出的下一笔必须**逐字段继承**它，
        // 否则唤醒调用的 `resume` / `tool` / `args` 会在重试链上丢掉。
        let request = request_of(facts, port, &id, attempt);
        let via = request
            .and_then(|e| e.payload.get("via").and_then(|v| v.as_str()))
            .map(str::to_string);
        let call = Call::new(decl.port, id.clone(), attempt).with_via_opt(via);
        if exhausted || !sent_awaited(facts, port, &id, attempt) {
            failed_with(store, &call, turn, produced_by, "lease_expired")?;
            out.push(Reaped::Killed { id, attempt });
            continue;
        }
        // 作废这次尝试，并立刻举起下一次。**先写下一笔 `request`、再写 `retry`**：
        // 两步之间崩溃时，下一次 `reap` 会看到 `request{attempt+1}` 已存在，只补
        // `retry{attempt}`——旧尝试因此不会被重复作废，新尝试也不会丢。
        let next = Call::new(decl.port, id.clone(), attempt + 1).with_via_opt(call.via.clone());
        if request_of(facts, port, &id, attempt + 1).is_none() {
            let mut payload = request
                .map(|e| e.payload.clone())
                .unwrap_or_else(|| serde_json::json!({ "port": port, "id": id }));
            payload["port"] = serde_json::json!(port);
            payload["id"] = serde_json::json!(id);
            payload["attempt"] = serde_json::json!(attempt + 1);
            append(
                store,
                Event::pending(
                    next.attempt_key(EVENT_REQUEST),
                    EVENT_REQUEST,
                    Entity::Turn,
                    Verb::Asserted,
                    turn,
                    "system",
                )
                .with_produced_by(produced_by)
                .with_payload(payload),
            )?;
        }
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
            .with_payload({
                let mut payload = serde_json::json!({
                    "port": call.port,
                    "id": call.id,
                    "attempt": call.attempt,
                });
                with_via(&mut payload, call.via.as_deref());
                payload
            }),
        )?;
        out.push(Reaped::Retried { id, attempt });
    }
    Ok(out)
}

/// 同 `(port, id, attempt)` 的 `request` 事实。
fn request_of<'a>(facts: &'a [Event], port: &str, id: &str, attempt: u64) -> Option<&'a Event> {
    facts.iter().find(|e| {
        e.kind == EVENT_REQUEST
            && e.payload.get("port").and_then(|v| v.as_str()) == Some(port)
            && e.payload.get("id").and_then(|v| v.as_str()) == Some(id)
            && e.payload.get("attempt").and_then(|v| v.as_u64()) == Some(attempt)
    })
}

/// 这次尝试有没有活着的等待方。旧日志没有这个字段 ⇒ `false`（按旧行为判死）。
fn sent_awaited(facts: &[Event], port: &str, id: &str, attempt: u64) -> bool {
    facts.iter().any(|e| {
        e.kind == EVENT_SENT
            && e.payload.get("port").and_then(|v| v.as_str()) == Some(port)
            && e.payload.get("id").and_then(|v| v.as_str()) == Some(id)
            && e.payload.get("attempt").and_then(|v| v.as_u64()) == Some(attempt)
            && e.payload.get("awaited").and_then(|v| v.as_bool()) == Some(true)
    })
}

/// 把一笔账的身份（`via`）并进载荷。
///
/// **缺席即不带键**：读侧按「键在不在」判「这一轮自己的 / 内部往返」，写成
/// `null` 会把两者的区别落在读侧的容错上——读侧一改，两类往返就分不开了。
fn with_via(payload: &mut serde_json::Value, via: Option<&str>) {
    if let (Some(dst), Some(via)) = (payload.as_object_mut(), via) {
        dst.insert(
            "via".to_string(),
            serde_json::Value::String(via.to_string()),
        );
    }
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
/// （当前唯一一条：`Answered{from_context}` 的答话生成）。DE-015 判据 2 的口径是
/// **每一次**模型调用都有 `sent` + 应答，内部请求不是例外——它同样是一次对外调用，
/// 同样要能在崩溃后被租约看护收走。
///
/// ## 调用点为什么能断定「这一笔真会打模型」
///
/// [`symbio_core::Verdict::needs_wording_generation`] 是这条判据的唯一 owner：
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
///
/// ## 失败也入账
///
/// `run` 返回 `Err` ⇒ 落一条带 `via` 的 `failed`（D-122 决策 2）。身份随失败一起
/// 走，是判据侧把「内部往返失败」与「这一轮自己的模型往返失败」分开的唯一依据：
/// 后者会让整轮收束，前者不退化成收束（判决没打出来 ⇒ 进工具循环）。
///
/// ## 应答格带 `calls: []`
///
/// 内部往返不带工具（D-122 决策 5）：应答格里 `calls` 是**空数组**，不是缺席。
/// 「缺席即空集合」是展开侧的读法，但写侧留一个空数组，判据侧不必猜「这个字段
/// 是不是漏了」。
///
/// ## 它不是第五个选债口（`TC-1b` 的第二处反例，结论：不成立）
///
/// 这里自己落 `request`、自己 `dispatch`，看上去像一段一次性的环。三条都不成立：
///
/// 1. **它没问「现在该还哪一笔」**。身份由调用方给的 [`Round`] 定死（逻辑 id、
///    端口、尝试号全在入参里），集合里恰好只有一个元素，「选」这个动作没有对象。
///    `TC-1b` 禁的是**在派发器之外直问求解器再自行派发**——本函数走的是
///    `dispatch_with`，与 `relay`（`call_model`）/ `courier`（`run_dispatch`）
///    同一个门，行还欠不欠仍由派发器现读。
/// 2. **`announce` 与 `dispatch` 之间的那一段没人能执行**。付款方是 `run` 这个
///    Rust 闭包，没有行能选中它；`awaited = false` 且 `max_attempts = 1`，
///    future 随栈一起没，重试没有对象。这正是 `D-222` §一 判 `turn` 不是往返的
///    同一条理由——**它是重入，不是往返**：`announce` 在这里的作用只是把「一次模型
///    调用 = 三格」这个等式补全（DE-015 §6 判据 2），不是「开一笔债等别人来还」。
/// 3. **`dispatch-model` 的前件按端口判**，所以内部往返与轮内那次站同一行。这不是
///    「那一行被它还了」（那一行的 `by` 是 `relay`），而是「派发器的前件它满足」：
///    `owes` 的 pattern 是 `request(port=model)`，任何一笔开着的模型请求都满足它。
///
/// ## 它不会消失，也不该消失（`D-235`）
///
/// 判决 / 措辞 / 压缩摘要三处是**那一行的载荷加工**：派出与收应答在同一个动作窗口里
/// （环在那一段 `.await` 里选不了任何行），所以它们是**一步**而不是一笔债。拆成
/// `announce` / `dispatch` / `await` 三行，前件与后件都由 `prio` 串死、中间无事可选
/// ——没有选择就没有债，得到的只是「同一个动作切三笔、每笔各要一条新视图说轮到我了」。
/// 调用点是**闭集**（裁决点名三处，`CP-002` 从裁决原文读），不是待归零的清单；
/// 删掉它只会得到「调了模型没有账」——比多一处记账口更坏。
pub(crate) async fn round_trip<S, T, E, F, Fut>(
    store: &S,
    round: Round<'_>,
    run: F,
) -> Result<T, DispatchError<E>>
where
    S: Store<Event = Event> + ?Sized,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    E: fmt::Debug,
{
    let call = Call::with_via("model", round.id(), 1, round.via);
    announce_with(
        store,
        &call,
        round.turn,
        round.produced_by,
        serde_json::Value::Null,
    )
    .map_err(DispatchError::Store)?;
    // 派发前切片必须是**刚写完 `request`** 的那一份：行欠不欠看它。结论由引擎现读
    // （`Dispatch::row_owed`），调用点只声明站在 `dispatch-model` 这一行上。
    dispatch_with(
        store,
        super::loop_rows::PORTS,
        Dispatch {
            call: &call,
            turn: round.turn,
            produced_by: round.produced_by,
            row: "dispatch-model",
            now: round.now,
            // 内部往返等在当前 future 里：进程一死等待方就没了，不能重试。
            awaited: false,
        },
        || async move { run().await },
        |e| format!("{e:?}"),
        // 应答格带 `calls: []`（内部往返不带工具，D-122 决策 5）；身份（`via`）
        // 由 [`Call`] 自带，四格同源。
        |_| serde_json::json!({ "calls": [] }),
    )
    .await
}

/// 压缩摘要的**入账面**：一次压缩产线里的模型往返（D-122 决策 2）。
///
/// ## 为什么不是 [`Round`] 一笔
///
/// 压缩可能**打不止一次**模型：快照校验不过时有一次纠正重试。两次往返是**两笔账**
/// （两个逻辑 id、两组三格）——并成一笔会让「三格 = 一次模型往返」这个等式失真，
/// 判据侧数不出真实调用次数。
///
/// ## 计数为什么由闭包数
///
/// 「派发被拒（执行体压根没跑）= 0」与「跑完但落格失败 = 1」分得开的唯一办法是
/// 在**跑之前**加一——与 `classify` 的口径同源。
pub(crate) struct Compress {
    store: Arc<dyn Store<Event = Event>>,
    turn: u64,
    produced_by: u64,
    drive_seq: u64,
    now: i64,
    /// 本请求轮：压缩按轮号计「本轮一次」（`turn.request_round`）。
    round: usize,
    /// 第几次摘要往返（纠正重试是第 1 次）——它进逻辑 id，两笔账因此不撞。
    attempt: AtomicU32,
    /// 实际打了几次模型。
    calls: AtomicU64,
}

impl Compress {
    /// 造一个入版面。
    ///
    /// `drive_seq` 是本幕的锚：逻辑 id 必须带它，否则续写幕会与首幕撞成同一笔账
    /// （与 [`Round`] 同一条理由）。
    pub(crate) fn new(
        store: Arc<dyn Store<Event = Event>>,
        turn: u64,
        produced_by: u64,
        drive_seq: u64,
        now: i64,
        round: usize,
    ) -> Self {
        Self {
            store,
            turn,
            produced_by,
            drive_seq,
            now,
            round,
            attempt: AtomicU32::new(0),
            calls: AtomicU64::new(0),
        }
    }

    /// 一次摘要往返：落三格（`via = "compress/summary"`），跑 `run`。
    ///
    /// 失败也入账（`failed` 格带 `via`）：压缩失败不收束这一轮（压缩是优化项），
    /// 但那次往返**发生过**，账上必须有它——D-122 决策 5 的「退化应答照样有应答格」
    /// 在这里落成「失败也有失败格」。
    pub(crate) async fn summary<T, E, F, Fut>(&self, run: F) -> Result<T, DispatchError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        E: fmt::Debug,
    {
        let attempt = self.attempt.fetch_add(1, Ordering::SeqCst);
        let step = format!("z{}-{attempt}", self.round);
        let round = Round {
            turn: self.turn,
            produced_by: self.produced_by,
            drive_seq: self.drive_seq,
            now: self.now,
            step: &step,
            via: "compress/summary",
        };
        let calls = &self.calls;
        round_trip(self.store.as_ref(), round, || async move {
            // 计数在**跑之前**加一：派发被拒时闭包压根没跑 ⇒ 0。
            calls.fetch_add(1, Ordering::SeqCst);
            run().await
        })
        .await
    }

    /// 这一轮压缩打了几次模型（落进 `turn.stage{compress}.model_calls`）。
    pub(crate) fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }
}

/// 判决往返的派发点（D-122 决策 2 / 4）——住在端口层，因为这一段的**全部**
/// 内容就是「一次模型往返怎么入账」。
pub(crate) mod classify;

#[cfg(test)]
#[path = "port.test.rs"]
mod tests;
