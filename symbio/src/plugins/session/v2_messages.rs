//! 消息变更事实的写方（ADR-053）。
//!
//! 「编辑一条消息」「截断一段消息」在轨 S 之前只重写 `messages.json`——事实源里
//! 一个字都没有。后果是**正确性**的：主读路径已切到「从 WAL 重建」，被截断的
//! 消息会**复活**、编辑过的内容会**回退**。本模块是这两件事**入格**的写方：
//! `commands::patch_message` / `commands::truncate_messages` 在重写缓存之后各调一次。
//!
//! ## 追加，不是改写
//!
//! 本模块对一次变更**只追加一条事实**、不动任何既有行为——重建时新事实排在旧
//! 事实之后，末态由列表投影折出。
//!
//! ## 失败口径：`AppendError` 只记日志，不冒泡
//!
//! 事实写入失败（拿不到写者令牌 / 撞幂等键）**不得**让一次编辑失败：缓存已经
//! 写成功了，用户看到的就是「改好了」。这与 `v2_facts::record_learning` 同一条
//! 口径（附加事实失败不该把成功的动作说成失败）。
//!
//! 三点边界要写清楚，否则「不冒泡」会被读成「什么都不会炸」：
//!
//! - **存储层的 IO 灾难仍会 panic**——那是 [`Store::append`] 自己的既有契约
//!   （「不可静默」），不是本模块的口径；本模块只吞 [`AppendError`]。
//! - **`NotTheWriter` 有寄存补落**（T-055，见下「写者令牌」一节）。
//! - **其余 `AppendError` 归 `Discarded`，调用方静默忽略**——主读切到 WAL 之后，
//!   该次编辑在重建后会**丢失且用户无感**。b3 批预告过「届时升级为错误」、随批
//!   漏做；升级与否的裁决归 T-164，本模块暂维持日志口径（告警文案已与切源事实对齐）。
//!
//! ## 写者令牌：会话正跑着时编辑会落不了格（T-055 已解决）
//!
//! [`EventWalStore::open`] 用文件锁取写者令牌（I1 单写者的跨进程边界）。一轮
//! 正在跑时它持有令牌，此刻来的编辑拿不到 ⇒ `NotTheWriter` ⇒ 少一条事实。
//!
//! 这曾是一条**静默的数据缺陷**：主读切到 WAL 之后，它会让编辑「偶发不生效」
//! 且无人报错。修法（T-055）与 b2 的压缩寄存同手法——**变更寄存，由持令牌的一方落格**：
//!
//! - 拿不到令牌时，把这条变更推进 [`PARKED`]（按会话目录分桶的进程内寄存区）；
//! - 持令牌的一方（`v2_exec::execute_turn` 的执行口）在下一次落格时调
//!   [`drain_parked`]，把寄存的变更补落进事实源。
//!
//! 「寄存」而不是「重试 / 阻塞等待」：编辑是**用户对列表的操作**，它的发起方
//! （VDFS 路由）在不持令牌的那条链上；阻塞等待一轮跑完（可能几分钟）会把用户界面
//! 拖死，而重试同样要等。「寄存 + 由令牌持有方落」是**唯一**能同时满足「事实必须
//! 落格」与「落格必须持令牌」两条约束的形态——与 `SessionContext::consolidated`
//! 完全同一条纪律（见该字段文档）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use serde_json::Value;

use crate::symbio_core::chat_node::ChatNode;
use crate::symbio_core::{
    AppendError, Entity, Event, EventWalStore, Seq, Store, Verb, EVENT_MESSAGE_ADDED,
    EVENT_MESSAGE_EDITED, EVENT_MESSAGE_REMOVED,
};

use super::paths::V2_WAL_FILE;

/// 一条**待寄存**的变更事实（拿不到写者令牌时暂存，由持令牌方补落）。
///
/// 形状刻意 = `append` 的入参元组：寄存就是为了**原样重放**同一次落格，
/// 因此不重新解释、不预先算锚（锚必须按**补落那一刻**的切片算——那时切片已经
/// 长出了新一轮的事件，用旧的锚会指到一个过期状态）。
#[derive(Debug, Clone)]
struct ParkedChange {
    kind: &'static str,
    verb: Verb,
    id_prefix: &'static str,
    target: String,
    payload: Value,
}

/// 进程内**寄存区**：会话目录 → 未落格的变更（FIFO）。
///
/// ## 为什么是进程内静态而不是磁盘
///
/// 寄存的**生命周期**就是「直到同进程里的下一个执行口落格」。跨进程（另一个
/// 进程持有令牌）在单写者模型里本就不该发生——令牌是**文件锁**，持锁进程崩溃时
/// 锁自动释放、下一个进程才拿得到。此时寄存区在旧进程的内存里、随它一起消失，
/// 这正是「进程内的东西不该假装持久」。真正需要跨崩溃保全的是**已落格**的事实，
/// 而那正是 WAL 的职责。
static PARKED: LazyLock<Mutex<HashMap<PathBuf, Vec<ParkedChange>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 寄存器上界（每会话）。超出即丢**最旧**的一条并告警——见 [`park`] 的理由。
///
/// 取 256：一次编辑最多两条（edited），一次截断一条（removed ids 已聚合），
/// 而寄存只发生在「一轮运行中」这个窗口里（几秒到几分钟）。256 远远够用；
/// 真堆到这个数说明令牌持有方长期没落格（比寄存溢出更严重的缺陷），
/// 告警把它暴露出来，而不是让它无声地撑爆内存。
const PARK_CAP: usize = 256;

/// 把一条变更寄存下来（拿不到令牌的路径）。
fn park(session_dir: &Path, change: ParkedChange) {
    let mut table = match PARKED.lock() {
        Ok(t) => t,
        Err(p) => p.into_inner(), // 锁中毒：寄存器是纯内存暂存，按无中毒处理（与 store 层同约定）
    };
    let bucket = table.entry(session_dir.to_path_buf()).or_default();
    if bucket.len() >= PARK_CAP {
        // 丢最旧的：寄存区是**尽力而为**的暂存，不是事实源。堆满说明持令牌方
        // 长期没落格——那是比溢出更该被看见的缺陷，故告警而不是静默截断。
        let dropped = bucket.remove(0);
        crate::plugin_warn!(
            "session",
            "[message] 寄存区已满（{PARK_CAP}），丢弃最旧的一条寄存变更（{} {target}）\
             ——持令牌的一方长期未落格，请排查",
            dropped.kind,
            target = dropped.target,
        );
    }
    bucket.push(change);
}

/// 取出并清空某会话的寄存变更（持令牌方在落格前调用）。
fn take_parked(session_dir: &Path) -> Vec<ParkedChange> {
    let mut table = match PARKED.lock() {
        Ok(t) => t,
        Err(p) => p.into_inner(),
    };
    table.remove(session_dir).unwrap_or_default()
}

/// **把寄存的变更补落进事实源**——持写者令牌的一方（执行口）调用。
///
/// ## 为什么必须传 `store` 进来（而不是自己 `open`）
///
/// 写者令牌是**文件锁**：同一进程里再 `open` 一次也拿不到（`try_lock` 失败 ⇒
/// 降级成只读实例 ⇒ `append` 恒 `NotTheWriter`）。所以补落**必须复用持令牌的那个
/// 实例**——传 `&EventWalStore` 不是为了省一次 IO，是因为自己 `open` 出来的那个
/// **根本不是写者**（这正是本任务要修的形态本身）。
///
/// ## 调用时机
///
/// 在 [`EventWalStore::open`] 成功**之后**、本轮的其它落格**之前**（见
/// `v2_exec::execute_turn` 与 `execute_final_reply_turn` 的落格窗口），且必须在
/// **读切片之前**——补落改变了 live 段，本轮后面的 turn 号 / 续写锚 / 幂等键都从
/// 切片折，先落完才不会站在一份缺了那次编辑的状态上做判断。
///
/// ## 幂等
///
/// 逐条走 [`append_into`]，幂等键与就地落格完全一致（`{prefix}-{n}`）——因此
/// 「同一条变更被寄存两次 / 补落两次」不会写出两条事实。补落仍失败（极端：令牌在
/// 这一瞬间被抢走）⇒ 重新寄存，等下一个窗口。
///
/// ## 为什么补落期间要重新算锚
///
/// 寄存时**不算锚**（见 [`ParkedChange`]）：锚必须按落格那一刻的切片算。
/// 补落发生在原编辑之后的某个执行口，此时切片已经长出了这一轮的事件——用编辑
/// 那一刻的锚会把事实指到一个**过期状态**上（I2 要的是「这次变更站在哪份状态上」，
/// 而那是**落格时刻**的状态）。
pub fn drain_parked(session_dir: &Path, store: &EventWalStore) {
    let parked = take_parked(session_dir);
    if parked.is_empty() {
        return;
    }
    let wal = wal_path(session_dir);
    for change in parked {
        match append_into(
            store,
            &wal,
            change.kind,
            change.verb,
            change.id_prefix,
            &change.target,
            change.payload.clone(),
        ) {
            AppendOutcome::Landed => {}
            // 又拿不到令牌（极端并发）⇒ 原样放回，等下一个窗口。
            AppendOutcome::NotWriter => park(session_dir, change),
            // 无锚 / 落格失败：寄存也救不了——按既有口径出声后丢（与就地落格同一处置）。
            AppendOutcome::Discarded => {}
        }
    }
}

/// 载荷里可能记着**本条事件所承载的节点身份**的键。
///
/// 与 `rebuild` / `transcript` 取节点身份用的是同一批键（见
/// [`crate::symbio_core::event_payload_node_id`]）——这里是反向查：**哪个事件承载了
/// 节点 X**。键列在这里而不是内联进 `find`：加一格带新身份键的事件时，
/// 「哪些键是节点身份」必须只有一处答案。
const NODE_ID_KEYS: &[&str] = &["id", "text_id", "reasoning_id", "tool_call_id", "result_id"];

/// 编辑一条消息 ⇒ 落一条 `message.edited`。
///
/// `msg` 是**补丁应用后**的整条消息（不是补丁本身）——重建与转写都按 id 整条替换，
/// 拿到整条就不必再猜「这次改的是哪几个字段」（理由见 `EVENT_MESSAGE_EDITED`）。
pub(crate) fn edited(session_dir: &Path, msg: &ChatNode) {
    let payload = match serde_json::to_value(msg) {
        Ok(v) => v,
        Err(e) => {
            crate::plugin_warn!("session", "[message] 编辑事实未入格：序列化失败：{e}");
            return;
        }
    };
    append(
        session_dir,
        EVENT_MESSAGE_EDITED,
        Verb::Asserted,
        "m-edit",
        &msg.id,
        serde_json::json!({ "message": payload }),
    );
}

/// 一条消息**诞生** ⇒ 落一条 `message.added`（载荷 `{ messages: [...] }`）。
///
/// 只服务「**没有别的格可搭车**」的节点（压缩记录节点是最典型的一个）——承载着
/// 对话内容的节点（用户发言 / 助手正文 / 工具结果）必须由它们自己的事实落，
/// 否则同一条节点有两个 owner（ADR-049 理由 1）。
///
/// 空列表不落格：「没有节点诞生」不该在事实源上留下痕迹，与 [`removed`] 同一条判据。
pub(crate) fn added(session_dir: &Path, msgs: &[ChatNode]) {
    if msgs.is_empty() {
        return;
    }
    let payload = match serde_json::to_value(msgs) {
        Ok(v) => v,
        Err(e) => {
            crate::plugin_warn!("session", "[message] 诞生事实未入格：序列化失败：{e}");
            return;
        }
    };
    append(
        session_dir,
        EVENT_MESSAGE_ADDED,
        Verb::Opened,
        "m-add",
        // 溯源锚取**第一条**：与 [`removed`] 同形——一次诞生的一个动作，锚在它身上。
        msgs.first().map(|m| m.id.as_str()).unwrap_or_default(),
        serde_json::json!({ "messages": payload }),
    );
}

/// 一段消息没了 ⇒ 落一条 `message.removed`（载荷 `{ ids: [...] }`）。
///
/// `ids` 是**被删节点 id 的完整列表**（与 `truncate_messages` 回执里那份同源）。
/// 空列表不落格：「什么都没删」不该在事实源上留下痕迹——与
/// `truncate_messages` 不发任何变更帧同一条判据。
pub(crate) fn removed(session_dir: &Path, ids: &[String]) {
    if ids.is_empty() {
        return;
    }
    append(
        session_dir,
        EVENT_MESSAGE_REMOVED,
        Verb::Closed,
        "m-del",
        // 溯源锚取**第一条被删的**：截断的语义是「从 X 起」——X 就是那个起点。
        ids.first().map(String::as_str).unwrap_or_default(),
        serde_json::json!({ "ids": ids }),
    );
}

/// 一条变更事实落格的**结果**（[`try_append`] 的返回）。
enum AppendOutcome {
    /// 落了格。
    Landed,
    /// 拿不到写者令牌 ⇒ **寄存**，等持令牌方补落（T-055）。
    NotWriter,
    /// 落不了且寄存也无益（事实源打不开 / 没有可用溯源锚）⇒ 出声后放弃。
    Discarded,
}

/// 把一条变更事实追加进会话的事实源；拿不到令牌时**寄存**（T-055 的入口）。
///
/// `target` = 本次变更作用在哪个节点上（用于解溯源锚）；`id_prefix` 是幂等键前缀。
fn append(
    session_dir: &Path,
    kind: &'static str,
    verb: Verb,
    id_prefix: &'static str,
    target: &str,
    payload: Value,
) {
    match try_append(session_dir, kind, verb, id_prefix, target, payload.clone()) {
        AppendOutcome::Landed => {}
        // **寄存，而不是出声丢弃**（T-055）：一轮正在跑时它持着写者令牌，此刻来的
        // 编辑拿不到 ⇒ 静默少一条事实。切源之后那就是「编辑偶发不生效」且无人报错。
        // 把变更寄存在进程内，由持令牌的一方（执行口 `drain_parked`）补落。
        //
        // 载荷**原样寄存**：锚与幂等键都在补落那一刻重算（见 `drain_parked`）。
        AppendOutcome::NotWriter => park(
            session_dir,
            ParkedChange {
                kind,
                verb,
                id_prefix,
                target: target.to_string(),
                payload,
            },
        ),
        AppendOutcome::Discarded => {}
    }
}

/// 从变更载荷里解出这次变更的**主体与时刻**。
///
/// 载荷的三张脸：`{ message: {...} }`（编辑）/ `{ messages: [...] }`（诞生）/
/// `{ ids: [...] }`（删除）。只有前两种谈得上主体，且口径与 `plugin::attributed`
/// 一致：消息自带 `principal` 就认它，否则按 `role` 派生 —— **只有 `role=user` 才是
/// 用户**；助手正文 / 工具结果 / 压缩记录 / 系统说明都属本会话主体（`agent:main`）。
///
/// 取不到时回落到 `user` + `ts = 0`（旧行为）。回落是为了老写方不被这次修正打懵，
/// 但它是**兜底不是默认**：有 principal / role 可判时绝不落到上面。
fn actor_and_ts(payload: &Value) -> (String, i64) {
    use crate::symbio_core::authz::{PRINCIPAL_MAIN, PRINCIPAL_USER};

    let msg = payload.get("message").cloned().or_else(|| {
        payload
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .cloned()
    });
    let Some(msg) = msg else {
        // 删除类载荷没有对象可言：npm 变更本身的确不知道是谁留下的那些条。
        // 保留旧行为，宁可标 user 也不编一个。
        return (PRINCIPAL_USER.to_string(), 0);
    };

    // `principal` 兼容两种 wire 形态（[T-147b]，[ADR-069 §决策 6]）：新形态是
    // `{kind, id, title?}` 对象、历史 `messages.json` 里是裸字符串——都通过
    // `Principal::deserialize` 一次性折到 `.id`，事件层的 actor 键仍是**可路由地址字符串**。
    let principal = msg
        .get("principal")
        .and_then(|v| {
            serde_json::from_value::<crate::symbio_core::principal::Principal>(v.clone()).ok()
        })
        .map(|p| p.id);
    let actor = principal.unwrap_or_else(|| {
        match msg.get("role").and_then(Value::as_str) {
            Some("user") => PRINCIPAL_USER.to_string(),
            // assistant / tool / system —— 都是本会话主体的产出
            _ => PRINCIPAL_MAIN.to_string(),
        }
    });
    // `ts`：消息自带的时刻是**事实**，优先用它；没有则退到「这条变更是何时落下的」
    // ——那是此刻，也是事实。
    //
    // 一律写 0，于是 548 个事件的时间戳全是一坨相同的 0，时序无从谈起——不是
    // 「未知」，是**一句假话**（假装都发生在纪元起点）。根因在上游：`NodeRole::
    // Assistant` 的构造点有 9 处，只有 1 处填了 `timestamp`（其余是复制胚胎的各份
    // 手写实现）。逐个去补那 8 处是同一类错误的第三次重演——这里兜底一层，无论谁
    // 写的消息，落到事实上都有真实时刻。
    let ts = msg
        .get("timestamp")
        .and_then(Value::as_i64)
        .filter(|t| *t > 0) // 上游把「没填」写成 0，这里一并兜住
        .unwrap_or_else(crate::symbio_core::clock_now_ms);
    (actor, ts)
}

/// [`append`] 的实现：打开事实源、解锚、落格——**不寄存**（寄存留给调用方决定）。
///
/// 抽出来是为了让 [`drain_parked`] 能复用**同一份**落格逻辑（一种变更只有一个
/// 落格实现，寄存与否只是它外面的分支）。
fn try_append(
    session_dir: &Path,
    kind: &str,
    verb: Verb,
    id_prefix: &str,
    target: &str,
    payload: Value,
) -> AppendOutcome {
    let wal = session_dir.join(V2_WAL_FILE);
    let store = match EventWalStore::open(&wal) {
        Ok(s) => s,
        Err(e) => {
            crate::plugin_warn!(
                "session",
                "[message] 变更事实未入格（{}）：事实源打不开：{e}",
                wal.display()
            );
            return AppendOutcome::Discarded;
        }
    };
    append_into(&store, &wal, kind, verb, id_prefix, target, payload)
}

/// 事实源文件路径（仅用于告警文案——`WalStore` 不暴露 path）。
fn wal_path(session_dir: &Path) -> PathBuf {
    session_dir.join(V2_WAL_FILE)
}

/// 用**给定的**事实源实例落一条变更事实（不自己 `open`）。
///
/// 拆出来是为了让 [`drain_parked`] 复用持令牌的那个实例——写者令牌是文件锁，
/// 自己 `open` 出来的实例根本不是写者（见 `drain_parked` 的文档）。
///
/// `wal` 只用于告警文案（`WalStore` 不暴露 path）；调用方从 `session_dir` 拼。
fn append_into(
    store: &EventWalStore,
    wal: &Path,
    kind: &str,
    verb: Verb,
    id_prefix: &str,
    target: &str,
    payload: Value,
) -> AppendOutcome {
    // `history()` 而非 `range()`（S2.7-b2）：
    // - **溯源锚**：被编辑的消息若来自压缩前（其定义事件在归档段里），只读 live 会
    //   找不到载体 ⇒ 退回 `last()` 的弱锚（`provenance` 的兜底）；
    // - **幂等键**：`n` = 本会话内 `Message` 实体的**全局**计数。压缩归档把前缀移出
    //   live 后，只读 live 会让 `n` 从头数起 ⇒ `{prefix}-{n}` 与归档段里的 id 重号
    //   （归档后旧 id 已离开内存幂等键集合，故不报错，但全局 id 唯一性被打破）。
    let snapshot = store.history();
    let Some(anchor) = provenance(&snapshot, target) else {
        // 切片为空（或切片内所有事件都没有 seq）⇒ 连「这次变更站在哪一份状态上」
        // 都无从谈起。会话还没有任何事实（v1 时代建的会话 / 从未走过 v2 轮）。
        // **不硬造一个号**：溯源是断言类事件的硬要求（I2），造不出就宁可不落。
        //
        // 寄存也救不了它：补落时切片至少含本轮开轮事件（锚必然可解），但**本轮
        // 之前**就是空切片的话，说明这个会话从未走过 v2——那时「编辑一条 v1 遗留
        // 消息」本就无锚可指，与「令牌被占」是两回事，故仍按 Discarded 出声。
        crate::plugin_warn!(
            "session",
            "[message] 变更事实未入格（{}）：切片内没有可用的溯源锚（{kind} {target}）",
            wal.display()
        );
        return AppendOutcome::Discarded;
    };
    // 幂等键：本会话内 `message` 实体的第 n 条。**不是「一条消息一个号」**——
    // 同一条消息可以被编辑多次，每次都是一条新事实；两种变更共用同一个计数，
    // 于是 id 天然不撞。
    let n = snapshot
        .iter()
        .filter(|e| e.entity == Entity::Message)
        .count();
    // —— actor 与 ts：从**被变更的消息本身**取，不写死 ——
    //
    // 不写死 `PRINCIPAL_USER`，理由是「变更由用户对列表的操作发起」（`d781a6c7`
    // 号会话把它证伪：那次会话 363 条 `message.edited` 里 **role=user 的是 0 条**——全是
    // 助手流式正文与工具结果，却一条不落地被记成了 `user`。后果不是「标签不好看」：
    // actor 是**鉴权语义**（谁在说话 / 谁的行为），把 agent 的产出署成用户，等于让
    // agent 拿用户身份落账；任何按主体过滤或计数的地方都会算错（真消息 1 条被读成
    // 364 条）。
    //
    // -correct- 口径照抄 `plugin::attributed`（「落库前补身份的唯一执行点」）：
    // 消息自带 `principal` 就认它；没有则按 role 派生——`user` 才是用户，其余（助手
    // 正文 / 工具结果 / 系统说明）都是本会话主体。两者都缺才回落到 `user`（历史写方），
    // 那时确实无从判断，但不再是「一律」。
    let (actor, ts) = actor_and_ts(&payload);
    let event = Event::pending(
        format!("{id_prefix}-{n}"),
        kind,
        Entity::Message,
        verb,
        // `turn = 0`：变更不属于任何一轮（截断跨轮）。与 `memory.recalled` /
        // `commitment.*` 同一条既有约定。
        0,
        actor,
    )
    .with_produced_by(anchor)
    .with_ts(ts)
    .with_payload(payload);
    match store.append(event) {
        Ok(_) => AppendOutcome::Landed,
        // **令牌被占**是自己的一类（寄存后能救回来），不能被归进"失败"里
        // ——那正是 T-055 要消灭的静默丢弃。
        Err(AppendError::NotTheWriter) => AppendOutcome::NotWriter,
        Err(e) => {
            crate::plugin_warn!(
                "session",
                "[message] 变更事实未入格（{}）：{e:?}（缓存已写成功，但主读已切 WAL：\
                 该编辑在下次重建后会丢失——升级裁决见 T-164）",
                wal.display()
            );
            AppendOutcome::Discarded
        }
    }
}

/// 溯源锚：**被改的那条事实**（承载该节点的事件 seq）。
///
/// 找不到（节点不在事实源里：v1 时代的历史消息、或按事件 id 合成的节点）⇒
/// 退化为**切片末尾的 seq**——它仍然回答一个确切的问题：「这次变更站在哪一份
/// 状态上」。退化的后果是「这次编辑源自哪条发言」变粗，而不是「没有溯源」。
///
/// 承载事件**没有 seq**（2026-09-30 那次修复之前落下的旧 WAL 行：`seq: null`）
/// 走同一条退化——它同样回答不了「源自哪条」，但不该因此把整条事实丢掉。只有
/// **切片整个为空**才返回 `None`（那时连「站在哪份状态上」都无从谈起）。
fn provenance(snapshot: &[Event], node: &str) -> Option<u64> {
    let carrier = snapshot.iter().find(|e| {
        NODE_ID_KEYS
            .iter()
            .any(|k| e.payload.get(*k).and_then(Value::as_str) == Some(node))
    });
    carrier
        .and_then(|e| e.seq)
        .or_else(|| snapshot.last().and_then(|e| e.seq))
        .map(Seq::value)
}

#[cfg(test)]
#[path = "v2_messages.test.rs"]
mod tests;
