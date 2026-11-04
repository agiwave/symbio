//! 驱动入口：把宿主的通用能力束（[`DriveSupport`]）适配成环的私有上下文。
//!
//! 适配是唯一允许的翻译层：中性事实 ↔ 五类事实、中性行 ↔ 债行。环本身仍然只认
//! 自己的类型；宿主因此不必认识 `ring` 里的任何东西（`DE-051` 机制形状）。

use std::sync::Arc;

use serde_json::Value;
use symbio_kernel::plugin::{
    PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse, PluginPayload,
};
use symbio_plugin_api::drive::{
    DriveFact, DriveOutcome, DriveReport, DriveRow, DriveSupport, DRIVE_REPORT, DRIVE_SCOPE,
    DRIVE_SUPPORT,
};
use symbio_plugin_api::port::{PortExecutor, PortFactEmit, PortFactReader};

use crate::ring::{drive, DebtKey, DebtRow, DebtSource, DriveCtx, Fact, FactKind, FactView};

/// 环入口：宿主经上下文键给出能力束与可见域，环把报告写回上下文。
pub(crate) async fn run(
    ctx: Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload> {
    let Some(support) = ctx.get(DRIVE_SUPPORT) else {
        return Err(PluginError::NotFound("缺少驱动能力束".to_string()));
    };
    let Some(scope) = ctx.get(DRIVE_SCOPE) else {
        return Err(PluginError::NotFound("缺少驱动可见域".to_string()));
    };

    let adapter = SupportCtx {
        support: Arc::clone(&support),
        scope,
        source: SupportSource { support },
    };
    let report = drive(&adapter, scope).await;
    let outcome = match report.outcome {
        crate::ring::DriveOutcome::NoDebt => DriveOutcome::NoDebt,
        crate::ring::DriveOutcome::Waiting(keys) => {
            DriveOutcome::Waiting(keys.into_iter().map(to_neutral_key).collect())
        }
    };
    ctx.set(
        DRIVE_REPORT,
        DriveReport {
            scope,
            dispatched: report.dispatched,
            outcome,
        },
    );
    Ok(PluginPayload::new(&Vec::<Value>::new()))
}

/// 中性债键 → 环债键。
fn to_ring_key(key: &symbio_plugin_api::drive::DriveKey) -> DebtKey {
    DebtKey {
        scope: key.scope,
        port: key.port,
        id: key.id.clone(),
        attempt: key.attempt,
    }
}

/// 环债键 → 中性债键。
fn to_neutral_key(key: DebtKey) -> symbio_plugin_api::drive::DriveKey {
    symbio_plugin_api::drive::DriveKey {
        scope: key.scope,
        port: key.port,
        id: key.id,
        attempt: key.attempt,
    }
}

/// 环的上下文：每次驱动现造，持有能力束的克隆，不跨步保留状态。
struct SupportCtx {
    support: Arc<dyn DriveSupport>,
    scope: u64,
    source: SupportSource,
}

impl DriveCtx for SupportCtx {
    fn facts(&self) -> Vec<Fact> {
        self.support
            .facts(self.scope)
            .into_iter()
            .filter_map(to_ring_fact)
            .collect()
    }

    fn append(&self, fact: Fact) {
        self.support.append(DriveFact {
            scope: fact.scope,
            kind: fact.kind.as_str(),
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
        self.support.executor(port)
    }

    fn now_ms(&self) -> u64 {
        self.support.now_ms()
    }

    fn concurrency(&self) -> usize {
        self.support.concurrency()
    }

    fn outlet(&self) -> PortFactEmit {
        self.support.outlet(self.scope)
    }

    fn fact_reader(&self) -> Option<Arc<dyn PortFactReader>> {
        self.support.reader(self.scope)
    }
}

/// 中性事实 → 五类事实；不是五类之一 ⇒ 丢弃（环不落别的事实）。
fn to_ring_fact(fact: DriveFact) -> Option<Fact> {
    let kind = FactKind::parse(fact.kind)?;
    Some(Fact {
        scope: fact.scope,
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
    fn rows(&self, facts: &FactView<'_>, scope: u64) -> Vec<DebtRow> {
        let mut out = Vec::new();
        for row in self.support.rows(scope) {
            let after = match &row.after {
                None => None,
                Some(id) => match closing_key(facts, scope, id) {
                    Some(key) => Some(key),
                    None => continue,
                },
            };
            out.push(to_ring_row(row, after));
        }
        out
    }

    fn exists(&self, kind: &str, scope: u64) -> bool {
        self.support.exists(kind, scope)
    }
}

/// 该身份在本可见域最近的收束债键。
fn closing_key(facts: &FactView<'_>, scope: u64, id: &str) -> Option<DebtKey> {
    facts
        .all()
        .iter()
        .rev()
        .find(|f| f.kind.closes() && f.key.scope == scope && f.key.id == id)
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
        unless: row.unless,
        when_no_open: row.when_no_open,
        lease_ms: row.lease_ms,
        max_attempts: row.max_attempts,
    }
}
