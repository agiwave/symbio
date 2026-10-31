//! 轮首检索 —— 能力在请求开口之前把**可见切片**折成有次序的读数（DE-026 D-264）。
//!
//! ## 它补的是哪一格
//!
//! 「这一轮该带哪些过往进上下文」是检索的判断：折叠（哪些条目还活着）、混合打分
//! （语义维 + 新近维）、扫描预算（超预算 ⇒ 只给出**一部分**并说出来）。这些判据过去
//! 住 `symbio-plugin-loop` 的 `recall.rs`——每换一副检索策略就要改控制面一次。
//! 本模块把「怎么折」交回能力包，宿主只留两件它本来就该做的事：
//!
//! ```text
//! 控制面 ──取快照、按可见域收放──▶ 可见切片（值）
//! 能力包 ──submit_retriever!────▶ inventory（编译期随二进制生效）
//! 宿主     ──retrieve_all(切片, 发言)──▶ 自有读数 ──▶ 债键格 / 段渲染 / 收束值
//! ```
//!
//! ## 与 D-251 的分界：为什么这是一副面，不是一个闸
//!
//! D-251 否掉的 `PromptGate` 有三样东西本面一样都没有：它**替宿主改写别人**（摘行权）；
//! 它的入参把业务视图的行包进端口形状；它的提议者只有一条能力。本面只做一件事——
//! **给定值，折成读数**：入参是协议值（事件切片 + 发言文本），产出是自有数据
//! （`Retrieval`），既不写事实、也不裁决「这一行进不进」——摘除与渲染各走既有的面
//! （D-262 的名字读、I-2 的 provider）。而它服务的不只一条能力：RAG / 日历 / 项目文档
//! 要的正是同一副形状（「轮首、可见切片进、排序读数出」），长期记忆检索是它的第一例。
//!
//! ## 可见域收放为什么不在包内
//!
//! 跨会话枚举（存储层的快照取数）与逐条可见判定（`thread_private` / `shared` / `public`
//! 的取值口径住 core 的读侧闸）都是**隔离面**（I-8）：判错的代价是泄漏，不是少读。
//! 所以入参交的是**收放之后**的切片——包内不需要、也拿不到「哪个会话是谁的」这格事实。
//!
//! ## 产出为什么是自有值
//!
//! 读数的寿命比一次收集长：宿主要把条目清单落进召回阶段格（技能行的债键面）、
//! 要把它借给两种节奏的 provider、还要在收束时带给贡献者。`String` 而不是 `&str`：
//! 切片随请求结束，读数随本轮存活。

use symbio_core::Event;

/// 本轮检索的**输入值包**：控制面取好的两个值，构造一次、各检索者共用。
#[derive(Debug, Clone, Copy)]
pub struct RetrievalRequest<'a> {
    /// 可见切片：跨会话枚举与可见域收放都做完之后的事件集（新近优先的原始次序）。
    pub events: &'a [Event],
    /// 本轮用户发言（打分的语义维）。`None` = 没有新发言（resume / 心跳），
    /// 检索据此退化为纯新近序——与「发言恰好是空串」不是一回事。
    pub query: Option<&'a str>,
}

/// 检索出的一条记忆（**自有**形状：行得通，因为读数比输入切片活得久）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retrieved {
    /// 来源事实的存储 `seq`——与 [`crate::RecallLine::seq`] 同一格身份。
    pub seq: u64,
    pub content: String,
    pub tag: String,
}

/// 一条能力的检索读数。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Retrieval {
    /// 本次检索是否没扫完（预算耗尽等）——降级必须随读数走，与 [`crate::RecallFacts`]
    /// 的 `degraded` 同一格语义（DE-036 §6 验收 2：不允许空且不标）。
    pub degraded: bool,
    /// 排好序的条目。**次序是包内判据的结论**：宿主不再排第二遍。
    pub entries: Vec<Retrieved>,
}

/// 能力对**轮首这一刻**的检索实现。
///
/// 一个实现 = 一副折叠 + 一次 [`submit_retriever!`](crate::submit_retriever)；
/// 加一副检索只改能力包。纯函数：不持状态、不碰事实源、不发 IO——快照的枚举与
/// 可见域的收放在调用方（见模块文档）。
pub trait Retriever: Sync {
    /// 收集次序的依据：多条检索按 owner 字典序合并（DE-025 §3 第 4 条）。
    fn owner(&self) -> &'static str;

    /// 本轮的读数。返回 `None` = 本检索在这份切片上没有可交的东西（**默认**）。
    fn retrieve(&self, _at: &RetrievalRequest<'_>) -> Option<Retrieval> {
        None
    }
}

/// 一条检索的链接期提交。
pub struct RetrieverDecl {
    pub retriever: &'static dyn Retriever,
}

inventory::collect!(RetrieverDecl);

/// 提交一条轮首检索。在插件 crate 的模块作用域调用一次即可。
///
/// ```ignore
/// symbio_plugin_api::submit_retriever!(RecallRetrieval);
/// ```
#[macro_export]
macro_rules! submit_retriever {
    ($r:expr) => {
        $crate::inventory::submit! {
            $crate::retrieval::RetrieverDecl { retriever: &$r }
        }
    };
}

/// 已链接进来的全部检索者，**按 owner 字典序**（同 owner 保持提交序）。
///
/// 排序在读取侧做而不是装配期做一次：表是 `inventory` 的编译期集合，本仓不持有第二个
/// 可变注册表（与 `effects::handlers` 同一条纪律）。
pub fn retrievers() -> Vec<&'static dyn Retriever> {
    let mut all: Vec<&'static dyn Retriever> = inventory::iter::<RetrieverDecl>()
        .map(|d| d.retriever)
        .collect();
    all.sort_by_key(|r| r.owner());
    all
}

/// 收集本轮检索：按 owner 序询问全部检索者，**合并成一份读数**。
///
/// 合并规则只有两条，都是机械的：条目按 owner 序拼接（不在宿主排第二遍次序），
/// `degraded` 取**任一**（任何一副折叠没扫完，这一轮就必须标降级——「只给了一部分」
/// 不许静默）。全部为空 ⇒ `None`（与本轮没有召回同一个形状）。
pub fn retrieve_all(at: &RetrievalRequest<'_>) -> Option<Retrieval> {
    let mut out = Retrieval::default();
    for retriever in retrievers() {
        if let Some(retrieved) = retriever.retrieve(at) {
            out.degraded |= retrieved.degraded;
            out.entries.extend(retrieved.entries);
        }
    }
    (!out.entries.is_empty()).then_some(out)
}

#[cfg(test)]
#[path = "retrieval.test.rs"]
mod tests;
