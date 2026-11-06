//! 驱动入口：把宿主的通用能力束（[`DriveSupport`]）适配成环的私有上下文。
//!
//! 适配是唯一允许的翻译层：中性事实 ↔ 五类事实、中性行 ↔ 债行。环本身仍然只认
//! 自己的类型；宿主因此不必认识 `ring` 里的任何东西（`DE-051` 机制形状）。
//!
//! 这个执行者只在本 crate 内可见，经 `traverse` 往 `drive` 族登记一行；宿主按族
//! `list` 取回后调用，能力束是参数。

use std::sync::Arc;

use symbio_plugin_api::drive::{
    DriveFact, DriveOutcome, DriveReport, DriveRow, DriveRunner, DriveSupport,
};
use symbio_plugin_api::port::{PortExecHandles, PortExecutor, PortFactEmit, PortFactReader};

use crate::ring::{drive, DebtKey, DebtRow, DebtSource, DriveCtx, Fact, FactKind, FactView};

/// 环的执行者：宿主把能力束交进来，报告原样回去。
pub(crate) struct LoopDrive;

#[async_trait::async_trait]
impl DriveRunner for LoopDrive {
    async fn drive(&self, support: Arc<dyn DriveSupport>) -> Result<DriveReport, String> {
        let adapter = SupportCtx {
            source: SupportSource {
                support: Arc::clone(&support),
            },
            support,
        };
        let report = drive(&adapter).await;
        let outcome = match report.outcome {
            crate::ring::DriveOutcome::NoDebt => DriveOutcome::NoDebt,
            crate::ring::DriveOutcome::Waiting(keys) => {
                DriveOutcome::Waiting(keys.into_iter().map(to_neutral_key).collect())
            }
        };
        Ok(DriveReport {
            dispatched: report.dispatched,
            outcome,
        })
    }
}

/// 中性债键 → 环债键。
fn to_ring_key(key: &symbio_plugin_api::drive::DriveKey) -> DebtKey {
    DebtKey {
        port: key.port.clone(),
        id: key.id.clone(),
        attempt: key.attempt,
    }
}

/// 环债键 → 中性债键。
fn to_neutral_key(key: DebtKey) -> symbio_plugin_api::drive::DriveKey {
    symbio_plugin_api::drive::DriveKey {
        port: key.port,
        id: key.id,
        attempt: key.attempt,
    }
}

/// 环的上下文：每次驱动现造，持有能力束的克隆，不跨步保留状态。
struct SupportCtx {
    support: Arc<dyn DriveSupport>,
    source: SupportSource,
}

impl DriveCtx for SupportCtx {
    fn facts(&self) -> Vec<Fact> {
        self.support
            .facts()
            .into_iter()
            .filter_map(to_ring_fact)
            .collect()
    }

    fn append(&self, fact: Fact) {
        self.support.append(DriveFact {
            kind: fact.kind.as_str().to_string(),
            key: to_neutral_key(fact.key),
            payload: fact.payload,
            at_ms: fact.at_ms,
            reply: fact.reply,
        });
    }

    fn source(&self) -> &dyn DebtSource {
        &self.source
    }

    fn executor(&self, port: &str) -> Option<Arc<dyn PortExecutor>> {
        // 行执行体是机制自己的执行体（与端口提供者同一条解析面，D-338）。
        if crate::plugin_rows::is_row_port(port) {
            return Some(crate::plugin_rows::executor());
        }
        self.support.executor(port)
    }

    fn now_ms(&self) -> u64 {
        self.support.now_ms()
    }

    fn concurrency(&self) -> usize {
        self.support.concurrency()
    }

    fn outlet(&self) -> PortFactEmit {
        self.support.outlet()
    }

    fn fact_reader(&self) -> Option<Arc<dyn PortFactReader>> {
        self.support.reader()
    }

    fn exec_handles(&self) -> PortExecHandles {
        self.support.exec_handles()
    }
}

/// 中性事实 → 五类事实；不是五类之一 ⇒ 丢弃（环不落别的事实）。
fn to_ring_fact(fact: DriveFact) -> Option<Fact> {
    let kind = FactKind::parse(&fact.kind)?;
    Some(Fact {
        kind,
        key: to_ring_key(&fact.key),
        payload: fact.payload,
        at_ms: fact.at_ms,
        reply: fact.reply,
    })
}

/// 债表数据源：从能力束取行，`after` 由当前事实投影解析成环债键。
struct SupportSource {
    support: Arc<dyn DriveSupport>,
}

impl DebtSource for SupportSource {
    fn rows(&self, facts: &FactView<'_>) -> Vec<DebtRow> {
        let mut out = Vec::new();
        for row in self.support.rows() {
            let after = match &row.after {
                None => None,
                Some(id) => match closing_key(facts, id) {
                    Some(key) => Some(key),
                    None => continue,
                },
            };
            out.push(to_ring_row(row, after));
        }
        // `Decl` 行参与选债（D-304 裁决六）：`by == plugin_row` 的行由机制自己执行，
        // 世界语义（欠什么、何时算还）全在声明里，这里只做形状适配。
        let events = crate::plugin_rows::decl_events(self.support.facts());
        out.extend(crate::plugin_rows::decl_rows(
            &events,
            self.support.now_ms(),
        ));
        out
    }

    fn exists(&self, kind: &str) -> bool {
        self.support.exists(kind)
    }

    /// 与宿主的事实读面同一份数据，只多问一句位置（宿主按本轮计划快照作答）。
    fn exists_since(&self, kind: &str, since_ms: u64) -> bool {
        self.support.exists_since(kind, since_ms)
    }

    fn exists_id(&self, kind: &str, id: &str) -> bool {
        self.support.exists_id(kind, id)
    }
}

/// 该身份在本会话最近的收束债键。
fn closing_key(facts: &FactView<'_>, id: &str) -> Option<DebtKey> {
    facts
        .all()
        .iter()
        .rev()
        .find(|f| f.kind.closes() && f.key.id == id)
        .map(|f| f.key.clone())
}

/// 中性行 → 债行（形状相同，只差 `after` 已解析）。
fn to_ring_row(row: DriveRow, after: Option<DebtKey>) -> DebtRow {
    DebtRow {
        name: row.name,
        port: row.port,
        id: row.id,
        payload: row.payload,
        after,
        when: row.when,
        when_id: row.when_id,
        unless: row.unless,
        when_no_open: row.when_no_open,
        lease_ms: row.lease_ms,
        max_attempts: row.max_attempts,
    }
}
