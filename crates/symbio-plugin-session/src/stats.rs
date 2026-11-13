//! `session/stats` —— v2 事实源的**只读读数口**。
//!
//! 背景：SLO §1.2 的四列（时延 / 兜底 / 成本 / 断点）由本插件的投影与 core 的
//! 协议投影算得出，
//! 但**算得出 ≠ 读得到**——没有生产出口，投影只是「能算的函数」，三类收益
//! 无法验收、回归也看不见。本模块就是那一格的读侧出口：出口形态②（只读路由）。
//! 读数今天还多一族**能力视图**：键来自各能力包的报表登记（`ReportDecl.views`）、值由
//! owner 自己的折叠算出，本插件只按登记取、不写任何视图名（`D-387`）——
//! 一项新能力登记一条视图，这一列就多一格，本文件不动一行。
//!
//! ## 两个文件的分工（判据：这一段错了谁会红）
//!
//! - 本文件：**出口形状与取数**——`TierRow` / `SessionStats` / `ViewerRequest` 这些
//!   契约字段，以及「读哪些会话、快照怎么来」的取数与路由。它出错，红的是
//!   **读不到 / 字段缺**；
//! - [`stats_reading`]：**读数装配**——把投影值排成每一格（可见域、承诺列、实例级
//!   聚合、面板）。它出错，红的是**某一格的数字与 core 的口径不符**。
//!
//! 分在这里不是为了行数，是因为这两类红的处置动作不同：前者要改的是出口契约与
//! 取数范围，后者要改的是换算，而纪律要求它们都不许自己发明口径。
//!
//! ## 三条纪律
//!
//! - **只调已有投影，不另写一份统计**：
//!   这两个文件里没有分位数 / 占比 / 累计的算式——口径只活在投影里（本插件 `projections/` 与 core 的协议投影），出口
//!   只做「取数 + 排版」。否则同一份事实会长出两套口径，读数与投影静默漂移，
//!   而两边都自称是 SLO。**不变量同理**：清单只调 `check_all`，宽限与预算判据
//!   由 core 定（本处不复判）。
//! - **真·只读**：事实由存储层以**值**交回（`SessionStore::snapshot` / 部署级
//!   扫描），本模块不拼路径、不开文件、不取令牌。存储层只读打开：不创建文件、
//!   不截断撕裂尾行——截断是**写方**的恢复语义，读方顺手做会与正在落行的写方
//!   撞车（append 模式下截断后剩下的字节接在新 EOF ⇒ 那一行静默损坏）
//!   （[D-084](../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构")）。
//! - **不重写视图类型**：口径住在 owner 那里——时延 / 兜底 / 跨度三列归能力包 `slo`、
//!   成本列归能力包 `cost`（各自 `ViewDecl::eval`，`slo` 另有 `agg`），本插件按名字读
//!   （`D-221`）并把值原样装进出口（`D-281`）；自主程度等仍是本插件 `projections/` 里的
//!   纯折叠。core 只留协议自身的投影（转写 / 轮状态 / 任务词表等），不再为会话读数占根
//!   出口（DE-025 D-221）。
//!
//! ## as-of 口径
//!
//! 本模块的读数一律走 `Projection::read_all`（= `now = i64::MAX` + 全量预算）：WAL 是
//! **落盘**事实源，文件里每一条都已发生，没有「未来的事件」可滤——按报表读的列
//! （`reports::read_all`）与能力视图（`capability_views` 的 `read_view_with_ver`）
//! 走的是同一个口，两族因此同一个 as-of。

use crate::declarations::{CHAT_VOCAB, MESSAGE_VOCAB};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use symbio_kernel::event::Event;
use symbio_kernel::plugin::{
    PluginError, PluginInvokeRequest, PluginInvokeResponse, PluginPayload,
};
use symbio_kernel::present::ReadingPanel;
use symbio_sdk::invariants::check_all;
use symbio_sdk::projection::transcript::transcript;

use crate::projections::checkpoint::checkpoint;

use super::plugin::SessionPlugin;
use super::route_guard::{require_sid, viewer_of};
use super::stats_reading::{
    capability_views, commitments_reading, content_scope_of, fleet_reading, panel_of, usage_section,
};

/// 一个档位的一行读数：时延列与兜底列并排（同一档位的两类收益同处一行）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct TierRow {
    /// 档位名（`deep` / `fast` / `reflect` / `autonomous` / `unspecified`）。
    pub tier: String,
    /// 轮数——兜底率的**分母**（开轮的 `user.message` 计数）。
    pub turns: u64,
    /// 其中走了兜底的轮数（分子）。
    pub fallbacks: u64,
    /// 兜底率（口径在能力包 `slo` 的 `rate`，本处只读它交的值）。
    pub rate: f64,
    /// 成功轮实测时延的分位数（口径在能力包 `slo` 的 `percentile`，本处只读它交的值）。
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    /// 时延样本数（兜底轮不计入时延样本——失败的耗时不是成功响应的耗时）。
    ///
    /// 取自 owner 交的 `sample_ms` 的**长度**：owner 交的是样本本身（跨会话合并要从
    /// 样本重算分位数），出口只要个数——取长度是排版，不是重算。
    pub samples: usize,
}

/// 会话级读数（四列同源，D-044）。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SessionStats {
    /// 会话 id。
    pub session_id: String,
    /// 事实源在不在。不在 = 这个会话还没跑过一轮，全零读数是**有据的零**，
    /// 不是错误（读方不该为了读而创建文件）。
    pub has_wal: bool,
    /// **报表列**（`D-387`）：键 = 报表登记的 `slot`，值 = owner 的 `render` 交出的
    /// 整条读数。
    ///
    /// ## 键由登记表决定
    ///
    /// 本插件不认识任何一列的键名或形状：owner 在自己的 crate 里 `submit_reports!`
    /// 一条登记，下一次读数就多一格。`#[serde(flatten)]` 把这些键抬到 `session/stats`
    /// 顶层，于是**出口顶层键集合 == 已登记 `slot` 集合**是构造保证（全装配判据见
    /// `crates/symbio` 的 `report_columns_acceptance`）。
    ///
    /// ## 键序与降级
    ///
    /// 序列化键序从「结构体声明序」变成 `BTreeMap` 的字典序——**数据不变，只是顺序**。
    /// 某一列今天读不出来（没链接 owner / 登记表不合法）⇒ 该键**整条不出现**，由
    /// `capability_views` 的可诊断缺席说明原因；读方不替 owner 猜空形状。
    #[serde(flatten)]
    pub reports: std::collections::BTreeMap<String, serde_json::Value>,
    // 用量与金额两列已随写方走（`D-461`）：`model` 写 `response.usage` /
    // `response.spend`，折叠也归它；本出口按 `slot`（`usage` / `spend`）从
    // [`Self::reports`] 收，不再持有这两本账。两列的口径与空形状见
    // `crates/symbio-plugin-model/src/ledger.rs`。
    /// 断点（`checkpoint` 视图原样序列化：`last_seq` / `event_count` /
    /// `kind_counts`——末两项就是「这一格里到底有多少事实」）。
    pub checkpoint: serde_json::Value,
    /// 不变量违规清单（`check_all` 七条：C1 `seq` 单调 / C3 断言带溯源 /
    /// C3 对偶 溯源不悬空 / C5 超预算 / C6 停机原因不在闭集 /
    /// `acyclic_deps` 依赖成环或悬空 / `rework_bounded` 返工超上界）。
    /// **空 = 七条全绿**；每条带 `event_id` 与人话。宽限口径（按声明档位取预算、
    /// 切片首部的跨片溯源边放行、历史收束格缺 `reason` 放行）
    /// 在 core 的 `check_all`，
    /// 任务表两条**无宽限**（环与超界不是时序假象），本处只取数——
    /// 「不变量进 CI/读侧」：e2e 断言的就是这一列。
    pub invariants: serde_json::Value,
    /// 承诺 / 委托（`agent_run` 那一次立约的**读侧出口**）：
    /// `{ items: [ { id, from, to, promise, state, why, anchor_seq } ], total, kept,
    /// broken, settled, breach_rate }`，`items` 按承诺号字典序。
    ///
    /// ## 这一列把「自述」变成「查得到的事实」
    ///
    /// `agent_run` 在能力元数据里自声明「我记委托账」（`delegation_promise` 行为名），
    /// 声明面由 `tool-decls-audit` 守着（名单封闭），但**声明说得不等于事实落了**——
    /// 判据只能是「这一轮委托过 ⇒ 这里读得出那一条」。写侧（工具执行层 → 轮末
    /// `record_derived`）与读侧（core 的 `commitment_records` 翻译）各钉了一段，中间
    /// 只有 `commitment_events` 这一个成格口（I1）。
    ///
    /// ## 违约率的分母是**已了结**数，不是立约数
    ///
    /// 在途的约既没守也没违约，把它放进分母等于把「这一轮还没跑完」读成违约。
    /// 率是纯查询（与 `unattended_rate` / `adoption_rate` 同族）：分子分母落数，
    /// 读时再除，不落第二个数。分母为 0 ⇒ `0.0`（与那两族同一个平凡值）。
    ///
    /// 归**四列**那一族（同一份切片、同一个 as-of）：`may_read = false` ⇒ 切片已空 ⇒
    /// `{ items: [], total: 0, … }`，两种「零条」由 `has_wal` 分辨。
    pub commitments: serde_json::Value,
    /// 转写（**对话面读侧**）：
    /// `transcript` 投影原样序列化（`{ entries: [ { role, text } ] }`，按事件顺序 =
    /// append-only 的时间序；`role` 是 `"user"` / `"assistant"` / `"tool"`，
    /// 工具行另带 `tool` = 工具名）。
    ///
    /// 读的是**事实源本身**（`user.message` / `chat.assistant.final` /
    /// `chat.assistant.fallback` / `artifact.added` 四格——最后一格是工具结果），
    /// 不是会话存储里的消息副本——同一份事实既喂模型（`Reasoner` 组 prompt 的输入）
    /// 又能从这里读回，D-044 的「实测与判据同源」。
    ///
    /// 归**四列**那一族（同切片、同 as-of、`may_read = false ⇒ 空切片` ⇒
    /// `{ entries: [] }`），不取声誉列的空对象形态：对话为空与「不给你看」由 `has_wal`
    /// 分辨——同一份判定，换个列名不换判据。
    ///
    /// 逐条可见域（「这条是谁说的」）不在这里过滤：本列只有**整列**闸（`may_read`），
    /// 与其余各列同款。
    pub transcript: serde_json::Value,
    /// **能力视图**（DE-025 D-221）：别的插件公开的视图，按名字读出来的原样值。
    ///
    /// ## 为什么这一列没有写死的键名
    ///
    /// 它是**通用读口**，不是某一项能力的专用列：键 = owner 在 `submit_reports!` 的
    /// `ReportDecl.views` 里登记的名字，值 =
    /// `{ owner, status: "ok", ver, value }`，或
    /// `{ owner, status: "unavailable", reason, detail }`。加一条「最近几条便签」
    /// 只需要能力包自己登记视图——本插件不改一行，下一次读数就多一格；
    /// 读的是 owner 的折叠（`ViewDecl::eval`），本插件不抄它的口径。
    ///
    /// ## 缺席态各有名字
    ///
    /// 未冻结 / 已声明但没实现分别报 `not_sealed` / `unknown`
    /// （词表在 `ViewUnavailable::reason`），不折叠成空值——「没算出来」与「算出来是空」
    /// 必须分得开（与 `usage.calls` 同一条纪律）。读侧没有「超预算」这一态：切片给多长
    /// 就折多长，`cost` 只约束环内窗口求值（`D-284`）。
    ///
    /// 归**四列**那一族：与其余各列同一份切片、同一个 as-of；`may_read = false`
    /// ⇒ 空切片 ⇒ 各视图读到「有据的空」，由 `has_wal` 与缺席态分辨。
    pub capability_views: serde_json::Value,
    /// **读数面板**（读数的**呈现描述**，前端零业务知识地渲染它）。
    ///
    /// ## 为什么读数也要一份描述
    ///
    /// 「这些数是什么意思、该叫什么、单位与精度怎么取」只有**产者**知道。前端自己
    /// 拼就等于把口径抄进第二处（D-044 的反面），而读数一改，抄本不报错、只静静
    /// 显示旧标签。故本字段由 `panel_of` 从**上面各列已算出的值**生成——
    /// 前端只按 [`ReadingPanel`] 排版，不解析任何会话/统计语义。
    ///
    /// ## 与各列的关系：一份事实、两种用法
    ///
    /// 各列（`tiers` / `cost` / `fleet` …）是**机器可读**的读数，判据与脚本读它们
    /// （`slo-exit-audit` 对的就是这些列）；本字段是**人读**的那一份，同一个数的
    /// 另一种说法。两者由同一个源算出，且判据 `panel_matches_the_columns_it_describes`
    /// 钉住「不脱节」——否则就是两份真相。
    pub panel: symbio_kernel::present::ReadingPanel,
    /// 用量那**一格**的读数（`ReadingSection`）：给「地方只有一条、但必须在眼前」的
    /// 位置用（会话头），与 [`Self::panel`] 里那一段**同一次装配**
    /// （`stats_reading::usage_section` 的唯一实现，两处调同一个函数，不是两份口径）。
    ///
    /// ## 为什么不与 `panel` 合并成一个字段
    ///
    /// 两个消费方要的量不同：抽屉要**全部**分区（时延 / 断点 / 不变量……），会话头
    /// 只要**一行**。让前端从 `panel.sections` 里按标题文本挑「用量」那一节，等于把
    /// 中文标签变成跨栈的定位键——改一个字，前端就静默少一格（方言里
    /// [`ReadingSection`] 没有身份字段，只有标题）。故产者直接把要的那一格交出来，
    /// 前端一个字符串都不用认。
    ///
    /// `None` = 一次实测都没有（`calls == 0`）⇒ 前端整格不渲染，不画一条空读数
    /// （与 `context_watermark` 缺数整条不出现同一条纪律）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<symbio_kernel::present::ReadingSection>,
    /// **实例级聚合**（跨会话的 SLO 真数）。
    ///
    /// 上面每一列回答「**这一个会话**怎么样」；本列回答「**整个实例**怎么样」——
    /// 口径是把本插件目录下**所有**会话的 WAL 各读一遍，再交给能力包 `slo` 的
    /// `ViewDecl::agg` 合并（`D-281`）。分子分母与分位数因此与单会话列**逐字同一套
    /// 口径**（D-044：实测与判据同源），本处不另写一套统计，也不认识交回来的形状。
    ///
    /// ## 为什么这不是「多一份真相」
    ///
    /// 它读的是**同一批 WAL、同一批投影**，只是切片范围从「一个会话」扩到
    /// 「整个实例」。真正的第二份真相是「另写一套合并算式」——那正是这里避免的。
    ///
    /// ## 读不到的时候给什么
    ///
    /// `may_read = false` ⇒ `{ wals: 0, sessions: 0 }`（与四列同族的空，不给零值
    /// 条目：不给读的人报数）。实例里一个 WAL 都没有 ⇒ `wals: 0`，同形状。
    ///
    /// ⚠️ 成本：本列要**遍历实例下的每个会话**。会话数上去之后它是本读数口最贵的
    /// 一列（其余各列只读一个文件）。今天没有生产调用方，接进前端时应
    /// 连同这一条成本一起考虑——要么按需取（单独一条路），要么增量缓存。
    pub fleet: serde_json::Value,
}

/// 读一个会话的事实源，出四列读数 + 校准 / 声誉 / 就绪集 / 转写四列 + 不变量清单 + 承诺列，
/// 再加一列**能力视图**：按各能力包公开的清单逐条经 `symbio_sdk::plugin::views` 读，
/// 本插件不写任何视图名（DE-025 D-221）。
///
/// 纯读：不写文件、不改网格、不碰会话存储（消息 / 转写）。事实由存储层以**值**
/// 交回（[D-084](../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构")）：`snapshot` 是本会话
/// 完整史（含归档前缀），`deep_facts` 是部署级快照的**惰性取数**——只有读方有权
/// 看时才调用（`may_read` 由本函数**单点**判定，见下）。
///
/// `viewer = Some(身份)` 时先按可见域取**可读切片**再出各列（**读什么由能看什么
/// 决定**，不是先算完再裁结果）；`None` = 本机默认，与判定引入之前逐字一致。
///
/// `session_principal` 是**本会话主体**（声誉列的 `own` 取谁），与读方身份是两回事：
/// 读方是「谁在看」，会话主体是「这份承诺是谁立的」——把两者合成一个参数会让
/// 「别人看我的声誉」退化成「别人的声誉」。
///
pub(crate) fn read(
    session_id: &str,
    snapshot: Option<Vec<Event>>,
    deep_facts: impl FnOnce() -> Vec<Vec<Event>>,
    viewer: Option<&str>,
    session_principal: &str,
    // 调度容量（本会话配置 `task_cap`）：就绪集列按**同一个** `cap` 取，
    // 与调度段 / 调度者同源——三处各取一个数，就会读出「调度派了 1 笔，
    // 统计说候选有 5 笔」这种看不出谁错的差。
    task_cap: usize,
) -> Result<SessionStats, PluginError> {
    let has_wal = snapshot.is_some();
    // 完整史（归档段 + 未取代的那段）：`snapshot` 由存储层按 `range(0)` 口径交值。
    let snapshot = snapshot.unwrap_or_default();

    // 读侧闸：**读什么由能看什么决定**——各列与不变量列都只从「你看得见的事实」算，不先算完再裁结果。
    //
    // 属主 = 本机会话的属主（`SESSION_OWNER`，部署事实；`[会话] ≈ [线程]`，
    // thread 实体落地前用会话属主）。于是：
    // 属主本人 = 全量读数（与不声明时逐字一致）；矩阵外主体 = **空切片**
    // ⇒ 四列全零、就绪集为空、不变量清单为空，与 `has_wal: true` 并排即可分辨「有源但不给你
    // 看」，而不是被读成「这里没有数」。
    // 可见域只判**一次**：四列、声誉列、就绪集列与转写列共用同一个答案——两个判定点 = 两套判据，
    // 必然漂移成「四列能看、声誉不能看」这种没有理由的半开半掩。
    //
    // **scope 取内容侧的事实**（②）：
    // 本会话记忆载荷里记的 `vis_scope`（情景记忆能力包收束贡献的产地，①），
    // 取**最宽**的那个——这份事实源里只要有共享内容，它就不是纯私有的。缺省
    // （没有任何记忆 / 存量事件没这个字段）= `thread_private`（C10），与接线前
    // 逐字一致。判据不再是恒传缺省值：「人的认知可供他人借用」（SC-010 判据 3）
    // 由此有生产读方——矩阵内主体读得到他人共享内容，矩阵外主体照旧空切片。
    //
    // **属主恒放行 + 矩阵谓词**那一条判据住在 core（`authz::content_visible`，
    // 与召回的逐条预过滤同一条，⑧）；本处只加自己的那一层作用域：
    // **未声明读方 = 全量**（CLI / 前端不署名的那条路径，沿用接线前口径）。
    let content_scope = content_scope_of(&snapshot);
    let may_read = match viewer {
        None => true,
        Some(principal) => symbio_sdk::authz::content_visible(content_scope, principal),
    };
    let snapshot = if may_read { snapshot } else { Vec::new() };

    // 报表读侧的公共上下文：三格都由本调用方给定，owner 的 `render` 只消费、不反查
    // （`D-387`）。可见域整列判一次的口径不变，`readable` 只是把同一个答案告诉 owner。
    let report_ctx = symbio_plugin_api::reports::ReportCtx {
        principal: session_principal,
        cap: task_cap,
        readable: may_read,
    };

    // 报表列（`D-387`）：`symbio_plugin_api::reports::read_all` 遍历已链接的报表登记，
    // 一次读出全部列。键 = 登记的 `slot`，`SessionStats::reports` 用 `#[serde(flatten)]`
    // 把它们抬到出口顶层——本插件不列任何列名、不认识任何一列的形状，加一条报表就多
    // 一格。登记表不合法时整批缺席并出声；「某一列算不出来」由 owner 的 `render` 交
    // 空形状（空形状住在 owner 那里）。
    let reports = symbio_plugin_api::reports::read_all(&snapshot, &report_ctx);

    // 以下各列仍由本插件折叠与排版（口径在 core 的投影与本插件的投影）。
    // 断点列读 `checkpoint` 投影；用量与金额两列不在这里——它们是 `model` 的报表
    // （`D-461`），随 `reports` 一起到。
    let ck = checkpoint().read_all(&snapshot).value;

    let checkpoint = serde_json::to_value(&ck)
        .map_err(|e| PluginError::InternalError(format!("断点序列化失败：{e}")))?;
    // 不变量清单的词汇面（DE-024 D-203）：判据问「这个名字扮演什么角色」，答案取自
    // owner 交来的装配表；本二进制没装出表时用清单生成物（`symbio_plugin_api::vocab`）。
    let vocab = symbio_plugin_api::vocab();
    let invariants = serde_json::to_value(check_all(vocab, &snapshot))
        .map_err(|e| PluginError::InternalError(format!("不变量清单序列化失败：{e}")))?;

    // 承诺列（段二）：立约记录与违约率。切片已按 `may_read` 取过。
    let commitments = commitments_reading(&snapshot);

    // 转写列（对话面读侧）：
    // doc-link-allow D-004: 「四格 → 三角色」是事件形状到角色的静态映射，非版本改造
    // 与四列同一份切片、同一个 as-of。口径（四格 → 三角色，含工具结果）只活在 core
    // 的 `transcript` 投影里，本处只取数 + 排版；`may_read = false` 时切片已空 ⇒
    // `{ entries: [] }`。
    let transcript = serde_json::to_value(
        transcript(CHAT_VOCAB, MESSAGE_VOCAB)
            .read_all(&snapshot)
            .value,
    )
    .map_err(|e| PluginError::InternalError(format!("转写序列化失败：{e}")))?;

    // 实例级聚合列：owner 在 `ReportDecl::fleet` 里声明落点，读侧按登记求值、按登记
    // 落格（`D-488`）。`may_read = false` 时**不扫**（扫遍实例再报「不给你看」是白花
    // 代价）：交空快照，owner 的 `agg(&[])` 交有据的零 / 空——空形状住 owner 那里
    // （与单会话列同一条），读方不再替 owner 猜形状。
    let fleet = fleet_reading(
        &if may_read { deep_facts() } else { Vec::new() },
        &report_ctx,
    );

    // 能力视图列（DE-025 D-221）：遍历**公开清单**，逐条经 plugin-api 的读口求值。
    // 本插件不认识任何具体视图名——名字、版本、成本与求值函数都在 owner 的声明里；
    // 新能力包公开一条视图，这里就多一格，不动本文件。
    let capability_views = capability_views(&snapshot);

    let reading = SessionStats {
        session_id: session_id.to_string(),
        has_wal,
        reports,
        checkpoint,
        invariants,
        commitments,
        transcript,
        fleet,
        capability_views,
        panel: ReadingPanel::default(),
        usage: None,
    };
    // 会话头那一格与面板里的「用量」段同源：两处都调 `usage_section` 这**一个**函数，
    // 入参是同一份 `reports`。分别算两次是纯函数，不产生第二份口径。
    let usage = usage_section(&reading.reports);
    Ok(SessionStats {
        panel: panel_of(&reading),
        usage,
        ..reading
    })
}

impl SessionPlugin {
    /// 路由 `session/stats` 的处理：向存储层取**值**（本会话快照 + 部署级提示的
    /// 惰性取数），交给 [`read`]。
    ///
    /// 与 `chat/send` / `chat/abort` 并列在路由表里，但性质不同——那两条是
    /// **编排 / 控制**，本条是**读数**：既不是数据 CRUD（CRUD 全走 VDFS，见
    /// `plugin.rs::route` 的分支注释），也不产生任何副作用。
    pub(crate) async fn handle_stats(
        &self,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sid = require_sid(&ctx, "session/stats")?;
        let store = self.get_store().await?;
        // 本会话快照立即取（读侧闸要它算可见域）；部署级扫描推迟到闸判之后——
        // `read` 只在 `may_read` 时调用 `deep_facts`（不给看就不扫）。
        let source = store
            .fact_source(&sid)
            .map_err(PluginError::InternalError)?;
        let snapshot = source.map(|s| s.range(0));
        // 载荷 = **读方身份声明**（可选）：不带 `principal` = 本机默认，与读侧闸
        // 引入之前逐字一致；带了却读不出身份（类型不对）⇒ 声明了却判不了 ⇒ 拒绝
        // 读数，不悄悄降级成「未声明」（fail-closed）。载荷整体缺席同理拒绝。
        let viewer: Option<String> = viewer_of(&ctx, "session/stats")?;
        // 会话主体 = **目标会话自己的**（`metadata.agent_id` → `principal_of`，
        // 与 `delegating_principal` / 心跳回退链同一份词汇），不是请求方的：
        // 本端点支持跨会话查数（委派监督、e2e 都传别人的 `session_id`），而
        // [`read`] 文档写明 `own` 列的语义是「这份事实源里的承诺是谁立的」——
        // 接到请求方就会把「别人看我的声誉」读成「别人的声誉」（t75 抓到的错标）。
        // 元数据没记 `agent_id`（未选智能体的会话 / 旧会话）⇒ 回落请求方取值点：
        // 自查场景两者恒同，行为与接线前逐字一致。
        let session_principal = match self.get_or_create_session(&sid).await.ok().and_then(|s| {
            s.metadata
                .get("agent_id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        }) {
            Some(agent) => symbio_plugin_api::principal_of(Some(&agent)),
            None => symbio_sdk::authz::request_principal(ctx.as_ref()),
        };
        // 调度容量是能力方贡献的数（D-427，归 `task`）：统计是**读方**，按共享名读
        // 最近一条 `session.config`；读不到就用出厂值——与所有读快照的决策同一口径，
        // 本插件不再持有这一格的配置类型。（技能编译开关同属 `session.config` 的
        // 贡献键，但它的读数出口与轮首编译判都归 `skill`——`D-489` 起按报表列交值，
        // 本处不再读第二遍。）
        let task_cap = {
            let latest = snapshot.as_ref().and_then(|events| {
                events
                    .iter()
                    .rev()
                    .find(|e| e.kind == symbio_plugin_api::events::EVENT_SESSION_CONFIG)
                    .map(|e| &e.payload)
            });
            latest
                .and_then(|p| p.get("readyset_cap"))
                .and_then(serde_json::Value::as_u64)
                .map(|v| v as usize)
                .unwrap_or(symbio_plugin_api::TASK_CAP)
        };
        let stats = read(
            &sid,
            snapshot,
            || store.deep_snapshots(),
            viewer.as_deref(),
            &session_principal,
            task_cap,
        )?;
        Ok(PluginPayload::new(&stats))
    }
}

#[cfg(test)]
#[path = "stats.test.rs"]
mod tests;
