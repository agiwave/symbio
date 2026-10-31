//! 长期记忆检索（名册 #6）——轮首把可见切片折成一份有次序的读数（DE-026 D-264）。
//!
//! ## 它原来在 `loop` 里长什么样
//!
//! 三份东西合在一处：`recall.rs`（一条 `Projection`：as-of / 排除式遗忘 / 标签过滤 /
//! 混合打分）、`recall_score.rs`（排序内核）、`memory_events.rs` 的 `recall_view`
//! （取快照 → 可见域收放 → 调投影 → 誊抄降级）与 `prompt_section`（排版）。
//! 每换一副检索策略就要改控制面一次，而「怎么折」本来是记忆能力自己的判据。
//!
//! 自 L2b 起这一整块住本包：
//!
//! ```text
//! 控制面 ──取快照、按可见域收放、按新近排好──▶ 可见切片（值）
//! 本包   ──submit_retriever!──────────────▶ inventory（编译期随二进制生效）
//! 宿主   ──retrieve_all(切片, 发言)────────▶ 读数 ──▶ 债键格 / 段渲染 / 收束值
//! ```
//!
//! ## 三件留在宿主的事（都不是「怎么折」）
//!
//! - **跨会话枚举**（`SessionStore::recall_snapshots`）与**逐条可见判定**
//!   （`thread_private` / `shared` / `public`）是隔离面（I-8）：判错的代价是泄漏。
//!   入参因此是**收放之后**的切片——本包不需要、也拿不到「哪个会话是谁的」；
//! - **降级读数去哪**：`Retrieval.degraded` 由宿主交给收束贡献与 `memory.recalled`，
//!   本包只负责把它**算对**（预算耗尽 ⇒ 标降级，不允许空且不标）；
//! - **段放不放、放哪一档**：优先级是宿主的预算判断（D-243）；措辞与字节上界在本包
//!   （[`section`]），因为它就是检索读数的排版口径。
//!
//! ## 扫描配额（原来由框架层执行，现在由本包自己执行）
//!
//! `Projection::apply` 的预算语义（D-105）有三条：配额取 `max(1)`、超窗只交**切片
//! 头部**、`degraded = 窗口 < 全长`。检索面不经过 `apply`，所以这三条在 [`Retrieval`]
//! 的实现里重算一遍——省掉就是「空且不标降级」，而 DE-036 §6 验收 2 要消灭的正是那个
//! 形态。
//!
//! ## 与声明视图 `memory.live` 的关系
//!
//! 检索在包内直接调 [`live`]；控制面的巩固按名字读同一副折叠的**第二种读法**
//! （`memory.live`，`eval = live::memory_live`）。一副折叠两个读者，不是两份实现。

pub(crate) mod decl;
pub(crate) mod live;
pub(crate) mod provider;
mod score;
mod section;

use symbio_plugin_api::retrieval::{Retrieval, RetrievalRequest, Retrieved, Retriever};

/// `recall` 检索的扫描配额（1 事件 = 1 个配额单位）：超配额 ⇒ `degraded` 的部分结果
/// （I3 在检索面上的形状），不装作扫完了。
///
/// 值沿用投影时代的口径（`RECALL_SCAN_QUOTA`）——换执行方不改数值：它封的是「一轮
/// 轮首最多扫多少条可见事实」，与扫描发生在框架层还是能力包里无关。
pub(crate) const RECALL_SCAN_QUOTA: usize = 500;

/// 长期记忆检索：一副折叠 + 一个排序内核，交一条 [`Retriever`]。
pub(crate) struct RecallRetrieval;

impl Retriever for RecallRetrieval {
    /// 合并次序的依据（多条检索按 owner 字典序拼接）。取插件 id，与上下文 provider
    /// 的 owner 同形：同一份能力的两个交出口用同一个名字。
    fn owner(&self) -> &'static str {
        "memory"
    }

    fn retrieve(&self, at: &RetrievalRequest<'_>) -> Option<Retrieval> {
        // 预算语义三条（D-105，见模块文档）：配额下限 1、只截尾、窗口短于全长即降级。
        let quota = RECALL_SCAN_QUOTA.max(1);
        let window = if at.events.len() > quota {
            &at.events[..quota]
        } else {
            at.events
        };
        let degraded = window.len() < at.events.len();

        let scored: Vec<score::ScoreEntry> = live::live(window, i64::MAX)
            .into_iter()
            .map(|e| score::ScoreEntry {
                content: e.content,
                tag: e.tag,
                ts: e.ts,
                seq: e.seq,
            })
            .collect();
        let entries: Vec<Retrieved> = score::rank(scored, at.query)
            .into_iter()
            .map(|s| Retrieved {
                seq: s.seq,
                content: s.content,
                tag: s.tag,
            })
            .collect();
        // 空 ⇒ `None`：与「本轮没有召回」同一个形状（`retrieve_all` 的合并口径）。
        // 注意此时 `degraded` 也随之消失——与投影时代的 `recall_view` 逐字一致
        // （那边也是空条目即 `None`）。
        (!entries.is_empty()).then_some(Retrieval { degraded, entries })
    }
}

// 装配口的提交：宿主链接本 crate 即生效（与上下文 provider、声明表同一个形状）。
symbio_plugin_api::submit_retriever!(RecallRetrieval);

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
