//! 读数的**口径装配**：把 core 投影算出来的值排成出口 JSON 的每一格。
//!
//! 从 `stats.rs` 分出来的判据是「这一段错了谁会红」：本文件出错，红的是
//! **某一格的数字**——内容侧可见域判宽了 / 承诺列的违约率与 core 的配对规则不符 /
//! 实例级聚合少并了一份快照 / 面板里的数与它描述的列对不上。它不碰存储、不碰路由
//! （那些在 `stats.rs`），因此两处的红脸不同：那边红「读不到」，这边红「读错了」。
//!
//! 三条纪律里最硬的一条在这里落地：**只调已有投影，不另写一份统计**
//! （[PL-010 §4](../../../../docs/004-plan/010-价值验收与基线埋点.md "PL-010-价值验收与基线埋点")）——
//! 本文件里没有分位数 / 占比 / 累计的算式，只有取数、合并（`*View::merge`）与排版
//! （起标签、定单位、挑语气）。同一份事实长出两套口径，读数与投影就会静默漂移，
//! 而两边都自称是 SLO。

use crate::symbio_core::{fallback_rate, slo_report, Event, ReadingPanel, VisScope};

use super::stats::SessionStats;

/// 本会话事实源的**内容侧可见域**（T-094 ②）：记忆载荷 `vis_scope` 的最宽取值。
///
/// 为什么取最宽而不是逐条判：读侧闸是**整列**判据（四列 / 声誉 / 就绪 / 转写
/// 共用同一个答案，见 `read` 的注释），而这份事实源里只要有共享内容，它就不是
/// 纯私有的——「不给你看」必须因为**每一条都私有**，不是因为其中一条私有。
/// 反过来取最窄会让共享内容被私有内容拖着一起锁死，那是把产地的宽度读丢了。
///
/// 缺省（没有任何记忆事件 / 存量事件没有这个字段）= `thread_private`（C10）——
/// 与 T-094 之前的接线逐字一致，存量事实源不因这次接线改变读数。
pub(super) fn content_scope_of(snapshot: &[crate::symbio_core::Event]) -> VisScope {
    let mut widest = VisScope::ThreadPrivate;
    for e in snapshot {
        if e.entity != crate::symbio_core::Entity::Memory {
            continue;
        }
        // 取值口径在 core（`VisScope::of_event`：缺字段 / 认不出 = 最窄，C10）；
        // 本函数只负责「一批里取最宽」这一件合并判据（`widen` 也是 core 的）。
        widest = widest.widen(VisScope::of_event(e));
    }
    widest
}

/// 承诺列：立约记录 + 三个计数 + 违约率。
///
/// 记录本身由 core 翻译（[`crate::symbio_core::commitment_records`]——配对规则在写侧
/// 那一族里，本处不另发明一套）；这里只做**取数与排版**，与 `slo_report` /
/// `reputation` 各列同一纪律。率的分母判据（已了结数）写在列定义里，不重复第二遍。
pub(super) fn commitments_reading(snapshot: &[Event]) -> serde_json::Value {
    let items = crate::symbio_core::commitment_records(snapshot);
    let kept = items.iter().filter(|r| r.state == "kept").count();
    let broken = items.iter().filter(|r| r.state == "broken").count();
    let settled = kept + broken;
    let breach_rate = if settled == 0 {
        0.0
    } else {
        broken as f64 / settled as f64
    };
    serde_json::json!({
        "items": items,
        "total": items.len(),
        "kept": kept,
        "broken": broken,
        "settled": settled,
        "breach_rate": breach_rate,
    })
}

/// 实例级聚合列（P2）：把存储层交回的**全部会话快照**合并成一份读数。
///
/// ## 为什么在存储层遍历、在 core 侧合并
///
/// 「会话目录长什么样、哪些事实源打得开」是存储层的布局知识
/// （`SessionStore::deep_snapshots`：全会话含嵌套子会话，单个坏源跳过并出声）；
/// 而「多个视图怎么合成一个」是统计口径——后者必须单点，故进 core 的
/// `SloLatencyView::merge` / `FallbackRateView::merge`。本函数只做
/// 「套投影 + 交出去合并」，不碰路径、不开文件。
///
/// ## 遍历的形状
///
/// **全会话**：存储层的深遍覆盖顶层各一份 WAL 与嵌套子会话（分形委派整棵子树）。
/// 子会话不是「别的实例」，文档上一行说着「**所有**会话」——只扫一层会让那句话说假话。
/// 打不开的快照已在存储层被跳过（出声一次）；本列只数交到手里的值。
pub(super) fn fleet_reading(snapshots: &[Vec<Event>]) -> serde_json::Value {
    let sessions = snapshots.len();

    let mut lat_parts = Vec::new();
    let mut fb_parts = Vec::new();
    // 外部效标两族（02 §3.1 · 主动性 / 时域跨度）：与 lat/fb 同批扫描、同一份
    // snapshot——「多扫一遍」不是更彻底，是多一次读错范围的机会。
    let mut con_parts = Vec::new();
    let mut span_parts = Vec::new();
    let mut aut_parts = Vec::new();
    for snapshot in snapshots {
        // SLO 四列是**累计口径**（S2.7-b2）：被快照取代而移入归档段的前缀也在
        // `range(0)` 里，实例聚合不会**静默**少算。
        lat_parts.push(slo_report().read_all(snapshot).value);
        fb_parts.push(fallback_rate().read_all(snapshot).value);
        con_parts.push(crate::symbio_core::conation().read_all(snapshot).value);
        span_parts.push(crate::symbio_core::span().read_all(snapshot).value);
        aut_parts.push(crate::symbio_core::autonomy().read_all(snapshot).value);
    }

    let latency = crate::symbio_core::SloLatencyView::merge(&lat_parts);
    let fallback = crate::symbio_core::FallbackRateView::merge(&fb_parts);
    let turns: u64 = fallback.by_tier.values().map(|s| s.turns).sum();
    let fallbacks: u64 = fallback.by_tier.values().map(|s| s.fallbacks).sum();
    // 两族的跨会话合并都在 core（统计口径单点）：分子分母可加、率是同一套算式的
    // 再应用；span 的「一个事件的会话不进分母」也在 core 判，不在这边挑数据。
    let conation = crate::symbio_core::ConationView::merge(&con_parts);
    let span = crate::symbio_core::SpanView::merge(&span_parts);
    let autonomy = crate::symbio_core::AutonomyView::merge(&aut_parts);

    serde_json::json!({
        // 交到手里的快照数（打不开的已在存储层跳过并出声）
        "wals": lat_parts.len(),
        // 与 `wals` 同值：键是出口形状的一部分（e2e 对账读它）；
        // 「跳过了几个」由存储层的告警说明，不在这里自行枚举
        "sessions": sessions,
        "turns": turns,
        "fallbacks": fallbacks,
        // 逐档位并集：合并后由 `of()` / 分位数纯查询取，与单会话列同一套口径
        "by_tier": {
            "latency": latency.by_tier.values().map(|t| serde_json::json!({
                "tier": t.tier, "samples": t.samples.len(),
                "p50": t.p50(), "p95": t.p95(), "p99": t.p99(),
            })).collect::<Vec<_>>(),
            "fallback": fallback.by_tier.values().map(|s| serde_json::json!({
                "tier": s.tier, "turns": s.turns, "fallbacks": s.fallbacks,
                "rate": s.rate(),
            })).collect::<Vec<_>>(),
        },
        // 外部效标两族（02 §3.1）：与 by_tier 同地位——同一个实例、同一批 WAL、
        // 同一批投影，只是问的问题不同（「哪条轴被抬高了」而不是「哪档跑得好」）。
        // 键名稳定（不随文档改字号）：conation ↔ 主动性、span ↔ 时域跨度。
        "axes": {
            "conation": {
                "expressed": conation.expressed,
                "escalated": conation.escalated,
                "adoption_rate": conation.adoption_rate(),
            },
            "span": {
                "sessions": span.sessions,
                "median_ms": span.median_ms(),
            },
            "autonomy": {
                "closed": autonomy.closed,
                "unattended": autonomy.unattended,
                "unattended_rate": autonomy.unattended_rate(),
            },
        },
    })
}

/// 从本函数的真实数据生成面板描述（`session/stats` 的呈现，concern 在**产者**）。
///
/// ## 只吃 `SessionStats` 的**其它字段**，一目了然、无法漂移
///
/// 签名故意收 `&SessionStats`：生成过程读到的每个数都是从**同一份数据**上算出来的，
/// 不存在「另算一遍」的可能。反过来若在这里重新扫 WAL 或重算分位数，两份口径就
/// 可以各自漂移——而判据 `panel_matches_the_columns_it_describes` 只看结果，
/// 看不住过程。写在这里的转换只有三类：起标签、定单位、挑语气。
pub(super) fn panel_of(stats: &SessionStats) -> ReadingPanel {
    use crate::symbio_core::{ReadingRow, ReadingSection, ReadingTone};

    // 档位列 → 一段「时延与兜底」。tiers 已按档位排序，此处原序输出（顺序是
    // 数据给的事实，不是排版选择）。
    let perf = ReadingSection {
        title: Some("时延与兜底".into()),
        rows: stats
            .tiers
            .iter()
            .map(|t| ReadingRow {
                label: format!("{} 档", t.tier),
                value: format!("{} 轮 · 兜底率 {:.0}%", t.turns, t.rate * 100.0),
                hint: Some(format!(
                    "P50 {}ms / P95 {}ms / P99 {}ms（{} 个成功样本）",
                    t.p50, t.p95, t.p99, t.samples
                )),
                tone: if t.rate > 0.05 {
                    ReadingTone::Warn
                } else {
                    ReadingTone::Normal
                },
            })
            .collect(),
    };

    // 断点列：事实条数 / 事实源在不在，再加一行种类分布。
    let ck = &stats.checkpoint;
    let events = ck.get("event_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let kinds = ck.get("kind_counts").and_then(|v| v.as_object());
    let last = ck.get("last_seq").and_then(|v| v.as_u64());
    let mut continuity_rows = vec![
        ReadingRow {
            label: "已记录事实".into(),
            value: format!("{events} 条"),
            hint: Some(format!(
                "末条 seq = {}",
                last.map(|s| s.to_string()).unwrap_or_else(|| "—".into())
            )),
            tone: ReadingTone::Normal,
        },
        ReadingRow {
            label: "事实源在不在".into(),
            value: if stats.has_wal {
                "在".into()
            } else {
                "不在".into()
            },
            hint: Some("不在 = 这个会话还没跑过一轮，其余各列是有据的零".into()),
            tone: if stats.has_wal {
                ReadingTone::Normal
            } else {
                ReadingTone::Warn
            },
        },
    ];
    // 事实种类的分布（BTreeMap 序列化 ⇒ 有序）。只列前 6 种：全部铺开是把
    // 「概览」变成清单，而清单归各列自己的读法。
    if let Some(kinds) = kinds {
        continuity_rows.push(ReadingRow {
            label: "事实种类".into(),
            value: kinds
                .iter()
                .take(6)
                .map(|(k, v)| format!("{k} × {}", v.as_u64().unwrap_or(0)))
                .collect::<Vec<_>>()
                .join(" · "),
            hint: Some("按事件 kind 计数（本面板只列前 6 种）".into()),
            tone: ReadingTone::Normal,
        });
    }
    let continuity = ReadingSection {
        title: Some("事实源".into()),
        rows: continuity_rows,
    };

    // 不变量：违规清单为空 ⇒ 一句「全绿」；非空 ⇒ 一条一行、语气需留意。
    let violations = stats.invariants.as_array().map(|a| a.len()).unwrap_or(0);
    let invariants = ReadingSection {
        title: Some("不变量".into()),
        rows: vec![ReadingRow {
            label: "违规".into(),
            value: if violations == 0 {
                "0 条".into()
            } else {
                format!("{violations} 条")
            },
            hint: Some(
                "seq 单调 / 每轮一条开轮与收束 / 断言带溯源 / 未收束 / 超预算 / 依赖环 / 返工界"
                    .into(),
            ),
            tone: if violations == 0 {
                ReadingTone::Normal
            } else {
                ReadingTone::Warn
            },
        }],
    };

    // 技能：开关与已编译数并排——两种「零技能」靠这一行分辨（关着 = 配置选择，
    // 开着但 0 条 = 本会话还没产出可编译的轨迹）。
    let skills = ReadingSection {
        title: Some("技能".into()),
        rows: vec![ReadingRow {
            label: "编译开关".into(),
            value: if stats.skill_compile_enabled {
                "开".into()
            } else {
                "关".into()
            },
            hint: Some(if stats.skill_compile_enabled {
                format!("本会话已编译 {} 条技能", stats.compiled_skills)
            } else {
                "关着 ⇒ 本轮轨迹不会编译成技能（与「开着但没编译过」不同）".into()
            }),
            tone: ReadingTone::Normal,
        }],
    };

    // 承诺：一条一行读的是清单，面板只报「几条约、几条约没守住」——违约必须
    // 当着人的面出声（S08 §5），所以有违约就是留意语气，没有才是常态。
    let cm = &stats.commitments;
    let cm_total = cm.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
    let cm_broken = cm.get("broken").and_then(|v| v.as_u64()).unwrap_or(0);
    let commitments = ReadingSection {
        title: Some("承诺".into()),
        rows: vec![ReadingRow {
            label: "代际立约".into(),
            value: if cm_total == 0 {
                "本会话没有委托".into()
            } else {
                format!("{} 条 · 违约 {} 条", cm_total, cm_broken)
            },
            hint: Some(format!(
                "违约率 {:.0}%（分母 = 已了结数，在途的约不算违约）",
                cm.get("breach_rate")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0)
                    * 100.0
            )),
            tone: if cm_broken > 0 {
                ReadingTone::Warn
            } else {
                ReadingTone::Normal
            },
        }],
    };

    ReadingPanel {
        title: "会话读数".into(),
        subtitle: Some(stats.session_id.clone()),
        sections: vec![perf, continuity, invariants, skills, commitments],
    }
}
