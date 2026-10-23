//! 消息变更事实的写方（D-053）。
//!
//! 「编辑一条消息」「截断一段消息」在轨 S 之前只重写 `messages.json`——事实源里
//! 一个字都没有。后果是**正确性**的：主读路径已切到「从 WAL 重建」，被截断的
//! 消息会**复活**、编辑过的内容会**回退**。本模块是这两件事**入格**的写方：
//! `commands::patch_message` / `commands::truncate_messages` 经唯一写入口
//! `chat_session::apply` 的步骤 ① 各调一次——**先于**写缓存（D-054 决策 2 的
//! 顺序反转），所以这里失败时缓存还没动过。
//!
//! ## 追加，不是改写
//!
//! 本模块对一次变更**只追加一条事实**、不动任何既有行为——重建时新事实排在旧
//! 事实之后，末态由列表投影折出。
//!
//! ## 失败口径：五个结果、四种处置，真失败**冒泡**（ / D-053 的 b3 工序）
//!
//! 旧口径是「事实失败只记日志、不冒泡」，理由是「附加事实失败不该把成功的动作
//! 说成失败」——那在 b1/b2 成立：缓存是读侧真源、事实是影子。**b3 把主读切到
//! WAL 之后事实就是本体**，影子成了唯一副本，放过它就等于「回执说改好了、
//! 重建后一看没改」，而且用户无感。
//!
//! 本模块因此把结果分成五种（[`AppendOutcome`]），**处置由调用方定**：
//!
//! - [`AppendOutcome::Landed`] / [`AppendOutcome::Nothing`]：落了 / 本来就没有事实可落；
//! - [`AppendOutcome::Parked`]：拿不到写者令牌 ⇒ **寄存**，由持令牌方补落；
//! - [`AppendOutcome::NoAnchor`]：事实源里没有可用的溯源锚——v1 遗留会话 / 从未走过
//!   v2 轮。这类会话的读侧本就回落 `messages.json`（D-049），缓存仍是真源 ⇒ **豁免**；
//! - [`AppendOutcome::Failed`]：事实源打不开 / 追加被拒。**这是真失败**：主读已切 WAL，
//!   这次变更在重建后会丢。`chat_session::apply` 在写缓存**之前**把它变成可见错误
//!   ——报失败是如实，不是「把成功的动作说成失败」。
//!
//! 一条边界不变：存储层的 IO 灾难仍会 panic——那是 [`Store::append`] 自己的契约
//! （「不可静默」），不是本模块的口径。
//!
//! ## 写者令牌：只有**跨进程**才拿不到它（ 立机器， 把它退成兜底）
//!
//! [`EventWalStore::open`] 用文件锁取写者令牌（I1 单写者的跨进程边界）。一轮
//! 正在跑时它持有令牌，此刻来的编辑拿不到 ⇒ `NotTheWriter` ⇒ 少一条事实。
//!
//! 这曾是一条**静默的数据缺陷**：主读切到 WAL 之后，它会让编辑「偶发不生效」
//! 且无人报错。修法与 b2 的压缩寄存同手法——**变更寄存，由持令牌的一方落格**：
//!
//! - 拿不到令牌时，把这条变更**写进寄存区**（与会话 WAL 同域的
//!   `transcripts/pending/`，一条变更一个文件，见 [`park`]）；
//! - 持令牌的一方（`turn_exec::execute_turn` 的执行口）在下一次落格时调
//!   [`drain_parked`]，把寄存的变更补落进事实源，落一条删一条。
//!
//! 「寄存」而不是「重试 / 阻塞等待」：编辑是**用户对列表的操作**，它的发起方
//! （VDFS 路由）在不持令牌的那条链上；阻塞等待一轮跑完（可能几分钟）会把用户界面
//! 拖死，而重试同样要等。「寄存 + 由令牌持有方落」是**唯一**能同时满足「事实必须
//! 落格」与「落格必须持令牌」两条约束的形态——与 `SessionContext::consolidated`
//! 完全同一条纪律（见该字段文档）。
//!
//! ## 为什么寄存区在盘上而不是内存里（ 项三）
//!
//! 触发条件先收窄过一次：[DE-014 §2.2 推论 3](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")
//! 的「一写者」落地后（`SessionStore::facts` 按会话缓存**唯一**一个 RW 句柄），
//! 同进程里第二个 RW 持令牌者的形态**不存在** ⇒ 今天还能拿到 `NotWriter` 的
//! 只剩**跨进程**这一种。而进程内静态寄存区恰好在这种形态下**永远救不回来**：
//! 寄存躺在没令牌的那个进程的内存里，持令牌的进程在另一个地址空间，它的
//! `drain_parked` 看不见；没令牌的那个进程一退出（CLI 一次命令就是短命进程），
//! 寄存随之消失——**不需要崩溃也已经丢**。所以那条静态表不是「崩溃才丢」，
//! 是「触发即丢」，它把一次本该冒泡的失败（ 的 `Failed`）改成了静默的成功。
//!
//! 盘上寄存区改的就是这一条：**寄存的可见性不再依赖寄存者活着**。崩溃、退出、
//! 另一个进程补落，三种都走同一条路——文件还在，下一个持令牌的窗口把它落掉。
//! 代价是本模块多一种 IO，于是三条纪律跟着搬过来：
//!
//! - **先落盘再回执**：写寄存区失败就是真失败（`AppendOutcome::Failed`），
//! 不能既没副本又回一句"改好了"（ 同一条判据）；
//! - **一件一份、暂名再改名**：与 `WalStore::archive_before` 的暂存改名同一手法，
//!   崩溃只会留下看不见的 `.staged` 半件，不会留下半条变更；
//! - **有界 + 出声**：上界与丢最旧的告警照旧（见 [`PARK_CAP`]）。

use crate::plugins::session::store::EventWalStore;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::symbio_core::chat_node::ChatNode;
use crate::symbio_core::{
    AppendError, Entity, Event, Seq, Store, Verb, EVENT_MESSAGE_ADDED, EVENT_MESSAGE_EDITED,
    EVENT_MESSAGE_REMOVED,
};

/// 一条**待寄存**的变更事实（拿不到写者令牌时暂存，由持令牌方补落）。
///
/// 形状刻意 = `append` 的入参元组：寄存就是为了**原样重放**同一次落格，
/// 因此不重新解释、不预先算锚（锚必须按**补落那一刻**的切片算——那时切片已经
/// 长出了新一轮的事件，用旧的锚会指到一个过期状态）。
///
/// 这个形状因此就是寄存件的**盘上格式**（一件一个 JSON 文件，见 [`park`]）：
/// 它带的全部是「重放这次落格所需的入参」，没有一条是新的词汇——
/// 这正是「寄存不是第五种原语」的形状证据（`MessageChange` 三种原语的入参，
/// 延迟一会儿原样再落一次；不新增 `kind`、不新增读侧投影）。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ParkedChange {
    kind: String,
    verb: Verb,
    id_prefix: String,
    target: String,
    payload: Value,
}

/// 寄存区目录：`<会话目录>/transcripts/pending`。
///
/// `transcripts` 这一段取自 [`paths::TRANSCRIPTS_SUBDIR`]——与会话 WAL 的归档段
/// （`store::wal::archive_dir_of`）用同一个会话布局词汇，只定义一次；`pending`
/// 是本模块自己的那一段。放在 `transcripts/` 之下还有一个实际好处：VDFS 会话树是
/// **显式构造**的，这个子目录不会漏进前端的文件列表。
///
/// 它落在**事实源旁边**而不是别处：寄存的目的地就是那条 WAL，路径由 WAL 句柄
/// 自己给出（[`EventWalStore::dir`]），调用方不再重新派生一遍布局。
fn pending_dir(store_dir: &Path) -> PathBuf {
    store_dir
        .join(crate::plugins::session::paths::TRANSCRIPTS_SUBDIR)
        .join("pending")
}

/// 半件后缀：先写成 `<名>.staged`，`sync` 完再改名成 `<名>.json`（与
/// `WalStore::archive_before` 的暂存改名同一手法）。
///
/// 看得见的顺序键只有 `.json` ⇒ 崩溃留下的半件天然不在队列里，由补落方顺手清掉；
/// 于是"写坏了的寄存"只会是**没发生过**，不会是**半条变更**。
const STAGED_SUFFIX: &str = "staged";

/// 可见件后缀：**`.json` 才是已寄存的**。与 [`STAGED_SUFFIX`] 一起构成寄存区唯一的
/// 两种文件名——别的扩展名一律按「不认识的东西」清掉。
const PARKED_SUFFIX: &str = "json";

/// 同一毫秒内的两次寄存靠它分开（文件名就是顺序键，见 [`park`]）。
/// 取模只为保证宽度稳定（`{serial % 1e6}` 恒为六位）。
static PARK_SERIAL: AtomicU64 = AtomicU64::new(0);

/// 寄存件的可见性：**`.json` 才是已寄存的**，`.staged` 是没写完的半件。
fn is_parked(path: &Path) -> bool {
    path.extension().is_some_and(|x| x == PARKED_SUFFIX)
}

/// 列出寄存区里的文件（按名升序 = 寄存先后）。目录不存在 ⇒ 空
/// （「还没有人寄存过」是常态，不该是一条错误）。
fn pending_listing(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    out.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    out
}

/// 寄存件上界（每会话）。超出即丢**最旧**的一件并告警——见 [`park`] 的理由。
///
/// 取 256：一次编辑最多两条（edited），一次截断一条（removed ids 已聚合），
/// 而寄存只发生在「另一个进程持着令牌」这个窗口里（一轮 = 几秒到几分钟）。
/// 256 远远够用；真堆到这个数说明**没有持令牌的窗口出现**（比寄存溢出更严重的
/// 缺陷），告警把它暴露出来，而不是让它无声地撑爆磁盘。
const PARK_CAP: usize = 256;

/// 把一条变更**写进盘上寄存区**（拿不到令牌的路径）。
///
/// 顺序键就是文件名：`{毫秒:013}-{pid:07}-{序号:06}.json`，字典序 = 寄存先后。
/// 同机两个进程读的是同一个时钟，跨机不存在的形态（会话目录在同一块盘上），
/// 所以这个排序只需要回答一件事——「先寄存的哪一件」；补落时的落格顺序
/// 才是折叠顺序的依据，时钟回拨最多让两件先后互换，不会让它们**消失**。
///
/// **返回 `Err` = 这条变更没有任何副本**，调用方必须按真失败处理：
/// 「回执说改好了、盘上其实没有」正是这一族缺陷的原形，不能由寄存这条路自己再犯一次。
///
/// 上界在写入**之前**检查：满了先丢最旧并告警，再写新的——与内存版同一条纪律，
/// 只是"丢"现在是删一件。
fn park(store_dir: &Path, change: &ParkedChange) -> Result<(), String> {
    let dir = pending_dir(store_dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("寄存区建不起来（{}）：{e}", dir.display()))?;

    let parked: Vec<PathBuf> = pending_listing(&dir)
        .into_iter()
        .filter(|p| is_parked(p))
        .collect();
    if parked.len() >= PARK_CAP {
        // 丢最旧的：寄存区是**尽力而为**的暂存，不是事实源。堆满说明没有持令牌的
        // 窗口出现——那是比溢出更该被看见的缺陷，故告警而不是静默截断。
        let dropped = &parked[0];
        if let Err(e) = std::fs::remove_file(dropped) {
            crate::plugin_warn!(
                "session",
                "[message] 寄存区已满且最旧一件删不掉（{}）：{e}",
                dropped.display()
            );
        }
        crate::plugin_warn!(
            "session",
            "[message] 寄存区已满（{PARK_CAP}），丢弃最旧的一件寄存变更\
             ——没有持写者令牌的窗口出现，请排查"
        );
    }

    let serial = PARK_SERIAL.fetch_add(1, Ordering::Relaxed) % 1_000_000;
    let stem = format!(
        "{:013}-{:07}-{serial:06}",
        crate::symbio_core::clock_now_ms(),
        std::process::id()
    );
    let visible = dir.join(format!("{stem}.{PARKED_SUFFIX}"));
    let staged = dir.join(format!("{stem}.{STAGED_SUFFIX}"));
    let bytes = serde_json::to_vec(change).map_err(|e| format!("寄存件序列化失败：{e}"))?;
    let written = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        f.write_all(&bytes)?;
        // 寄存的**全部意义**是「进程不在了它还在」，所以这一句 `sync` 不是可选项：
        // 只写进操作系统缓冲就回执，崩溃时照样丢，等于把缺陷从内存搬到了页缓存。
        f.sync_all()?;
        drop(f);
        std::fs::rename(&staged, &visible)
    })();
    written.map_err(|e| format!("寄存写入失败（{}）：{e}", dir.display()))
}

/// **把盘上寄存的变更补落进事实源**——持写者令牌的一方（执行口）调用。
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
/// `turn_exec::execute_turn` 与 `execute_final_reply_turn` 的落格窗口），且必须在
/// **读切片之前**——补落改的就是那份切片，本轮后面的 turn 号 / 续写锚 / 幂等键都从
/// 切片折，先落完才不会站在一份缺了那次编辑的状态上做判断。
///
/// ## 幂等
///
/// 逐条走 [`append_into`]，幂等键与就地落格完全一致（`{prefix}-{n}`）——因此
/// 「同一条变更被补落两次」（崩在删文件之前 ⇒ 下一次窗口再落一次）不会写出两条事实。
///
/// ## 为什么补落期间要重新算锚
///
/// 寄存时**不算锚**（见 [`ParkedChange`]）：锚必须按落格那一刻的切片算。
/// 补落发生在原编辑之后的某个执行口，此时切片已经长出了这一轮的事件——用编辑
/// 那一刻的锚会把事实指到一个**过期状态**上（I2 要的是「这次变更站在哪份状态上」，
/// 而那是**落格时刻**的状态）。
///
/// ## 顺序
///
/// 按文件名升序逐件落；**拿不到令牌就整队停下**（文件本来就在盘上，不需要重新寄存）。
/// 跳过它继续落后面的，等于把后寄存的变更排在先寄存的**之前**落格——折叠顺序一旦错
/// 开，「同一条消息编辑两次」就变成了倒放。
pub fn drain_parked(store: &EventWalStore, session_id: &str) {
    let dir = pending_dir(store.dir());
    for path in pending_listing(&dir) {
        if !is_parked(&path) {
            // `.staged` 半件：从没成功改名过 ⇒ 那次寄存没发生过，顺手清掉。
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            crate::plugin_warn!(
                "session",
                "[message] 寄存件读不出来，已放弃（{}）——该次变更丢失",
                path.display()
            );
            let _ = std::fs::remove_file(&path);
            continue;
        };
        let Ok(change) = serde_json::from_slice::<ParkedChange>(&bytes) else {
            crate::plugin_warn!(
                "session",
                "[message] 寄存件不是合法形状，已放弃（{}）——该次变更丢失",
                path.display()
            );
            let _ = std::fs::remove_file(&path);
            continue;
        };
        match append_into(
            store,
            session_id,
            &change.kind,
            change.verb,
            &change.id_prefix,
            &change.target,
            change.payload.clone(),
        ) {
            Ok(()) => {
                if let Err(e) = std::fs::remove_file(&path) {
                    // 已经落了，删不掉只是下一次多问一次幂等键（`Duplicate`），
                    // 不是正确性问题——故 warn 而不改判。
                    crate::plugin_warn!(
                        "session",
                        "[message] 寄存件已补落但删不掉（{}）：{e}",
                        path.display()
                    );
                }
            }
            // 令牌还没回来 ⇒ 整队停下，队列留在盘上等下一个窗口（见函数文档「顺序」）。
            Err(TryAppendError::NotWriter) => return,
            // 无锚：寄存救不了它（这个会话从未走过 v2），也不能让它永远堆在寄存区里。
            Err(TryAppendError::NoAnchor) => {
                crate::plugin_warn!(
                    "session",
                    "[message] 寄存的变更补落时无可用溯源锚，已放弃（{} {}）",
                    change.kind,
                    change.target
                );
                let _ = std::fs::remove_file(&path);
            }
            // 真失败：补落没有第二次机会（调用方早已把回执发给了用户），出声并出队。
            Err(TryAppendError::Failed(why)) => {
                crate::plugin_error!(
                    "session",
                    "[message] 寄存的变更补落失败，该次变更已丢失（重建后不可见）：{}",
                    why
                );
                let _ = std::fs::remove_file(&path);
            }
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
pub(crate) fn edited(store: &EventWalStore, session_id: &str, msg: &ChatNode) -> AppendOutcome {
    let payload = match serde_json::to_value(msg) {
        Ok(v) => v,
        // 序列化失败是**本模块自己的**失败（与事实源无关）：调用方同样不能当它没发生。
        Err(e) => {
            return AppendOutcome::Failed(format!("编辑事实未入格：序列化失败：{e}"));
        }
    };
    append(
        store,
        session_id,
        EVENT_MESSAGE_EDITED,
        Verb::Asserted,
        "m-edit",
        &msg.id,
        serde_json::json!({ "message": payload }),
    )
}

/// 一条消息**诞生** ⇒ 落一条 `message.added`（载荷 `{ messages: [...] }`）。
///
/// 只服务「**没有别的格可搭车**」的节点（压缩记录节点是最典型的一个）——承载着
/// 对话内容的节点（用户发言 / 助手正文 / 工具结果）必须由它们自己的事实落，
/// 否则同一条节点有两个 owner（D-049 理由 1）。
///
/// 空列表不落格：「没有节点诞生」不该在事实源上留下痕迹，与 [`removed`] 同一条判据。
pub(crate) fn added(store: &EventWalStore, session_id: &str, msgs: &[ChatNode]) -> AppendOutcome {
    if msgs.is_empty() {
        return AppendOutcome::Nothing;
    }
    let payload = match serde_json::to_value(msgs) {
        Ok(v) => v,
        Err(e) => {
            return AppendOutcome::Failed(format!("诞生事实未入格：序列化失败：{e}"));
        }
    };
    append(
        store,
        session_id,
        EVENT_MESSAGE_ADDED,
        Verb::Opened,
        "m-add",
        // 溯源锚取**第一条**：与 [`removed`] 同形——一次诞生的一个动作，锚在它身上。
        msgs.first().map(|m| m.id.as_str()).unwrap_or_default(),
        serde_json::json!({ "messages": payload }),
    )
}

/// 一段消息没了 ⇒ 落一条 `message.removed`（载荷 `{ ids: [...] }`）。
///
/// `ids` 是**被删节点 id 的完整列表**（与 `truncate_messages` 回执里那份同源）。
/// 空列表不落格：「什么都没删」不该在事实源上留下痕迹——与
/// `truncate_messages` 不发任何变更帧同一条判据。
pub(crate) fn removed(store: &EventWalStore, session_id: &str, ids: &[String]) -> AppendOutcome {
    if ids.is_empty() {
        return AppendOutcome::Nothing;
    }
    append(
        store,
        session_id,
        EVENT_MESSAGE_REMOVED,
        Verb::Closed,
        "m-del",
        // 溯源锚取**第一条被删的**：截断的语义是「从 X 起」——X 就是那个起点。
        ids.first().map(String::as_str).unwrap_or_default(),
        serde_json::json!({ "ids": ids }),
    )
}

/// 一条变更事实落格的**结果**。
///
/// 五种而不是三种：旧形态把「无锚可指」与「真失败」压进同一个 `Discarded`，
/// 于是「v1 遗留会话可以放过」和「主读已切 WAL、这次编辑会丢」用了同一条处置。
/// 切源之后这两件事的代价相反，必须分开。
#[must_use]
#[derive(Debug)]
pub(crate) enum AppendOutcome {
    /// 落了格。
    Landed,
    /// 本来就没有事实可落（空列表）——不是失败，也不该在事实源上留痕。
    Nothing,
    /// 拿不到写者令牌（= 另一个进程持着它）⇒ **寄存**到盘上，等持令牌方补落（ / ）。
    /// 回执按成功走，因为这条变更已经有副本了——寄存失败不在这一格，它冒泡成 `Failed`。
    Parked,
    /// 无锚可指：事实源里没有可用的溯源锚（v1 遗留会话 / 从未走过 v2 轮）。
    /// 这类会话的读侧本就回落 `messages.json`（D-049），缓存仍是真源 ⇒ 豁免。
    NoAnchor,
    /// 真失败：事实源打不开 / 追加被拒。主读已切 WAL ⇒ 这次变更在重建后会丢。
    Failed(String),
}

/// [`try_append`] / [`append_into`] 的失败形态——寄存与否由调用方决定。
enum TryAppendError {
    /// 拿不到写者令牌（可寄存后补落）。
    NotWriter,
    /// 切片里没有可用的溯源锚（v1 遗留会话；寄存也救不了）。
    NoAnchor,
    /// 事实源打不开 / 追加被拒（真失败）。
    Failed(String),
}

/// 把一条变更事实追加进会话的事实源；拿不到令牌时**寄存**（ 的入口）。
///
/// `target` = 本次变更作用在哪个节点上（用于解溯源锚）；`id_prefix` 是幂等键前缀。
fn append(
    store: &EventWalStore,
    session_id: &str,
    kind: &str,
    verb: Verb,
    id_prefix: &str,
    target: &str,
    payload: Value,
) -> AppendOutcome {
    match append_into(
        store,
        session_id,
        kind,
        verb,
        id_prefix,
        target,
        payload.clone(),
    ) {
        Ok(()) => AppendOutcome::Landed,
        // **寄存，而不是出声丢弃**：另一个进程正持着写者令牌，此刻来的
        // 编辑拿不到 ⇒ 静默少一条事实。切源之后那就是「编辑偶发不生效」且无人报错。
        // 把变更写进盘上寄存区，由持令牌的一方（执行口 `drain_parked`）补落。
        //
        // 载荷**原样寄存**：锚与幂等键都在补落那一刻重算（见 `drain_parked`）。
        // 寄存**写不进去**就是这条变更彻底没了 ⇒ 按真失败冒泡，而不是 `Parked`。
        Err(TryAppendError::NotWriter) => {
            let change = ParkedChange {
                kind: kind.to_string(),
                verb,
                id_prefix: id_prefix.to_string(),
                target: target.to_string(),
                payload,
            };
            match park(store.dir(), &change) {
                Ok(()) => AppendOutcome::Parked,
                Err(why) => {
                    AppendOutcome::Failed(format!("令牌被另一个进程持有，寄存又失败：{why}"))
                }
            }
        }
        Err(TryAppendError::NoAnchor) => AppendOutcome::NoAnchor,
        Err(TryAppendError::Failed(why)) => AppendOutcome::Failed(why),
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
    use crate::symbio_core::{PRINCIPAL_MAIN, PRINCIPAL_USER};

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

    // `principal` 兼容两种 wire 形态（b，[D-069 §决策 6]）：新形态是
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

/// 用**给定的**事实源实例落一条变更事实。
///
/// [`append`]（就地落格）与 [`drain_parked`]（补落）共用这一份实现——一种变更
/// 只有一个落格实现，寄存与否只是它外面的分支。`store` 由会话存储层按会话构造
/// 一次（D-084 §2），本模块不再自己拼路径 `open`。
fn append_into(
    store: &EventWalStore,
    session_id: &str,
    kind: &str,
    verb: Verb,
    id_prefix: &str,
    target: &str,
    payload: Value,
) -> Result<(), TryAppendError> {
    // 完整史（S2.7-b2）：
    // - **溯源锚**：被编辑的消息若来自压缩前（其定义事件在归档段里），切片里没有它
    //   ⇒ 退回 `last()` 的弱锚（`provenance` 的兜底）；
    // - **幂等键**：`n` = 本会话内 `Message` 实体的**全局**计数。只数没被取代的那段
    //   会让 `n` 从头数起 ⇒ `{prefix}-{n}` 与归档段里的 id 重号（归档后旧 id 已离开
    //   内存幂等键集合，故不报错，但全局 id 唯一性被打破）。
    let snapshot = store.range(Seq::new(0));
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
            "[message] 变更事实未入格（会话 {session_id}）：切片内没有可用的溯源锚（{kind} {target}）"
        );
        return Err(TryAppendError::NoAnchor);
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
        Ok(_) => Ok(()),
        // **令牌被占**是自己的一类（寄存后能救回来），不能被归进"失败"里
        // ——那正是 要消灭的静默丢弃。
        Err(AppendError::NotTheWriter) => Err(TryAppendError::NotWriter),
        // 真失败：主读已切 WAL，放过它 = 这次变更在重建后消失。
        // 文案只陈述**发生了什么**，不再替调用方判断「缓存是否已写成功」——
        // 就地落格发生在写缓存**之前**、补落发生在**之后**，同一句话对两条路径
        // 不可能都成立（ 修掉的那句假话）。
        Err(e) => Err(TryAppendError::Failed(format!(
            "追加被拒（{e:?}）：{kind} {target}（会话 {session_id}）"
        ))),
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
#[path = "message_events.test.rs"]
mod tests;
