//! 收束的**授权级联**（ADR-116）：`turn.closing` → `authz.granted` / `authz.denied` → 收束格。
//!
//! ## 它解决什么
//!
//! 写侧闸此前是一个**函数**（`closure_gate`）：判完就返回，拒绝时顺手落一条
//! `authz.denied`。判定本身没错，缺的是**另一半**——「允许」只能靠「没有 `denied`」
//! 推断。而否定推断正是义务环禁掉的那件事（ADR-115 决策 6：`owes` 里不出现否定
//! 原子），所以 `close-turn` 行的 `owes` 写不出来，「这一轮为什么能收束」也答不出来。
//!
//! 级联把三件事分开：
//!
//! | 谁 | 落什么 | 为什么 |
//! |---|---|---|
//! | 收束点（执行体） | `turn.closing{key, kind}` | 「该收束了」是**判断**（与 `request` 同一条纪律） |
//! | 授权行 `authorize-close` | `authz.granted` / `authz.denied` | 闸的**结论**是事实，不是返回值 |
//! | 收束行 `close-turn` | 收束格 | 只有 `granted` 才开账 |
//!
//! ## 为什么触发不是「模型回来了」
//!
//! ADR-116 初稿用 `turn.has_final`（从 `response{port:model}` 折出）作前件。施工时
//! 逐点核对六条收束路径，三条**没有模型事实可折**：首请求前中止、定稿答话、反射档
//! ——它们正是零模型往返的那三条。以模型事实为前件，它们永不收束。详见
//! [`super::loop_rows`] 的 `authorize-close` 一节。
//!
//! ## 落不进去怎么办
//!
//! 三处记账失败都**不放行**（fail-closed），理由与 T-219 边界 3 同源：
//!
//! - `turn.closing` 落不进去 ⇒ 授权行不开账 ⇒ 收束格不落；
//! - `authz.granted` 落不进去 ⇒ `close-turn` 不欠 ⇒ 收束格不落；
//! - `authz.denied` 落不进去 ⇒ 结论仍是拒（闸的结论是「不许写」，不能因为记账
//!   失败就放行）。
//!
//! 三者都只 warn 不上抛：收束点此刻要做的是**收尾**，把它变成 `Err` 会让「记账
//! 失败」吃掉「这一轮怎么结束」——而后者才是调用方要的答案。该轮留在未收束态，
//! `check_all` 的 C4 报得出。

use crate::symbio_core::{
    ActorSpec, AppendError, Entity, Event, Seq, Store, Verb, EVENT_AUTHZ_DENIED,
    EVENT_AUTHZ_GRANTED, EVENT_TURN_CLOSING,
};

/// 收束格的**种类**（[ADR-065] 裁决、[ADR-067] 落地形态）：同一道写侧闸按种类分派。
///
/// 一轮的收束有两条路，谓词各问自己那一项能力：
///
/// | 种类 | 落什么格 | 谓词 |
/// |---|---|---|
/// | [`ClosureKind::Speech`] | `chat.assistant.final` / `.fallback` | `can_reply`（`reply.first`） |
/// | [`ClosureKind::Artifact`] | `artifact.added` | `can_produce_artifact`（`produce.artifact`） |
///
/// 为什么必须分种类：对等主体（`agent:peer-b`，授权表只给它 `produce.artifact`）
/// 在发言闸下**恒拒**——合并前它连一轮都收束不了，「对等 offer / assert」的写方
/// （住轮末收束族）因此永远执行不到。分种类后它能收束为**一件产物**，而它的发言
/// 资格仍然一项都没有（「没有发言权」与「留不下任何事实」是两件事）。
///
/// [ADR-065]: ../../../../docs/010-adr/065.md "ADR-065"
/// [ADR-067]: ../../../../docs/010-adr/067.md "ADR-067"
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClosureKind {
    /// 发言收束：答话 / 兜底 / 中止的那一格。
    Speech,
    /// 产物收束：工具结果的那一格（`artifact.added`）。
    Artifact,
}

impl ClosureKind {
    /// 种类名——`authz.*` 载荷的 `kind` 与幂等键的尾段都用它。
    ///
    /// 一个名字一个来源：载荷里写一套、幂等键里写另一套，读侧按 `kind` 分组时
    /// 会漏掉一半（两套都是合法拼写，谁也不报错）。
    pub(crate) fn name(self) -> &'static str {
        match self {
            ClosureKind::Speech => "speech",
            ClosureKind::Artifact => "artifact",
        }
    }

    /// 这一类格要的**能力名**（拒绝原因里说的就是它）。
    ///
    /// 拼写与 `Capability::Reply` / `Capability::ProduceArtifact` 的 `.name()` 同值，
    /// 但本模块看不见 `governance`（core 的私有子模块）——所以这里写一遍，
    /// 由 `denial_names_the_capability_the_gate_judges` 用 `ActorSpec::holds`
    /// **对拍**：拼错一个字符，那条用例当场红。手写一份没有对拍，就只是又一处
    /// 「表授予 A、闸门判 B」。
    pub(crate) fn capability_name(self) -> &'static str {
        match self {
            ClosureKind::Speech => "reply",
            ClosureKind::Artifact => "produce.artifact",
        }
    }
}

/// 收束的**结论**：能落格，或者不能（且带出为什么）。
///
/// 三值而不是布尔：`Denied` 的两个来源（闸拒 / 行不欠）在日志上都是「没有收束格」，
/// 但排障时要能分开——所以 `Denied` 带一句人话，且**拒因已经在 `authz.denied`
/// 里有一份可判的记录**（`reason: denied`），这句话只给人读。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// 可以落格：`turn.closing` 与 `authz.granted` 都已入格，`close-turn` 因此可还。
    Granted,
    /// 不许落格：`authz.denied` 已入格（或记账失败导致行不开账）。`why` 只给人读。
    Denied(String),
}

/// 收束的**唯一授权口**：六个收束点都问它「这一格能不能落」。
///
/// 三步对应三行 / 三格：
///
/// 1. 落 `turn.closing`（决策）→ `authorize-close` 因此开账；
/// 2. 若 `authorize-close` 还欠，跑闸并落 `authz.granted` 或 `authz.denied`
///    （**结论**）——已经欠不着就不重跑：闸是纯函数，重跑只会再落一条同样的格；
/// 3. 问 `close-turn` 欠不欠——`granted` 在 ⇒ 可落；`denied` 在 ⇒ 不可落。
///
/// ## 为什么第 3 步问行而不是问「有没有 granted」
///
/// 「能不能收束」的判据只该有一处。问行 ⇒ 判据在表里（`owes: authz.granted`），
/// 将来改级联形状只改表；在代码里再写一遍 `has_granted(..)` 就有了第二个判据，
/// 而它不会跟着表变——那正是本模块要收掉的那种分叉。
///
/// ## 为什么入参是 `&S` 而不是事实切片
///
/// 第 1 步**写**事实，第 2 / 3 步必须看见刚写完的那一份（与 `port::announce` →
/// `owes` 同一条顺序纪律：先落、再问。反过来问表恒不欠，而写反了不会有任何东西
/// 报错）。
pub(crate) fn allow<S>(
    store: &S,
    actor: &ActorSpec,
    turn: u64,
    user_seq: u64,
    kind: ClosureKind,
    key: &str,
) -> Verdict
where
    S: Store<Event = Event> + ?Sized,
{
    if !land_intent(store, actor, turn, user_seq, kind, key) {
        return Verdict::Denied("收束意图未入格（义务环不会承认这次收束）".into());
    }
    if owed("authorize-close", store, turn) {
        match closure_gate(actor, kind) {
            Ok(()) => grant(store, actor, turn, user_seq, kind, key),
            Err(why) => {
                deny(store, actor, turn, user_seq, kind, key, &why);
                return Verdict::Denied(why);
            }
        }
    }
    if owed("close-turn", store, turn) {
        Verdict::Granted
    } else {
        Verdict::Denied("收束行不欠（无 `authz.granted` 或收束格已落）".into())
    }
}

/// 这一行此刻欠不欠（本轮切片，`now` 取墙钟——两行都不读时间，取什么都一样）。
fn owed<S>(row: &str, store: &S, turn: u64) -> bool
where
    S: Store<Event = Event> + ?Sized,
{
    super::loop_rows::owes(
        row,
        &store.range(Seq::new(0)),
        turn,
        crate::symbio_core::clock_now_ms(),
    )
}

/// 落 `turn.closing`（决策）。`false` = 没落成 ⇒ 后面的授权不会开账。
///
/// 幂等：键 `closing-{turn}-{kind}`——续写幕重跑同一轮时说的是同一件事，
/// `Duplicate` 视为已记录（与 `port::announce` 同一条）。
fn land_intent<S>(
    store: &S,
    actor: &ActorSpec,
    turn: u64,
    user_seq: u64,
    kind: ClosureKind,
    key: &str,
) -> bool
where
    S: Store<Event = Event> + ?Sized,
{
    let kind_name = kind.name();
    let event = Event::pending(
        format!("closing-{turn}-{kind_name}"),
        EVENT_TURN_CLOSING,
        Entity::Turn,
        Verb::Asserted,
        turn,
        &actor.principal,
    )
    .with_produced_by(user_seq)
    .with_payload(serde_json::json!({ "key": key, "kind": kind_name }));
    match store.append(event) {
        Ok(_) => true,
        Err(AppendError::Duplicate) => true,
        Err(e) => {
            crate::plugin_warn!(
                "actors",
                "[v2] 收束意图未入格（turn={turn} / {kind_name} / {key}）：{e:?}"
            );
            false
        }
    }
}

/// 落 `authz.granted`（许可）：与 [`deny`] 同形同值域的**肯定**事实。
fn grant<S>(store: &S, actor: &ActorSpec, turn: u64, user_seq: u64, kind: ClosureKind, key: &str)
where
    S: Store<Event = Event> + ?Sized,
{
    let kind_name = kind.name();
    let event = Event::pending(
        format!("grant-{turn}-{kind_name}"),
        EVENT_AUTHZ_GRANTED,
        Entity::Turn,
        Verb::Asserted,
        turn,
        &actor.principal,
    )
    .with_produced_by(user_seq)
    .with_payload(serde_json::json!({ "key": key, "kind": kind_name }));
    match store.append(event) {
        Ok(_) => {}
        Err(AppendError::Duplicate) => {}
        Err(e) => crate::plugin_warn!(
            "actors",
            "[v2] 许可事实未入格（turn={turn} / {kind_name} / {key}）：{e:?}"
        ),
    }
}

/// 写侧授权闸（[plan/01 §7](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构") 写侧，
/// [04 §3.1 批⑥](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地")）：该主体此刻能否写
/// `kind` 所指的那一格。
///
/// ## 谓词与判据
///
/// 谓词是 `PermissionMatrix` 的两种判定（写侧闸判什么**只定义一次**，见
/// `symbio_core::governance`）：`Speech` 问 `can_reply`，`Artifact` 问
/// `can_produce_artifact`。`Speech` 的 `first` 恒为 `true`：运行器一轮只落一格
/// 发言收束（`final_unique_per_turn` 是不变量），不存在「同轮追加」那一态。
///
/// ## 拒绝之后
///
/// 判定本身**不落格**（那是调用点的事），它只回答「为什么拒」。落 `authz.denied`
/// 的是 [`deny`]——级联之后它由 [`allow`] 统一调，调用点不再各写一遍。
///
/// ## 第二道：作用域上限（[T-105]）
///
/// 主体自己那一行之外还有一道——**委派不放大权威**：`scope = child <parent>` 的
/// 主体（`agent_run` 派生的子会话）能落的格，其**父主体也必须能落**。
/// `root` = 独立主体，无上限。语义与取值点见
/// [T-128 §3.2](../../../../docs/009-task/128-多主体与对等承诺.md "T-128-多主体与对等承诺")
/// 与 `chat_loop::state::request_scope`。
///
/// 平凡值下（顶层会话 = `root`，以及主智能体派生的子会话）两道都是恒真——
/// 闸是 fail-closed 的**结构**，不是生产里会翻面的开关。
///
/// [T-105]: ../../../../docs/009-task/105-Scope-Child首产点批.md "T-105-Scope-Child首产点批"
pub(crate) fn closure_granted(actor: &ActorSpec, kind: ClosureKind) -> bool {
    closure_gate(actor, kind).is_ok()
}

/// 与 [`closure_granted`] **同一个判定**，但把「为什么拒」带出来。
///
/// 分成两个函数而不是各写一份谓词：`closure_granted` 是 `is_ok()` 的一行壳，
/// 判据只有这一处。两处各写一套「这一类格要什么能力」就会让闸门与拒绝原因分叉
/// ——那时 `denied` 事实记的原因与实际拒它的理由不是同一件事，比不记更坏。
///
/// 原因字符串里带**能力名**（`Capability::name()`）而不是「缺 reply.first」这种
/// 手写措辞：能力名是表里的同一份数据，改了名字这里跟着改；手写措辞不会。
pub(crate) fn closure_gate(actor: &ActorSpec, kind: ClosureKind) -> Result<(), String> {
    // 八个收束调用点全在「本轮尚无收束格」的位置，因此发言只问一个 `Capability::Reply`
    // ——名住 core，规格方法与表谓词共用它（ADR-103：不按轮次位置分能力）。
    let own = match kind {
        ClosureKind::Speech => actor.can_land_speech(),
        ClosureKind::Artifact => actor.can_land_artifact(),
    };
    if !own {
        return Err(format!(
            "{} 不持 {}",
            actor.principal,
            kind.capability_name()
        ));
    }
    within_delegation_ceiling(actor, kind)
}

/// 落一条**否定事实**（`EVENT_AUTHZ_DENIED`，T-219）。
///
/// 闸门拒绝时此前只留一条 `warn`：日志里剩下的是「该轮没收束」，与「根本没跑完」
/// 同形。本函数把否定落成事实——判定与矩阵**一个字不动**，变的只是否定的表达。
///
/// ## 为什么是「落事实」而不是「返回错误」
///
/// I1 单通道：一切状态变更只经事实源。「被拒绝」是一个状态变更，因此它也必须
/// 是一条事实——否则不可审计（谁 / 何时 / 因何被拒）、不可回放（没有东西可重放）。
///
/// ## 三条边界（与 `EVENT_AUTHZ_DENIED` 的载荷说明同源）
///
/// 1. **不是收束格**：落在 `turn × asserted`。该轮仍然未收束，`unresolved_turns`
///    照报——把它算成收束等于把「静默中断」这条最该被发现的失效一起掩盖掉。
/// 2. **幂等**：键 `deny-{turn}-{kind}`，`Duplicate` 静默吞掉。产物臂在工具结果的
///    循环里，多轮 / 多个结果会重复走到闸前，而它们说的是同一件事。
/// 3. **记账失败不改变结论**：落不进去也照原样返回拒绝结果（warn 里带错误）。
///    闸门的结论是「不许写」，不能因为记账失败就放行。
///
/// 载荷 `{ key, kind, reason, detail }`：`reason` 取 [`ClosureReason::Denied`]——
/// 与收束格**同一个字段名、同一个闭集**，于是读侧能用**一个**问题（「这一轮为什么停」）
/// 问遍两种格，不必先猜这是收束格还是否定事实。`detail` 是人话，只给人读。
///
/// [`ClosureReason::Denied`]: crate::symbio_core::ClosureReason::Denied
pub(crate) fn deny<S>(
    store: &S,
    actor: &ActorSpec,
    turn: u64,
    user_seq: u64,
    kind: ClosureKind,
    key: &str,
    detail: &str,
) where
    S: Store<Event = Event> + ?Sized,
{
    let kind_name = kind.name();
    let event = Event::pending(
        format!("deny-{turn}-{kind_name}"),
        EVENT_AUTHZ_DENIED,
        Entity::Turn,
        Verb::Asserted,
        turn,
        &actor.principal,
    )
    .with_produced_by(user_seq)
    .with_payload(serde_json::json!({
        "key": key,
        "kind": kind_name,
        "reason": crate::symbio_core::ClosureReason::Denied.as_str(),
        "detail": detail,
    }));
    match store.append(event) {
        Ok(_) => {}
        // 同一轮同一格被拒多次：说的是同一件事，落一条就够。
        Err(AppendError::Duplicate) => {}
        Err(e) => crate::plugin_warn!(
            "actors",
            "[v2] 拒绝事实未入格（turn={turn} / {kind_name} / {key}）：{e:?}"
        ),
    }
}

/// 作用域上限（[T-105]）：`scope = child <parent>` ⇒ 父主体也必须能落这一类格。
///
/// 谓词与 [`closure_granted`] 的**同一份**（`can_reply` / `can_produce_artifact`）
/// ——两处各写一套「这一类格要什么能力」就会让上限与许可分叉。
fn within_delegation_ceiling(actor: &ActorSpec, kind: ClosureKind) -> Result<(), String> {
    let crate::symbio_core::Scope::Child(parent) = &actor.scope else {
        return Ok(());
    };
    let matrix = crate::symbio_core::production_matrix();
    let granted = match kind {
        ClosureKind::Speech => matrix.can_reply(parent),
        ClosureKind::Artifact => matrix.can_produce_artifact(parent),
    };
    if granted {
        return Ok(());
    }
    Err(format!(
        "父主体 {parent} 不持 {}（委派不放大权威）",
        kind.capability_name()
    ))
}

#[cfg(test)]
#[path = "close.test.rs"]
mod tests;
