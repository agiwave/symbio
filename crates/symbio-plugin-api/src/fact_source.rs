//! 事实源契约 —— 装配层创建、插件经上下文取用（唯一写口 + 唯一读面）。
//!
//! ## 信封由事实源生成
//!
//! 写方只交 [`FactDraft`]（幂等键 / 事件名 / 格位 / 载荷 / 溯源），**不交信封**：
//! `event_id` / `seq` / `ts` / `cost_ms` 一律由事实源在入库时生成。写方没有
//! 「指定信封」的入口，因此不会撞 id、序号归属也不会有第二份说法。
//!
//! ## `range` 不带查询条件
//!
//! [`FactSource::range`] 只有「从序号往后读全史」这一种口径：没有 filter / where /
//! 窗口对象。能带条件就会有人传 `since` / `until` / `kind`，契约随即变成查询层。
//!
//! ## 开面方向
//!
//! 装配层经 [`FactSourceOpener`] **创建**唯一事实源（`inventory` 注册，与
//! `submit_object_creator!` 同手法），同一 `Arc<dyn FactSource>` 交给宿主与
//! [`FACT_SOURCE`] 上下文键；插件从上下文取，**不自建 store**。

use std::sync::Arc;

use symbio_kernel::event::Seq as KernelSeq;
use symbio_kernel::store::{MemoryStore, Store};

use crate::FactDraft;

/// 写方主体：完整主体名（如 `person/local` / `agent:main`）。
pub type Principal = String;

/// 事实（= 内核信封 `Event`）：读面交出的就是入库后的完整事实。
pub type Fact = symbio_kernel::event::Event;

/// 事实源：一个会话域的事实读写入口。
///
/// 五个方法，多一个都不许。需要第四条以外的形状时先报「缺什么 + 为什么现有
/// 参数表达不了」，不在实现里顺手加。
pub trait FactSource: Send + Sync + 'static {
    /// 落一条草稿；返回分配到的 `seq`。
    fn append(&self, actor: Principal, draft: FactDraft) -> Result<u64, String>;

    /// 本会话的全部事实（按号序）。
    fn facts(&self) -> Vec<Fact>;

    /// 从序号 `from_seq` 起的完整史（含被取代的前缀）。
    fn range(&self, from_seq: u64) -> Vec<Fact>;

    /// 下一个待分配的序号。
    fn head(&self) -> u64;

    /// 该前缀已固化，可回收；**不得影响 `before_seq` 之后的可读性**。
    fn retire_prefix(&self, before_seq: u64) -> Result<(), String>;
}

/// 事实源开面：装配层调用一次，拿到该会话的唯一句柄。
pub trait FactSourceOpener: Send + Sync + 'static {
    /// 为 `session_id` 打开事实源。
    fn open(&self, session_id: &str) -> Result<Arc<dyn FactSource>, String>;
}

/// `inventory` 登记项：实现者随二进制链接期生效。
pub struct FactSourceOpenerDecl {
    pub opener: &'static dyn FactSourceOpener,
}

inventory::collect!(FactSourceOpenerDecl);

/// 已登记的事实源开面；构建里没有实现者时为 `None`。
pub fn declared_fact_source_opener() -> Option<&'static dyn FactSourceOpener> {
    inventory::iter::<FactSourceOpenerDecl>()
        .next()
        .map(|decl| decl.opener)
}

/// 上下文键：装配层下发的同一 `Arc<dyn FactSource>`。
pub struct FactSourceKey;

impl symbio_kernel::keys::SymbioKey for FactSourceKey {
    type Value = Arc<dyn FactSource>;
    const WIRE: bool = false;

    fn name(&self) -> &'static str {
        "fact_source"
    }

    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }

    fn format(&self, _v: &Self::Value) -> String {
        String::new()
    }
}

/// 上下文键的唯一实例。
pub const FACT_SOURCE: FactSourceKey = FactSourceKey;

/// 内存事实源：**只作插件单测 / 平凡基线的假事实源**（与生产实现走同一契约）。
///
/// 它没有会话身份，因此**没有资格推环**：这里不踢驱动。宿主装配路径上的事实源
/// 一律经 [`FactSourceOpener`] 打开、由 `SessionFactSource` 承担；把 `MemoryStore`
/// 当生产写口是错误使用。
impl FactSource for MemoryStore<Fact> {
    fn append(&self, actor: Principal, draft: FactDraft) -> Result<u64, String> {
        let event = draft_into_event(actor, draft);
        Store::append(self, event)
            .map(|seq| seq.value())
            .map_err(|e| format!("{e:?}"))
    }

    fn facts(&self) -> Vec<Fact> {
        Store::range(self, KernelSeq::new(0))
    }

    fn range(&self, from_seq: u64) -> Vec<Fact> {
        Store::range(self, KernelSeq::new(from_seq))
    }

    fn head(&self) -> u64 {
        Store::head(self).value()
    }

    fn retire_prefix(&self, _before_seq: u64) -> Result<(), String> {
        Ok(())
    }
}

/// 草稿 + 主体 → 入库事件（信封的号与时刻由事实源填）。
///
/// `cost_ms` 与号 / 时刻同属**信封**：写方把这次往返的实测耗时放在载荷的
/// `cost_ms` 字段（那是它唯一能给的值），入库前提到信封。读方（`slo` 的分位数、
/// 成本台账、`I3` 的预算判据）只认信封字段——载荷里那份是给重建面留的原始读数。
pub fn draft_into_event(actor: Principal, draft: FactDraft) -> Fact {
    let mut event = Fact::pending(draft.key, draft.kind, draft.entity, draft.verb, actor);
    event.payload = draft.payload;
    event.produced_by = draft.produced_by;
    if let Some(cost) = event
        .payload
        .get("cost_ms")
        .and_then(serde_json::Value::as_u64)
    {
        event.cost_ms = cost;
    }
    event
}

/// 登记宏（`inventory::submit!`）。
///
/// - 参数是 `&'static dyn FactSourceOpener` 的来源（通常是 `static` 单元结构）；
/// - 运行时经 [`declared_fact_source_opener`] 取回。
#[macro_export]
macro_rules! submit_fact_source_opener {
    ($opener:expr) => {
        $crate::inventory::submit! {
            $crate::fact_source::FactSourceOpenerDecl { opener: &$opener }
        }
    };
}
