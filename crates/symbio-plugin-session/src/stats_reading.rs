//! 读数的**口径装配**：把 core 投影算出来的值排成出口 JSON 的每一格。
//!
//! 从 `stats.rs` 分出来的判据是「这一段错了谁会红」：本文件出错，红的是
//! **某一格的数字**——内容侧可见域判宽了 / 承诺列的违约率与 core 的配对规则不符 /
//! 实例级聚合少并了一份快照 / 面板里的数与它描述的列对不上。它不碰存储、不碰路由
//! （那些在 `stats.rs`），因此两处的红脸不同：那边红「读不到」，这边红「读错了」。
//!
//! 两条出口都在这里，因为它们要的是同一条纪律：
//! - **`session/stats` 路由**：一份 [`ReadingPanel`]（`panel_of` 起）；
//! - **消息节点交付**：一格 [`ReadingSection`]（[`present_message`] / [`notice_stats_of`]，
//!   [D-111](README.md#d-111-消息读数由交付出口装配压缩与记忆的事实行离开前端 "crates/symbio-plugin-session/src/README")）——压缩与记忆那几条事实行原先由前端
//!   读 `meta` 数字自己拼，口径住在两端；现在水位、触发词、token 单位都只在本文件出现一次。
//!
//! 三条纪律里最硬的一条在这里落地：**只调已有投影，不另写一份统计**
//! （[DE-016 §4](../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）——
//! 本文件里没有分位数 / 占比 / 累计的算式，只有取数、合并（`*View::merge`）与排版
//! （贴标签、定单位、挑语气）。同一份事实长出两套口径，读数与投影就会静默漂移，
//! 而两边都自称是 SLO。

use crate::declarations::{CHAT_VOCAB, COMMITMENT_VOCAB};
use symbio_core::chat_node::ChatNodePresentationExt;
use symbio_core::{
    chat_node as cm, Event, ReadingBar, ReadingPanel, ReadingRow, ReadingSection, ReadingTone,
    VisScope,
};

use crate::projections::{
    fallback::{fallback_rate, FallbackRateView},
    slo::{slo_report, SloLatencyView},
    span::{span, SpanView},
};

use super::stats::SessionStats;

/// 读侧出口的**读方身份声明**：`session/stats` 与 `session/changes` 同一份载荷形状。
///
/// 全部可选——不传 = 本机默认。`principal` 只回答「我是谁」；**可见域与属主不在
/// 载荷里**（请求方自己声明「我能看到什么」等于给自己授权）。声明了却读不出身份
/// ⇒ 出口拒绝（fail-closed），判据落在各出口的解析处。
#[derive(Debug, Default, Clone, serde::Deserialize)]
pub(super) struct ViewerRequest {
    /// 读方身份；缺席 / `null` ⇒ 不判定。
    #[serde(default)]
    pub principal: Option<String>,
}

/// 本会话事实源的**内容侧可见域**（②）：记忆载荷 `vis_scope` 的最宽取值。
///
/// 为什么取最宽而不是逐条判：读侧闸是**整列**判据（四列 / 声誉 / 就绪 / 转写
/// 共用同一个答案，见 `read` 的注释），而这份事实源里只要有共享内容，它就不是
/// 纯私有的——「不给你看」必须因为**每一条都私有**，不是因为其中一条私有。
/// 反过来取最窄会让共享内容被私有内容拖着一起锁死，那是把产地的宽度读丢了。
///
/// 缺省（没有任何记忆事件 / 存量事件没有这个字段）= `thread_private`（C10）——
/// 与旧接线逐字一致，存量事实源不因这次接线改变读数。
pub(super) fn content_scope_of(snapshot: &[symbio_core::Event]) -> VisScope {
    let mut widest = VisScope::ThreadPrivate;
    for e in snapshot {
        if e.entity != symbio_core::Entity::Memory {
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
/// 记录本身由 core 翻译（[`symbio_core::commitment_records`]——配对规则在写侧
/// 那一族里，本处不另发明一套）；这里只做**取数与排版**，与 `slo_report` /
/// `reputation` 各列同一纪律。率的分母判据（已了结数）写在列定义里，不重复第二遍。
pub(super) fn commitments_reading(snapshot: &[Event]) -> serde_json::Value {
    let items = symbio_core::commitment_records(snapshot, &COMMITMENT_VOCAB);
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

/// 实例级聚合列：把存储层交回的**全部会话快照**合并成一份读数。
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
    // 外部效标两族（DE-008 §3.1.1 · 主动性 / 时域跨度）：与 lat/fb 同批扫描、同一份
    // snapshot——「多扫一遍」不是更彻底，是多一次读错范围的机会。
    let mut con_parts = Vec::new();
    let mut span_parts = Vec::new();
    let mut aut_parts = Vec::new();
    for snapshot in snapshots {
        // SLO 四列是**累计口径**：被快照取代而移入归档段的前缀也在
        // `range(0)` 里，实例聚合不会**静默**少算。
        lat_parts.push(slo_report(CHAT_VOCAB).read_all(snapshot).value);
        fb_parts.push(fallback_rate(CHAT_VOCAB).read_all(snapshot).value);
        con_parts.push(crate::conation::conation().read_all(snapshot).value);
        span_parts.push(span().read_all(snapshot).value);
        aut_parts.push(
            crate::autonomy::autonomy_for(crate::declarations::TASK_VOCAB)
                .read_all(snapshot)
                .value,
        );
    }

    let latency = SloLatencyView::merge(&lat_parts);
    let fallback = FallbackRateView::merge(&fb_parts);
    let turns: u64 = fallback.by_tier.values().map(|s| s.turns).sum();
    let fallbacks: u64 = fallback.by_tier.values().map(|s| s.fallbacks).sum();
    // 跨会话合并是**统计口径单点**：分子分母可加、率是同一套算式的再应用。
    // `span` 的「一个事件的会话不进分母」在 core 判，不在这边挑数据；`conation` 与
    // `autonomy` 的合并住本插件（[`crate::conation`] / [`crate::autonomy`]）——这两条读数的
    // 读写两端都在这里，投影因此不是共享类型（DE-024 D-200 / D-201）。
    let conation = crate::conation::ConationView::merge(&con_parts);
    let span = SpanView::merge(&span_parts);
    let autonomy = crate::autonomy::AutonomyView::merge(&aut_parts);

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
        // 外部效标两族（DE-008 §3.1.1）：与 by_tier 同地位——同一个实例、同一批 WAL、
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

/// 能力视图列：遍历**公开清单**，逐条经 `symbio_plugin_api::views` 求值（DE-025 D-221）。
///
/// ## 为什么这里不认识任何视图名
///
/// 名字、版本、扫描成本与折叠都在 owner 的 `ViewDecl` 里；本函数只做「按清单取、
/// 原样装一格」。于是加一项能力视图 = owner 声明 + `PLUGIN.yml` 公开一行，本文件
/// 不动——这正是「横向扩展不改架构」在本列上的落点。读不到时给**可诊断的缺席**
/// （`reason` + `detail`），不给空值冒充读数。
pub(super) fn capability_views(snapshot: &[Event]) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for shared in symbio_plugin_api::shared_views() {
        let entry = match symbio_plugin_api::ViewRead::named(shared.name) {
            Ok(view) => match (view.ver(), view.read_all(snapshot)) {
                (Ok(ver), Ok(value)) => serde_json::json!({
                    "owner": shared.owner,
                    "status": "ok",
                    "ver": ver,
                    "value": value,
                }),
                (Err(why), _) | (_, Err(why)) => view_unavailable(shared.owner, &why),
            },
            Err(why) => view_unavailable(shared.owner, &why),
        };
        out.insert(shared.name.to_string(), entry);
    }
    serde_json::Value::Object(out)
}

/// 一格缺席：原因词（稳定拼写）+ 人话细节。两者都要——前者给分支，后者给定位。
fn view_unavailable(owner: &str, why: &symbio_plugin_api::ViewUnavailable) -> serde_json::Value {
    serde_json::json!({
        "owner": owner,
        "status": "unavailable",
        "reason": why.reason(),
        "detail": why.to_string(),
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
        bar: None,
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
        bar: None,
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
        bar: None,
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
        bar: None,
    };

    // 承诺：一条一行读的是清单，面板只报「几条约、几条约没守住」——违约必须
    // 当着人的面出声（DE-038 §5），所以有违约就是留意语气，没有才是常态。
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
        bar: None,
    };

    ReadingPanel {
        title: "会话读数".into(),
        subtitle: Some(stats.session_id.clone()),
        sections: match (usage_section(stats), capability_section(stats)) {
            (Some(usage), Some(capability)) => {
                vec![
                    perf,
                    usage,
                    continuity,
                    invariants,
                    skills,
                    commitments,
                    capability,
                ]
            }
            (Some(usage), None) => vec![perf, usage, continuity, invariants, skills, commitments],
            (None, Some(capability)) => {
                vec![
                    perf,
                    continuity,
                    invariants,
                    skills,
                    commitments,
                    capability,
                ]
            }
            (None, None) => vec![perf, continuity, invariants, skills, commitments],
        },
    }
}

/// 能力视图那一段（DE-025 D-221）：**清单有几条，段里就有几行**。
///
/// 一行代表一条公开视图，值只有两类话：「可读 · vN」或「读不出来（原因词）」。
/// 语义属于 owner，本段不解释载荷——它把完整值留在 `capability_views` 列里，
/// 这里只把「这一格有没有数」说出来。清单为空 ⇒ 整段不出现（不是「0 条视图」）。
///
/// 预览截到 80 个字符并明说截了：面板是概览，全量在那一列——截断在标签上说清楚，
/// 不让人把省略号后面的内容当成没有。
fn capability_section(stats: &SessionStats) -> Option<ReadingSection> {
    let views = stats.capability_views.as_object()?;
    if views.is_empty() {
        return None;
    }
    let rows = views
        .iter()
        .map(|(name, entry)| {
            let owner = entry.get("owner").and_then(|v| v.as_str()).unwrap_or("");
            let readable = entry.get("status").and_then(|v| v.as_str()) == Some("ok");
            let (value, hint, tone) = if readable {
                let ver = entry.get("ver").and_then(|v| v.as_u64()).unwrap_or(0);
                (
                    format!("可读 · v{ver}"),
                    Some(format!(
                        "owner `{owner}` 的折叠；全量在 capability_views 列：{}",
                        preview(&entry["value"])
                    )),
                    ReadingTone::Normal,
                )
            } else {
                (
                    format!(
                        "读不出来 · {}",
                        entry
                            .get("reason")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                    ),
                    entry
                        .get("detail")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    ReadingTone::Warn,
                )
            };
            ReadingRow {
                label: name.clone(),
                value,
                hint,
                tone,
            }
        })
        .collect();
    Some(ReadingSection {
        title: Some("能力视图".into()),
        rows,
        bar: None,
    })
}

/// 一行里的 JSON 预览：紧凑序列化后按**字符**截到 80，并在文案里明说截断了。
///
/// 按字符而不是字节：便签正文是中文，按字节切会把一个汉字劈成两半。
fn preview(value: &serde_json::Value) -> String {
    const KEEP: usize = 80;
    let text = value.to_string();
    let mut out: String = text.chars().take(KEEP).collect();
    if text.chars().count() > KEEP {
        out.push_str("…（预览截断）");
    }
    out
}

/// 用量那一段（token + 钱，两者同属「这一次往返测到了什么」）。
///
/// ## 为什么「一次实测都没有」时整段不出现
///
/// `calls` 是「带实测用量的应答条数」。为 0 意味着 provider 一次都没报 `usage`
/// （桩 / 自建端点 / 协议不给），此时写「输入 0 / 输出 0」就是在编一个数——
/// 与 `context_watermark` 缺数整条不出现、`bar` 没有分母就不发是同一条纪律
/// （[DE-020](../../../docs/003-design/020-前端UI-UX.md "DE-020-前端UI-UX") §11 判据 4）。
///
/// 钱的行**只在有账时出现**：价格是 Provider 配置，没配就没有金额可报。
/// 它缺的是「定价」不是「测量」，故不拦着 token 那三行照常出现；反过来
/// token 缺时也不该凭空出现一行「0 元」。
///
/// ## 钱为什么按币种分行
///
/// 金额只在同一币种内可加（兑换率不是事实源里的东西）。一个币种一行，
/// 行数由账本的币种数决定；币种顺序由 `spend_ledger` 的 BTreeMap 给。
pub(super) fn usage_section(stats: &SessionStats) -> Option<ReadingSection> {
    let t = &stats.tokens;
    let calls = t.get("calls").and_then(|v| v.as_u64()).unwrap_or(0);
    let mut rows = Vec::new();

    // token 三行只在有实测时出现：`calls == 0` 写「输入 0 / 输出 0」就是编一个数。
    if calls > 0 {
        let num = |k: &str| t.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
        let input = num("input");
        let output = num("output");
        rows.push(ReadingRow {
            label: "输入".into(),
            value: format_tokens(input),
            hint: Some(format!("{calls} 次模型往返的实测合计")),
            tone: ReadingTone::Normal,
        });
        rows.push(ReadingRow {
            label: "输出".into(),
            value: format_tokens(output),
            hint: None,
            tone: ReadingTone::Normal,
        });
        rows.push(ReadingRow {
            label: "合计".into(),
            value: format_tokens(input + output),
            hint: Some("输入 + 输出；不含缓存命中与思考的单独计价".into()),
            tone: ReadingTone::Normal,
        });
        // 多主体（子智能体 / 委派）才逐条摊开；单主体时它上面的合计就是全部。
        if let Some(by) = t.get("by_principal").and_then(|v| v.as_object()) {
            if by.len() > 1 {
                rows.push(ReadingRow {
                    label: "按主体".into(),
                    value: by
                        .iter()
                        .take(4)
                        .map(|(p, e)| {
                            let i = e.get("input").and_then(|v| v.as_u64()).unwrap_or(0);
                            let o = e.get("output").and_then(|v| v.as_u64()).unwrap_or(0);
                            format!("{p} {}", format_tokens(i + o))
                        })
                        .collect::<Vec<_>>()
                        .join(" · "),
                    hint: Some("按内容的归属主体（不是写方）；本面板只列前 4 个".into()),
                    tone: ReadingTone::Normal,
                });
            }
        }
    }

    // 钱：一个币种一行。`by_currency` 为空 ⇒ 一行都没有（不是「0 元」）；
    // 币种不合计（没有汇率，跨币种求和是编一个数）。顺序由视图的 BTreeMap 给。
    let mut has_money = false;
    if let Some(by_currency) = stats.spend.get("by_currency").and_then(|v| v.as_object()) {
        for bucket in by_currency.values() {
            let currency = bucket
                .get("currency")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if currency.is_empty() {
                continue;
            }
            let amount = bucket
                .get("amount_nano")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let charges = bucket.get("charges").and_then(|v| v.as_u64()).unwrap_or(0);
            has_money = true;
            rows.push(ReadingRow {
                label: format!("花费 · {currency}"),
                value: format_money(amount),
                hint: Some(format!(
                    "{charges} 次带定价的往返合计（单价来自 Provider 配置，不是按模型名猜的）"
                )),
                tone: ReadingTone::Normal,
            });
        }
    }

    if rows.is_empty() {
        return None;
    }
    Some(ReadingSection {
        // 只有 token 时标题说「用量」；有钱的行才把「花费」也说出来——标题不
        // 预告没有的行。
        title: Some(
            if has_money {
                "用量与花费"
            } else {
                "用量"
            }
            .into(),
        ),
        rows,
        bar: None,
    })
}

// ==================== 消息节点的读数（D-111） ====================

/// 会话把消息节点**交给前端**那一刻的呈现装配：core 的形状派生（`title`/`icon`/`actions`）
/// 之后补上本域的读数行。
///
/// 三条交付出口（转写列表、会话叶子读、实时帧投递）都过这里，因此「消息上那几个数怎么说」
/// 在整条链路上只有一处口径。存储与事件里的节点**永不含** `stats`——呈现不是事实，
/// 改一次文案不需要迁移一份历史，而存量会话重新读到就带行（零迁移）。
pub(crate) fn present_message(mut m: cm::ChatNode) -> cm::ChatNode {
    if m.stats.is_none() {
        m.stats = notice_stats_of(&m);
    }
    m.with_derived_presentation()
}

/// token 数的紧凑写法（`128000` → `128k`、`950` → `950`）——**全仓唯一的一处**。
///
/// 单位与精度归产者：同一条 `after_tokens` 若在两处各自格式化，早晚一个显示 `44.5k`
/// 一个显示 `45k`，而两个数字都自称是同一个水位。
fn format_tokens(n: u64) -> String {
    if n < 1000 {
        return n.to_string();
    }
    let k = n as f64 / 1000.0;
    // ≥100k 取整（`200k` 比 `200.3k` 更像「快满了」要传达的量级），否则留一位小数。
    let rounded = if k >= 100.0 {
        format!("{}", k.round())
    } else {
        format!("{}", (k * 10.0).round() / 10.0)
    };
    format!("{rounded}k")
}

/// 金额的写法：整数纳单位 → 至少两位小数、最多四位（`0.0123` / `1.20` / `0.00`）。
///
/// 与 [`format_tokens`] 同一条纪律：单位与精度只在产者定一次。**纳单位不进界面**——
/// 那是事实的形状，不是读数的形状；显示到 1e-4 是因为小额往返（一次几百 token
/// 按每百万计价）常见到 1e-4 量级，再往下一位只会把「这一轮几乎没花钱」写成
/// 一长串噪声。非零但小于半格时写 `<0.0001`：它既不是零，也不该四舍五入成零
/// ——与「没定价」由行在不在分辨是同一条「不补数」纪律。
///
/// 币种不进本函数：它由行标签说（`花费 · USD`），金额只负责数字本身。
fn format_money(amount_nano: u64) -> String {
    if amount_nano == 0 {
        return "0.00".into();
    }
    if amount_nano < 50_000 {
        return "<0.0001".into();
    }
    // 四舍五入到 1e-4 后的整数格数（1e-4 = 100_000 纳）；饱和加法防溢出。
    let units = amount_nano.saturating_add(50_000) / 100_000;
    let whole = units / 10_000;
    let frac = units % 10_000;
    if frac == 0 {
        format!("{whole}.00")
    } else if frac.is_multiple_of(100) {
        format!("{whole}.{:02}", frac / 100)
    } else {
        format!("{whole}.{frac:04}")
    }
}

/// 触发来源 → 面向用户的词。
///
/// 词表住在这里而不是前端的 `COMPACTION_TRIGGER_LABELS`：取值集合是后端的协议词
/// （`compression_stats` 的入参），前端那份是抄本。**未知取值原样透出**——那是
/// 「协议加了新档位而这里没跟上」最省事的发现方式。
fn trigger_label(trigger: &str) -> String {
    match trigger {
        "threshold" => "自动整理".into(),
        "tool" => "模型主动".into(),
        "retry" => "手动重试".into(),
        other => other.into(),
    }
}

fn row(label: &str, value: String) -> ReadingRow {
    ReadingRow {
        label: label.into(),
        value,
        hint: None,
        tone: ReadingTone::Normal,
    }
}

/// 一次压缩 / 一份压缩产物的**事实行**（数字取自节点 `meta`，口径见
/// `crates/symbio-plugin-loop/src/context/pipeline.rs::compression_stats` 的文档注释）。
///
/// ## 缺数字就不出行，绝不补 0
///
/// `after_tokens` 只在成功路径写入；失败 / 未触发时补一个 0 会显示成「水位已降到 0」，
/// 那是最坏的一种谎报。同理，一个字段都没有 ⇒ [`None`]（前端因此只渲染正文那一行，
/// 与后端没给这些字段的老会话同形）。
pub(crate) fn notice_stats_of(m: &cm::ChatNode) -> Option<ReadingSection> {
    let meta = m.meta.as_ref()?;
    let num = |key: &str| meta.get(key).and_then(|v| v.as_u64());
    let text = |key: &str| meta.get(key).and_then(|v| v.as_str());

    let rows = if meta.get("compacted").and_then(|v| v.as_bool()) == Some(true) {
        // 压缩的**产物**（那条接替历史的记忆自述）：字段名与压缩动作不同。
        let mut rows = Vec::new();
        if let Some(post) = num("post_tokens") {
            rows.push(row("压缩后水位", format_tokens(post)));
        }
        if text("transcript_path").is_some_and(|p| !p.is_empty()) {
            rows.push(row("完整历史", "已转存".into()));
        }
        rows
    } else if m.is_compression() {
        let mut rows = Vec::new();
        // 有后值报「压缩后」，只有前值报「当前」——两个词说的是同一个水位槽。
        match (num("after_tokens"), num("before_tokens")) {
            (Some(after), _) => rows.push(row("压缩后水位", format_tokens(after))),
            (None, Some(before)) => rows.push(row("当前水位", format_tokens(before))),
            (None, None) => {}
        }
        if let Some(limit) = num("context_limit") {
            rows.push(row("上限", format_tokens(limit)));
        }
        if let Some(t) = text("compact_trigger") {
            if !t.is_empty() {
                rows.push(row("触发", trigger_label(t)));
            }
        }
        if let Some(dropped) = num("dropped") {
            rows.push(row("压掉", format!("{dropped} 条")));
        }
        rows
    } else {
        return None;
    };

    if rows.is_empty() {
        return None;
    }
    // 分区不给标题：节点自己已经有 `title`（「上下文压缩」），再来一个标题是排版噪声。
    Some(ReadingSection {
        title: None,
        rows,
        bar: None,
    })
}

/// 会话级**上下文水位**读数（挂会话节点 `attributes.context_watermark`）。
///
/// 入参是 `SessionSummary::context` —— 最近一轮 Turn 骨架 `meta.context` 的原样投影
/// （四个绝对 token 数，写方 `crates/symbio-plugin-loop/src/context.rs::context_watermark`）。本函数是
/// 这些数**变成话**的唯一一处，与 [`notice_stats_of`] 同一条纪律：
///
/// - 单位与精度只在这里定（[`format_tokens`] 全仓唯一）；
/// - 产出的是宿主方言的一个 [`ReadingSection`]（行 + 可选 [`ReadingBar`]），**不是**一个
///   自造形状：几何住在 `bar` 这一格里，渲染器只认「填到这儿 / 在这儿画根线」，
///   不认「已用」「上限」任何一个词（换一个产者——成本、记忆——前端零改动）；
/// - 除法只在这里做：比率由出口算好，前端再除一次就是第二套口径，改系数时
///   只有一半界面跟着变；
/// - 缺数绝不出行、绝不补 0：存量会话（水位落地之前跑的）没有 `meta.context`，
///   那时本函数收不到入参（`Option::None`），属性整条不出现，前端因此不渲染水位条。
///   「上下文是空的」与「这一轮没记录水位」是两件事，不许在界面上混成一件。
pub(crate) fn context_reading_of(wm: &serde_json::Value) -> Option<ReadingSection> {
    let num = |key: &str| wm.get(key).and_then(|v| v.as_u64());
    let content = num("content_tokens");
    let request = num("request_tokens");
    let limit = num("limit_tokens");
    let compress = num("compress_tokens");

    let mut rows: Vec<ReadingRow> = Vec::new();
    let mut level: Option<f64> = None;
    let mut marks: Vec<f64> = Vec::new();
    let mut tone = ReadingTone::Normal;

    // 「已用」一行带上下限：水位条要表达的是「离顶还有多远」，只报分子等于让用户
    // 自己找一个他看不见的分母。
    match (request, limit) {
        (Some(used), Some(cap)) if cap > 0 => {
            level = Some(ratio(used, cap));
            // 请求级开销 = 两者之差：把它单独说出来，用户才知道「历史没长、
            // 但换一批工具定义也会把水位推上去」这件事长什么样。
            let hint = match content {
                Some(body) => format!(
                    "本轮请求开头的估算（内容 {} + 请求级 {}）",
                    format_tokens(body),
                    format_tokens(used.saturating_sub(body))
                ),
                None => "本轮请求开头的估算，含 system prompt 与工具定义".to_string(),
            };
            rows.push(ReadingRow {
                label: "已用".into(),
                value: format!("{} / {}", format_tokens(used), format_tokens(cap)),
                hint: Some(hint),
                tone: ReadingTone::Normal,
            });
        }
        (Some(used), _) => rows.push(row("已用", format_tokens(used))),
        (None, Some(cap)) => rows.push(row("上限", format_tokens(cap))),
        (None, None) => {}
    }

    // 参考线与说明它的那一行**一起下发**：`marks` 里那根线是什么，只有紧跟着这行字
    // 说得清——对应关系住在产者，渲染器不参与命名。
    if let (Some(used), Some(at), Some(cap)) = (request, compress, limit) {
        if cap > 0 {
            marks.push(ratio(at, cap));
            // 语气由产者判（越线 = 需留意），渲染器不解析值的语义再决定颜色。
            tone = if used >= at {
                ReadingTone::Warn
            } else {
                ReadingTone::Normal
            };
            rows.push(ReadingRow {
                label: "自动整理".into(),
                value: format_tokens(at),
                hint: Some("越过这条线后端会自动整理历史（无需你操作）".into()),
                tone,
            });
        }
    }

    if rows.is_empty() {
        return None;
    }
    // 分区不给标题：这条读数自己就是那一句（再配一个「上下文水位」标题是排版噪声）。
    // 没有分母也没有参考线 ⇒ `bar` 整格不出现：渲染器照常把行排完，但**不画一条
    // 宽度 0 的条**冒充「水位是 0」——缺数与读数是两件事。
    let bar = if level.is_none() && marks.is_empty() {
        None
    } else {
        Some(ReadingBar { level, marks, tone })
    };
    Some(ReadingSection {
        title: None,
        rows,
        bar,
    })
}

/// 条形几何：分子 / 分母，**截到 1.0**。
///
/// 截顶是因为这条线只画「离顶还有多远」：请求超出上限是真实的失败形态
/// （`InputOverLimit`），但把条形画成 130% 只会让界面溢出，而那件事已经有
/// 失败节点与告警在说。文本值不截——`已用` 那一行报的始终是实测的数。
fn ratio(part: u64, whole: u64) -> f64 {
    (part as f64 / whole as f64).clamp(0.0, 1.0)
}

#[cfg(test)]
#[path = "stats_reading.test.rs"]
mod tests;
