//! 记忆的**读侧与巩固**：可见域收放 / 检索事实 / 巩固——全部落在 per-session 的 v2 事实源上。
//!
//! **编码写方不在本模块**：轮末两条编码（经验 / 判断）是情景记忆能力包的收束贡献
//! （`crates/symbio-plugin-memory`，DE-026 D-263）。**检索判据也不在本模块**：折叠、
//! 排序与预算自 L2b 起住同一个能力包的 `recall` 子模块（DE-026 D-264），本模块只留
//! 宿主那一半——取快照、按可见域收放、把切片交给检索面。
//!
//! 与 `super::memory`（本会话 `MEMORY.md` 置顶约定，文件形态）的分工：那层记
//! 「钉住的约定」，本层是
//! [DE-036](../../../docs/003-design/036-长期记忆与语义检索.md "DE-036-长期记忆与语义检索") 的**事件
//! 记忆**——不另开文件，记忆就是 `v2-events.wal` 里的普通事件（D-044 同族纪律：
//! 同一事实一份持久化，没有旁路存储）。
//!
//! ## 各段判定方（一个规则一个判定方）
//!
//! | 段 | 判定方 | 本模块做什么 |
//! |---|---|---|
//! | 编码 | 能力包贡献的同文去重 | ——（写方已迁出，D-263） |
//! | 可见域收放 | core 的读侧闸（`content_visible` / `VisScope`） | 逐条收放，交**可见切片**（I-8） |
//! | 检索 | 能力包的 `Retriever`（折叠 / 排序 / 预算） | 把切片交给 `retrieve_all`，读回读数 |
//! | 检索事实 | 能力包的 `semantic` 收束贡献（D-264 第 4 条） | ——（写方已迁出，本模块只交 `Settling.recall` 这个值） |
//! | 巩固 | 本模块（`consolidate::accept` 的调用方） | 按名字读 `memory.live`，合并最旧两条，**过门才入格** |
//!
//! 巩固的**算法**（怎么合并、保真度怎么算）仍在 `symbio_sdk::text::merge_kernel` 与
//! `projections::consolidate`；**活集折叠**住能力包（一副折叠两个读者，D-268）。
//!
//! ## 两种认知内容标签（七类的两个生产写方）
//!
//! | 标签 | 写方 | 溯源 |
//! |---|---|---|
//! | 「经验」 | 本轮**用户**发言（能力包编码） | 本轮 `user.message` 格 |
//! | 「判断」 | 本轮**智能体**的收束正文（能力包编码） | 同一个 `user.message` 格（这条判断因它而起） |
//!
//! 巩固同标签进行（[`CONSOLIDABLE_TAGS`]——取值跟写方拼，两个标签的字面量与能力包
//! 各写一份，一致性由 e2e t29 钉住），技能标签**不在其内**（合并会毁掉 `skill_id`
//! 身份，技能有自己的生死周期）。
//!
//! ## 四条口径（为什么是这样）
//!
//! - **记什么**：由能力包的编码贡献决定（空发言不记、超长截断、同文去重读全量史，
//!   DE-036 §3）——转写（`messages.json`）已存全文，记忆存**提炼**。
//! - **存哪 / 怎么读**：写方只写**本会话自己的** `v2-events.wal`；召回反过来**跨会话
//!   扫**（[`recall_view`]），读侧无锁、按文件数与字节双预算封顶。
//! - **何时注入**：请求视图**置顶**一条 `role=user`、`meta.kind = recall_context` 的
//!   记忆消息（请求级注入、不落库）。段的正文由能力包的 provider 排（`recall::section`），
//!   本模块只把已判过的读数转成端口形状。

// 巩固合并的纯计算内核：生产与 verify 共有同一份源码（立这条纪律，
// 住址随 [D-082](../../../docs/003-design/014-单环一表.md#d-082-6b驱动器成片落为独立插件-loop触发走地址面服务走端口词表先上移分四刀 "DE-014-单环一表") 裁决三归位 core 文本域）。
// 本模块续一条原名短路径——调用点一行不动。
use symbio_sdk::text::merge_kernel::merge_with_fidelity;

use symbio_core::PRINCIPAL_USER;
use symbio_kernel::event::{Entity, Event, Verb};
use symbio_kernel::store::Store;
use symbio_sdk::turn::facts::FleetSnapshot;

use crate::projections::consolidate::{accept, ConsolidateParams, Rejection};

use crate::control_decl::{EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_FORGOTTEN};
use symbio_plugin_api::retrieval::{Retrieval, RetrievalRequest};

/// 日志通道名（`plugin_*!` 宏的 `plugin` 字段）。
///
/// 取值与 `symbio_deploy::PLUGIN_ID_SESSION` 同一串：本插件是会话驱动的那一环，
/// 它的日志一直挂在会话通道上。**这是运维侧的过滤键**，不是本插件的 `PLUGIN.yml`
/// id（那是 `loop`）——迁移批不改任何可观测行为，通道名因此一个字不动。
const LOG_CHANNEL: &str = symbio_deploy::PLUGIN_ID_SESSION;

// ==================== 常量（口径的数值面，单点定义） ====================

/// 单条记忆正文的字符上限（截断点）。
///
/// 与巩固产物同一个数：两条都很长的记忆合并不过 `accept` 的 0.7 下界，
/// **源记忆原样保留**（拒收路径在生产上可达，不是只活在测试里的分支）。
/// 编码侧的截断上限住在写方（情景记忆能力包，DE-026 D-263）。
pub(crate) const MERGE_MAX_CHARS: usize = 400;

/// 触发一次巩固所需的**活**记忆条数（同标签）。取 4：每合并一次净减 1 条，
/// 条数收敛到 3–4 后停在 `accept` 的代数上界（`max_gen = 3`）上。
pub(crate) const CONSOLIDATE_MIN_ENTRIES: usize = 4;

/// 巩固的**标签域**：同标签活记忆 ≥ [`CONSOLIDATE_MIN_ENTRIES`] ⇒ 合并最旧两条。
///
/// 两个生产写方的标签都在内（经验 / 判断）——写方不加巩固域，记忆
/// 就无界增长（每轮一条），那是**静默失效**：扫得越来越慢、注入越来越偏，
/// 没有任何一处报错。标签的字面量由写方交出（情景记忆能力包，DE-026 D-263），
/// 这里按**取值**拼一份——两条拼写是否一致由 e2e t29 钉住，不由本模块依赖
/// 那个 crate 来保证（与技能包对 `SKILL_TAG` 的处理同一条口径）。
/// 技能标签（`SKILL_TAG`）**刻意在外**：合并会丢掉载荷里的
/// `skill_id` 身份，路由与校准账整条断——技能有自己的生命周期（编译 / 置信度
/// 闸 / 反自动化回退），不由巩固代管。
pub(crate) const CONSOLIDABLE_TAGS: [&str; 2] = ["经验", "判断"];

/// 活集折叠的**声明视图名**（owner = 情景记忆能力包，DE-026 D-268）。
///
/// 巩固读它，不读本地投影：折叠只有一份，住在能力包里。名字是公开面（`PLUGIN.yml`
/// 的 `shared_views`），本模块按名字读、不抄口径。
pub(crate) const MEMORY_LIVE_VIEW: &str = "memory.live";

/// **轮末写方读史的窗口**（条）：一份快照的尾部 N 条。
///
/// 两个轮末写方各有一处**全史聚合读**——巩固的活集扫描（本模块 [`consolidate`]）与
/// 技能能力包的校准聚合（它按自己的贡献端口读全史，D-262）。它们读的都是「最近发生过什么」：
/// 巩固从活集里挑最旧两条——**窗外更旧的条目不会让结论换方向**，
/// 只会让这一轮少看一段历史。所以窗口封的是「一轮收尾最多读多久的史」，
/// 与 WAL 总长解耦；这与 `chat_loop::drive` 的收租窗口同一个形态、同一个理由
/// （一轮的收尾不该与 WAL 总长成正比）。
///
/// **它不取 `Frozen::scan_bound`**：那个数是「一份切片能喂给视图多长」（求值前提），
/// 本窗口读的是给巩固 / 校准两处的**全史聚合**，不经过任何视图求值。两个数今天
/// 都是 4096 是巧合，任一侧改动都不该牵连另一侧——所以它们各自成常量。
///
/// ## 它是护栏，不是旋钮
///
/// 值不由用户调——没有配置项，也没有参数面。它的作用只在一个时刻可见：**超窗**。
/// 超窗时两处各留一条读数（`plugin_debug`：看了多少 / 全史多少 / 这一轮少了什么），
/// 平时不出现。所以它不是「恒空旋钮」（写不出值的开关），
/// 而是「有边界、且边界生效时说得出来」的护栏。
///
/// ## 窗口是常量，不是保证
///
/// 窗外的事实这一轮看不见：极长会话里，比窗口更旧的记忆这一轮不参与巩固（它仍在
/// 召回读数里，只是这一轮不会被压缩）。这是**选值的边界**，不是安全边际——
/// 与 DE-015 §2.6 第 7 条同一条纪律：常量表达的是「通常够」，不是「一定够」。
pub(crate) const HISTORY_WINDOW: usize = 4096;

/// 快照尾部 [`HISTORY_WINDOW`] 条，以及「是不是切过」。
///
/// 单独成函数，让「窗口边界」成为**被断言的行为**而不是散在调用点里的一段切片表达式
/// （与 `chat_loop::drive::lease_tail` 同一个理由）：切点写反（取头部 / 少切一条）
/// 立刻有用例红。切过 = `true` ⇒ 调用方留读数（见 [`HISTORY_WINDOW`]）。
pub(crate) fn history_window(snapshot: &[Event]) -> (&[Event], bool) {
    let start = snapshot.len().saturating_sub(HISTORY_WINDOW);
    (&snapshot[start..], start > 0)
}

// ==================== 读方（跨会话召回） ====================

/// 可见切片：跨会话快照 → **收放之后**的事件集（新近在前）。
///
/// `snapshots` 是存储层交回的**值**（[D-084](../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构")）：
/// 枚举平级会话、mtime 新近优先、文件数与字节双预算、判定哪份是调用方自己的
/// 会话——全在 `SessionStore::recall_snapshots`。每份快照是**完整史**（含归档前缀，
/// 存储层 `range(0)` 口径）⇒ 压缩前记下的记忆不会被静默丢掉。
///
/// ## 扫描范围 = 平级会话（刻意不含嵌套子会话）
///
/// 只扫一层不是没看见 `session_wals_deep` 那棵树，
/// 是**范围跟着信息增量走**（与淘汰闸 / stats 的「部署级事实必须完整」不同性质）：
///
/// 1. **子会话的事实经工具结果回流**（代际承诺：`agent_run` 的收束答复进父
///    会话上下文）——子会话的 shared 记忆内容（被委派的 prompt）父会话本来就有
///    （是它发的），判断记忆 thread_private 本就不出会话：深遍的边际
///    信息量 ≈ 0；
/// 2. **召回是有预算的检索**（存储层的 16 个名额 + 字节双上限）：分形委派
///    树的 WAL 恒最新（mtime 序），深遍会让一棵树占满名额、把平级会话挤出
///    扫描窗——那才是真丢失；
/// 3. 判据 `recall_view_scan_is_one_level_deliberately` 钉住这个范围（嵌套
///    子会话的记忆不进顶层召回），改范围先改裁决。
///
/// ## 可见域收放为什么在宿主（I-8）
///
/// 逐条判定（`thread_private` / `shared` / `public` 的取值口径住 core 的读侧闸）是
/// **隔离面**：判错的代价是泄漏，不是少读。能力包只准链接 plugin-api，而
/// `FleetSnapshot` / `VisScope` 是 core 臂的容器面（HS-002 棘轮只许降）——所以这里
/// 交出去的是**收放之后**的切片，能力包不需要、也拿不到「哪个会话是谁的」。
///
/// `viewer` = **本轮主体**（`request_principal`，与写事件的 actor 同一取值点）：
/// - `thread_private`（判断记忆的写方取值）：只对**记录它的那个会话**的轮次可见
///   （`FleetSnapshot::own` = 快照就是调用方的会话）——判断是本部署私有的工作记忆，
///   跨会话可见正是 C10 要消灭的泄漏；存量事件没这个字段 ⇒ 缺省也是它；
/// - `shared` / `public`：本会话恒可见（可见域只管**跨会话**收放，见
///   [`memory_visible`]），跨会话时矩阵内主体可见、矩阵外不可见——与 `stats` 读闸
///   **同一份判据**（`can_see` + 属主 `SESSION_OWNER` 恒放行）。
///
/// 新近在前（`sort_by` 稳定 ⇒ 同毫秒时保持「新会话优先」的输入序）：检索面按给定
/// 次序做配额截断（截尾不截头），所以「哪一头是新」必须由**交切片的人**说清楚。
pub(crate) fn visible_slice(snapshots: Vec<FleetSnapshot>, viewer: &str) -> Vec<Event> {
    let mut events: Vec<Event> = Vec::new();
    for snap in snapshots {
        events.extend(snap.events.into_iter().filter(|e| {
            let is_memory = e.entity == Entity::Memory
                && (e.kind == symbio_plugin_api::events::EVENT_MEMORY_ENCODED
                    || e.kind == EVENT_MEMORY_CONSOLIDATED);
            !is_memory || memory_visible(e, snap.own, viewer)
        }));
    }
    events.sort_by(|a, b| b.ts.cmp(&a.ts));
    events
}

/// 检索的读方：可见切片 → 能力包的检索面 → 本轮读数（一条记忆都没有 ⇒ `None`）。
///
/// 判据全在能力包（折叠 / 排序 / 扫描预算，DE-026 D-264）：本函数只做两件事——
/// 交切片、把读数带回来。`query` = 本轮用户发言，是打分的**语义维**；`None` =
/// 本轮没有问题（取值点在 `chat_loop/inputs.rs::last_user_text`），检索退化为纯
/// 新近序 = 平凡值（J2）。
pub(crate) fn recall_view(
    snapshots: Vec<FleetSnapshot>,
    query: Option<&str>,
    viewer: &str,
) -> Option<Retrieval> {
    let events = visible_slice(snapshots, viewer);
    symbio_plugin_api::retrieval::retrieve_all(&RetrievalRequest {
        events: &events,
        query,
    })
}

/// 一条记忆事件对 `viewer`（本轮主体）可见吗——召回读侧预过滤的**唯一判定点**。
///
/// 可见域管的是**跨会话**的收放：本会话自己的内容（`own`）无论什么域都可见
/// ——「private < shared < public」是放宽序，shared 比缺省更宽，若不含 own
/// 就成了「共享的反而比私有的看得少」（t76 抓到的语义颠倒）。
///
/// 两支各有一个 owner：
/// - 本会话的内容 ⇒ 恒可见（判断记忆的 `thread_private` 走到这一支为止）；
/// - 别的会话的 `shared` / `public` ⇒ 走 core 的读侧闸
///   （`symbio_core::content_visible`，与 `stats` 的整列闸**同一条判据**，
///   ⑧——那条判据若在这里复写一遍，就会漂成「stats 给看、召回不给看」）。
///   缺省（存量事件没有 `vis_scope` 字段）= `thread_private` ⇒ 只剩 own 一条路。
///
/// 取值口径也在 core（`symbio_core::VisScope::of_event`：认不出 = 私有，不把笔误读宽）——
/// 产地在写方（能力包的编码贡献），判定只消费不重算。
fn memory_visible(e: &Event, own: bool, viewer: &str) -> bool {
    use symbio_core::VisScope;
    match VisScope::of_event(e) {
        VisScope::ThreadPrivate => own,
        s => own || symbio_core::content_visible(s, viewer),
    }
}

// ==================== 写方（检索事实）已迁出 ====================

// 检索事实（`memory.recalled`）自 L2c 起由能力包写：`symbio-plugin-memory` 的
// `semantic` 子模块交一条收束贡献，读 `Settling.recall`（宿主在收束时交进去的读数）
// 落一格事实（DE-026 D-264 第 4 条）。本模块因此不再持有这条事实的名字、格位与写口
// ——留在这里就是第二个写方（I1），两份实现可以同时生效。

// ==================== 巩固 ====================

/// 活集里的一条记忆（**声明视图** `memory.live` 的一项，DE-026 D-268）。
///
/// 只解出巩固要用的三格：`seq`（溯源 / 点查全史的键）、`content`（合并的输入）、
/// `tag`（分组）。代数 / 可见域 / 最初一代仍按 seq 点查全史——那几处取的是**一条
/// 具体事实**，与总长无关，且溯源链恰恰需要窗外的最初一代。
struct LiveMemory {
    seq: u64,
    content: String,
    tag: String,
}

/// 按名字读活集（`memory.live`）。
///
/// ## 读不到就不巩固（退化方向写在前面）
///
/// 四种缺席（装配表没冻结 / 名字没公开 / 声明表里没有 / 切片超预算）都是**可诊断的
/// 缺席**，不是「空活集」：本函数把它们折成 `None` + 一条 debug，调用方跳过这一轮。
/// 这不是「静默丢数据」——能力包停用 ⇒ **编码也不发生**（写方是同一个包，D-268 第 6
/// 条），没有增长要治；巩固缺席只意味着「这一轮没压缩」。
///
/// 解析失败同理：形状是公开契约（`{seq, ts, content, tag}`），读不出来说明版本对不上，
/// 那是装配问题——出声（warn），不假装「没有活记忆」。
fn live_set(scan: &[Event]) -> Option<Vec<LiveMemory>> {
    let value = match symbio_plugin_api::read_shared_view_all(MEMORY_LIVE_VIEW, scan) {
        Ok(value) => value,
        Err(why) => {
            symbio_plugin_api::plugin_debug!(
                LOG_CHANNEL,
                "[memory] 活集视图 `{MEMORY_LIVE_VIEW}` 本轮读不到（{why}）：本轮不巩固"
            );
            return None;
        }
    };
    let Some(items) = value.as_array() else {
        symbio_plugin_api::plugin_warn!(
            LOG_CHANNEL,
            "[memory] 活集视图 `{MEMORY_LIVE_VIEW}` 的形状不是数组：本轮不巩固"
        );
        return None;
    };
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let (Some(seq), Some(content), Some(tag)) = (
            item.get("seq").and_then(serde_json::Value::as_u64),
            item.get("content").and_then(serde_json::Value::as_str),
            item.get("tag").and_then(serde_json::Value::as_str),
        ) else {
            symbio_plugin_api::plugin_warn!(
                LOG_CHANNEL,
                "[memory] 活集视图 `{MEMORY_LIVE_VIEW}` 的一条缺少 seq/content/tag：本轮不巩固"
            );
            return None;
        };
        out.push(LiveMemory {
            seq,
            content: content.to_string(),
            tag: tag.to_string(),
        });
    }
    Some(out)
}

/// 巩固写方：[`CONSOLIDABLE_TAGS`] 每个标签内，活记忆 ≥
/// [`CONSOLIDATE_MIN_ENTRIES`] ⇒ 合并该标签最旧两条，**过 `accept` 才入格**。
/// 返回 `(标签, seq)` 列表（条数不够 / 被拒收 ⇒ 不入列——拒收不是错误，
/// 源记忆原样留在活集里，下一轮再试）。
///
/// 沿 `sources` 溯源链收集**最初一代**的正文——保真度的比较基准。
///
/// 产物可以再次被巩固：只和当次两条源比会掩盖逐代累积的漂移（每代都「合格」，
/// 产物却早已偏离最初的信息）。[DE-001 §8](../../../docs/003-design/001-核心架构.md "DE-001-核心架构")
/// 的口径是「与源比，不与上一代比」，而 `sources` 载荷（I2 规定的产物溯源）就是
/// 取回最初一代的路径：没有 `sources` 的事件即最初一代。
///
/// 取不到快照条目（历史归档外的边角）时退回该条自身的正文——那是唯一可知的来源，
/// 不假装拿到了更早的一代。
fn origin_texts(snapshot: &[Event], sources: &[(u64, &str)]) -> Vec<String> {
    fn walk(
        snapshot: &[Event],
        seq: u64,
        seen: &mut std::collections::HashSet<u64>,
        out: &mut Vec<String>,
    ) {
        if !seen.insert(seq) {
            return;
        }
        let Some(event) = snapshot
            .iter()
            .find(|e| e.seq.map(|s| s.value()) == Some(seq))
        else {
            return;
        };
        if let Some(sources) = event.payload.get("sources").and_then(|v| v.as_array()) {
            for s in sources.iter().filter_map(|v| v.as_u64()) {
                walk(snapshot, s, seen, out);
            }
            return;
        }
        if let Some(content) = event.payload.get("content").and_then(|v| v.as_str()) {
            out.push(content.to_string());
        }
    }

    let mut out = Vec::new();
    for &(seq, fallback) in sources {
        let before = out.len();
        walk(
            snapshot,
            seq,
            &mut std::collections::HashSet::new(),
            &mut out,
        );
        if out.len() == before {
            out.push(fallback.to_string());
        }
    }
    out
}

/// 写入顺序是设计过的：**先写合并产物、后写两条 `memory.forgotten`**——后者失败
/// ⇒ 活集里多一条重复（无害），前者失败 ⇒ 源记忆还在。反过来会丢记忆。
///
/// 读史分两层：**聚合扫**只扫快照尾部 [`HISTORY_WINDOW`] 条（护栏，超窗留读数），
/// **按 seq 点查**仍走全史（代数 / 可见域 / 最初一代溯源）——那几处是「取一条具体事实」，
/// 与总长无关，且溯源链恰恰需要窗外的最初一代。
///
/// **活集从能力包的声明视图读**（`memory.live`，D-268）：折叠只有一份，住能力包；
/// 本函数不再持有一份私有折叠（那是两副折叠，必然漂移）。
pub(crate) fn consolidate(
    store: &dyn Store<Event = Event>,
    snapshot: &[Event],
    turn: u64,
    now: i64,
) -> Result<Vec<(&'static str, u64)>, String> {
    // 扫描面 = 快照尾部 `HISTORY_WINDOW` 条（护栏，不是旋钮——见常量的文档）。
    // **点查仍走全史**（`generation_of` / `scope_of` / `origin_texts`）：它们按 seq
    // 找一条具体的事实，代价 O(1) 条、与总长无关，且窗外那几条恰恰是溯源链要的
    // 最初一代——窗口只封「聚合扫」，不封「按 seq 取一条」。
    let (scan, truncated) = history_window(snapshot);
    if truncated {
        symbio_plugin_api::plugin_debug!(
            LOG_CHANNEL,
            "[memory] 巩固只扫尾部 {} 条事实（全史 {} 条）：窗外更旧的记忆本轮不参与压缩",
            scan.len(),
            snapshot.len()
        );
    }
    let Some(live) = live_set(scan) else {
        return Ok(Vec::new());
    };
    let mut merged: Vec<(&'static str, u64)> = Vec::new();
    for &tag in CONSOLIDABLE_TAGS.iter() {
        // 活集是新近在前（视图的口径）⇒ 同标签倒序取最旧两条（巩固从最旧开始推进代数）。
        let entries: Vec<&LiveMemory> = live.iter().filter(|e| e.tag == tag).collect();
        if entries.len() < CONSOLIDATE_MIN_ENTRIES {
            continue;
        }
        let oldest: Vec<&LiveMemory> = entries.iter().rev().take(2).copied().collect();
        if oldest.len() < 2 {
            continue;
        }
        let source_seqs = [oldest[0].seq, oldest[1].seq];

        // 代数从源事件的载荷读（活集不带代数——它是读视图，不是写账本）。
        let generation_of = |seq: u64| -> u32 {
            snapshot
                .iter()
                .find(|e| e.seq.map(|s| s.value()) == Some(seq))
                .and_then(|e| e.payload.get("generation"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32
        };
        // 可见域同样从源事件载荷读（合并产物继承源记忆的内容侧可见域）——
        // 同标签的源可见域相同（写方按标签定，见情景记忆能力包的编码贡献），取不到
        // （源不在快照里）时落 thread_private：合并放大可见域比丢一次合并更坏。
        let scope_of = |seq: u64| -> symbio_core::VisScope {
            snapshot
                .iter()
                .find(|e| e.seq.map(|s| s.value()) == Some(seq))
                .map(symbio_core::VisScope::of_event)
                .unwrap_or_default()
        };
        let generation = generation_of(source_seqs[0]).max(generation_of(source_seqs[1])) + 1;
        let vis_scope = scope_of(source_seqs[0]);

        // 保真度与**最初一代**比（跨代累积），不是与当次两条源比——后者每代都
        // 合格却可能早已偏离最初的信息。溯源链从 `sources` 载荷取回。
        let basis = origin_texts(
            snapshot,
            &[
                (source_seqs[0], oldest[0].content.as_str()),
                (source_seqs[1], oldest[1].content.as_str()),
            ],
        );
        let basis: Vec<&str> = basis.iter().map(String::as_str).collect();
        let (content, fidelity) = merge_with_fidelity(
            &[oldest[0].content.as_str(), oldest[1].content.as_str()],
            &basis,
            MERGE_MAX_CHARS,
        );

        // 判定方只有一个：core 的 `accept`。拒收 ⇒ **不入格**（不是降标入库，也不是
        // 写条日志照写）。两个分支分开说，是因为它们是两件事：代数上界是**预期的终点**，
        // 保真度不足是**这条策略装不下它**。
        match accept(&ConsolidateParams::default(), generation, fidelity) {
            Ok(()) => {}
            Err(Rejection::RejectedMaxGen { generation, max }) => {
                symbio_plugin_api::plugin_debug!(
                    LOG_CHANNEL,
                    "[memory] 巩固停在代数上界（gen {generation} > max {max}，tag={tag}）：源记忆原样保留"
                );
                continue;
            }
            Err(Rejection::RejectedLowFidelity { fidelity, min }) => {
                symbio_plugin_api::plugin_debug!(
                    LOG_CHANNEL,
                    "[memory] 巩固保真度不足（{fidelity:.3} < {min}，tag={tag}）：源记忆原样保留"
                );
                continue;
            }
        }

        let consolidated_seq = store
            .append(
                Event::pending(
                    format!("v2mc-{turn}-{}-{}", source_seqs[0], source_seqs[1]),
                    EVENT_MEMORY_CONSOLIDATED,
                    Entity::Memory,
                    Verb::Progressed,
                    turn,
                    PRINCIPAL_USER,
                )
                .with_produced_by(source_seqs[0])
                .with_ts(now)
                .with_payload(serde_json::json!({
                    "content": content,
                    "tag": tag,
                    "generation": generation,
                    "fidelity": fidelity,
                    "sources": source_seqs,
                    "vis_scope": vis_scope.name(),
                })),
            )
            .map_err(|e| format!("memory.consolidated 入格失败：{e:?}"))?;

        // 源记忆 → 排除式遗忘：Log 不删，只是活集不再包含（可撤销、可审计）。
        for target in source_seqs {
            store
                .append(
                    Event::pending(
                        format!("v2mf-{target}"),
                        EVENT_MEMORY_FORGOTTEN,
                        Entity::Memory,
                        Verb::Closed,
                        turn,
                        PRINCIPAL_USER,
                    )
                    .with_produced_by(target)
                    .with_ts(now)
                    .with_payload(serde_json::json!({
                        "why": "consolidated",
                        "into": consolidated_seq.value(),
                    })),
                )
                .map_err(|e| format!("memory.forgotten 入格失败：{e:?}"))?;
        }
        merged.push((tag, consolidated_seq.value()));
    }
    Ok(merged)
}

#[cfg(test)]
#[path = "memory_events.test.rs"]
mod tests;
