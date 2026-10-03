//! `session/stats` —— v2 事实源的**只读读数口**。
//!
//! 背景：网格**只写不读**——SLO §1.2 的四列（时延 / 兜底 / 成本 / 断点）早就
//! 有投影算得出数，却没有任何生产出口能读到它们，于是这些投影只能挂
//! `#[allow(dead_code)]` 承认「接线未落地」（[04 §3.1 批③](../../../docs/plan/04-工程落地.md)）。
//! 本模块就是那一格的读侧出口：[plan/12 §3 批 0](../../../docs/plan/12-价值验收与基线埋点.md)
//! 的出口形态②（只读路由）。
//!
//! ## 三条纪律
//!
//! - **只调已有投影，不另写一份统计**（[plan/12 §4](../../../docs/plan/12-价值验收与基线埋点.md)）：
//!   本文件里没有分位数 / 占比 / 累计的算式——口径只活在 core 的投影里，出口
//!   只做「取数 + 排版」。否则同一份事实会长出两套口径，读数与投影静默漂移，
//!   而两边都自称是 SLO。**不变量同理**：清单只调 `check_all`，宽限与预算判据
//!   由 core 定（[04 §3.1 批④](../../../docs/plan/04-工程落地.md)——本处不复判）。
//! - **真·只读**：WAL 经 [`EventWalStore::open_readonly`] 打开——不创建文件、
//!   不截断撕裂尾行。截断是**写方**的恢复语义，读方顺手做会与正在落行的写方
//!   撞车（append 模式下截断后剩下的字节接在新 EOF ⇒ 那一行静默损坏）。
//! - **不命名 core 的视图类型**：core 的域模块是私有的（C-002：外部只写
//!   `symbio_core::<符号>`），插件侧因此只出现**根导出的函数名**，类型随返回值
//!   流动、在出口边界一次性序列化。这里没有 `SloLatencyView` 之类的名字，也
//!   不需要为类型加根导出。
//!
//! ## as-of 口径
//!
//! 读数取 `now = i64::MAX`：WAL 是**落盘**事实源，文件里每一条都已发生，没有
//! 「未来的事件」可滤——与 `projection/slo.test.rs` 的扫描口同一口径。

use std::path::Path;
use std::sync::Arc;

use serde::Serialize;

use crate::symbio_core::{
    check_all, checkpoint, cost_ledger, fallback_rate, slo_report, Budget, EventWalStore,
    PermissionMatrix, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginPayload, Seq, Store, VisScope, SESSION_ID,
};

use super::plugin::SessionPlugin;

/// 一个档位的一行读数：时延列与兜底列并排（同一档位的两类收益同处一行）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct TierRow {
    /// 档位名（`deep` / `fast` / `reflect` / `autonomous` / `unspecified`）。
    pub tier: String,
    /// 轮数——兜底率的**分母**（开轮的 `user.message` 计数）。
    pub turns: u64,
    /// 其中走了兜底的轮数（分子）。
    pub fallbacks: u64,
    /// 兜底率（口径在 `TierStats::rate`，本处不重算）。
    pub rate: f64,
    /// 成功轮实测时延的分位数（口径在 `TierLatency::percentile`，本处不重算）。
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    /// 时延样本数（兜底轮不计入时延样本——失败的耗时不是成功响应的耗时）。
    pub samples: usize,
}

/// 会话级读数（四列同源，[ADR-044](../../../docs/decisions/core.md)）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SessionStats {
    /// 会话 id。
    pub session_id: String,
    /// 事实源路径——**复算的证据链**：拿这条路径
    /// `open_readonly → range → apply` 必须与本读数逐字相等。
    pub wal: String,
    /// 事实源在不在。不在 = 这个会话还没跑过一轮，全零读数是**有据的零**，
    /// 不是错误（读方不该为了读而创建文件）。
    pub has_wal: bool,
    /// 按档位的时延 / 兜底两列。
    pub tiers: Vec<TierRow>,
    /// 成本台账（`cost_ledger` 视图原样序列化）。
    pub cost: serde_json::Value,
    /// 断点（`checkpoint` 视图原样序列化：`last_seq` / `event_count` /
    /// `kind_counts`——末两项就是「这一格里到底有多少事实」）。
    pub checkpoint: serde_json::Value,
    /// 不变量违规清单（`check_all` 五条：C1 `seq` 单调 / C2 每轮一条 final /
    /// C3 断言带溯源 / C4 未收束 / C5 超预算）。**空 = 五条全绿**；每条带
    /// `event_id` 与人话。宽限口径（尾轮在途放行、按声明档位取预算）在 core 的
    /// `check_all`，本处只取数——[04 §3.1 批④](../../../docs/plan/04-工程落地.md)
    /// 的「不变量进 CI/读侧」：e2e 断言的就是这一列。
    pub invariants: serde_json::Value,
}

/// `session/stats` 的请求体：**全部可选**——不传 = 本机默认（今天的行为）。
///
/// `principal` 是读方身份（[plan/01 §7](../../../docs/plan/01-核心架构.md) 读侧）：
/// 声明了才判可见域，没声明就不判定。**这里的选择权只在读方，可见域与属主
/// 不在载荷里**——请求方自己声明「我能看到什么」等于给自己授权（governance 模块
/// 文档点名的那条捷径），本字段只回答「我是谁」。
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub(crate) struct StatsRequest {
    /// 读方身份；缺席 / `null` ⇒ 不判定。
    #[serde(default)]
    pub principal: Option<String>,
}

/// 读一个会话的事实源，出四列读数 + 不变量清单。
///
/// 纯读：不写文件、不改网格、不碰会话存储（消息 / 转写）。
///
/// `viewer = Some(身份)` 时先按可见域取**可读切片**再出四列（**读什么由能看什么
/// 决定**，不是先算完再裁结果）；`None` = 本机默认，与判定引入之前逐字一致。
pub(crate) fn read(
    session_id: &str,
    wal: &Path,
    viewer: Option<&str>,
) -> Result<SessionStats, PluginError> {
    let has_wal = wal.exists();
    let store = EventWalStore::open_readonly(wal)
        .map_err(|e| PluginError::InternalError(format!("v2 WAL 读取失败：{e}")))?;
    let snapshot = store.range(Seq::new(0));

    // 读侧闸（[04 §3.1 批⑥](../../../docs/plan/04-工程落地.md)）：**读什么由能看
    // 什么决定**——四列与不变量列都只从「你看得见的事实」算，不先算完再裁结果。
    //
    // 属主 = 本机会话的属主（`SESSION_OWNER`，部署事实；`[会话] ≈ [线程]`，S4 的
    // thread 实体落地前用会话属主），可见域取 C10 缺省 `thread_private`。于是：
    // 属主本人 = 全量读数（与不声明时逐字一致）；非属主 / 矩阵外主体 = **空切片**
    // ⇒ 四列全零、不变量清单为空，与 `has_wal: true` 并排即可分辨「有源但不给你
    // 看」，而不是被读成「这里没有数」。
    let snapshot = match viewer {
        None => snapshot,
        Some(principal) => {
            let matrix: &PermissionMatrix = crate::authz::production_matrix();
            let may_read =
                matrix.can_see(principal, crate::authz::SESSION_OWNER, VisScope::default());
            if may_read {
                snapshot
            } else {
                Vec::new()
            }
        }
    };

    // 四列全部来自同一份事件切片（ADR-044：实测与判据同源）。
    let latency = slo_report()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    let fallback = fallback_rate()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    let cost = cost_ledger()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;
    let ck = checkpoint()
        .apply(&snapshot, i64::MAX, Budget::generous())
        .value;

    // 档位取两列的并集：只开轮没收束的档位（时延列无样本）与只有兜底样本的
    // 档位都要在表里出现——漏掉一档等于把「这一档没有数」报成「这一档不存在」。
    let mut names: Vec<String> = latency
        .by_tier
        .keys()
        .chain(fallback.by_tier.keys())
        .cloned()
        .collect();
    names.sort_unstable();
    names.dedup();

    let tiers = names
        .into_iter()
        .map(|t| {
            let l = latency.of(&t);
            let f = fallback.of(&t);
            TierRow {
                tier: l.tier.clone(),
                turns: f.turns,
                fallbacks: f.fallbacks,
                rate: f.rate(),
                p50: l.p50(),
                p95: l.p95(),
                p99: l.p99(),
                samples: l.count(),
            }
        })
        .collect();

    let cost = serde_json::to_value(&cost)
        .map_err(|e| PluginError::InternalError(format!("成本台账序列化失败：{e}")))?;
    let checkpoint = serde_json::to_value(&ck)
        .map_err(|e| PluginError::InternalError(format!("断点序列化失败：{e}")))?;
    let invariants = serde_json::to_value(check_all(&snapshot))
        .map_err(|e| PluginError::InternalError(format!("不变量清单序列化失败：{e}")))?;

    Ok(SessionStats {
        session_id: session_id.to_string(),
        wal: wal.display().to_string(),
        has_wal,
        tiers,
        cost,
        checkpoint,
        invariants,
    })
}

impl SessionPlugin {
    /// 路由 `session/stats` 的处理：由会话 id 派生事实源路径，交给 [`read`]。
    ///
    /// 与 `chat/send` / `chat/abort` 并列在路由表里，但性质不同——那两条是
    /// **编排 / 控制**，本条是**读数**：既不是数据 CRUD（CRUD 全走 VDFS，见
    /// `plugin.rs::route` 的分支注释），也不产生任何副作用。
    pub(crate) async fn handle_stats(
        &self,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sid = ctx.get(SESSION_ID).unwrap_or_default();
        if sid.is_empty() {
            return Err(PluginError::ValidationError(
                "session/stats 需要 metadata.session_id".into(),
            ));
        }
        let wal =
            super::paths::session_dir(&self.storage_dir(), &sid).join(super::paths::V2_WAL_FILE);
        // 载荷 = **读方身份声明**（可选）：不带 `principal` = 本机默认，与读侧闸
        // 引入之前逐字一致；带了却读不出身份（类型不对）⇒ 声明了却判不了 ⇒ 拒绝
        // 读数，不悄悄降级成「未声明」（fail-closed）。载荷整体缺席同理拒绝。
        let viewer: Option<String> = match ctx.payload::<serde_json::Value>()? {
            serde_json::Value::Null => None,
            v => {
                serde_json::from_value::<StatsRequest>(v)
                    .map_err(|e| {
                        PluginError::ValidationError(format!("session/stats 载荷不合法：{e}"))
                    })?
                    .principal
            }
        };
        let stats = read(&sid, &wal, viewer.as_deref())?;
        Ok(PluginPayload::new(&stats))
    }
}

#[cfg(test)]
#[path = "stats.test.rs"]
mod tests;
