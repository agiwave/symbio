//! 默认能力收集器 —— [`Registry`] 的内存实现
//!
//! ## 为什么在 `providers/` 而不是 `symbio_core`
//!
//! 本类型的**写入者是全体插件**（经 `ctx` 的 `CAPABILITY_VISITOR` 键注册工具 / 模型服务 /
//! 系统提示词 / VDFS 挂载点），安装它的宿主（session 的 `collect_capabilities`）只是
//! 其中之一。它不是任何插件的内部物，故按「不存在第二种实现的机制底座直接组合具体类型」
//! 落在实现层——完整判据见 [`symbio_providers::collectors`] 的模块文档。

use super::approval::ApprovalGate;
use async_trait::async_trait;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::sync::Arc;
use symbio_core::{CapabilityFamily, CapabilityRow, Registry};
use tokio::sync::RwLock;

/// 默认能力管理器：内存注册表实现，一次会话请求一个实例
///
/// 全部注册通道共用**一张行表**（DE-025 §3）：
/// - `rows`：装全部族，按注册顺序保序。这条顺序就是送给模型的 `tools` 数组顺序：
///   注册序来自能力收集的广播序，写成哈希表时**每起一个进程重洗一次**，而工具面是
///   prompt 里最大的一段（实测 18KB 对正文 1.1KB），换序等于每轮把这段
///   前缀作废——这也是 D-057 一度记下「数组顺序每次不同」的根因。
/// - `families`：已登记族 → 首次声明的槽位策略。表按族名过滤（[`Registry::list`]），
///   槽位只用于「同名两套声明」的告警——族身份是名字，不是枚举值。
pub struct DefaultToolVisitor {
    rows: Arc<RwLock<IndexMap<String, Arc<CapabilityRow>>>>,
    families: Arc<RwLock<HashMap<CapabilityFamily, CapabilityFamily>>>,
}

impl DefaultToolVisitor {
    pub fn new() -> Self {
        Self {
            rows: Arc::new(RwLock::new(IndexMap::new())),
            families: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 登记族声明；同名但槽位策略不同是**登记冲突**，留一条告警再放行。
    async fn record_family(&self, family: CapabilityFamily) {
        let mut families = self.families.write().await;
        match families.get(&family) {
            Some(declared) if declared.is_single() != family.is_single() => {
                symbio_sdk::plugin_warn!(
                    "core",
                    "能力族 `{}` 的槽位声明冲突：先登记为 {}，本次为 {}（族身份按名字，冲突以先登记为准）",
                    family.name(),
                    if declared.is_single() { "single" } else { "multi" },
                    if family.is_single() { "single" } else { "multi" }
                );
            }
            Some(_) => {}
            None => {
                families.insert(family, family);
            }
        }
    }
}

impl Default for DefaultToolVisitor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Registry for DefaultToolVisitor {
    async fn put(&self, row: CapabilityRow) {
        // 闸门住在收口点：每条进表的**工具**能力都裹一层 [`ApprovalGate`]——
        // `meta` / `name` / `risk_for` 原样透传（见 `approval.rs`），故 `get` 与
        // `list` 看到的名字、声明位（三位）与风险档都不变；判定只发生在
        // 执行那一步（`capability_invoke` → `Capability::execute`）。
        // 非工具行（其余各族）不裹，原样入表。
        let row = match row.cap().cloned() {
            Some(cap) => CapabilityRow::tool(Arc::new(ApprovalGate::new(cap))),
            None => row,
        };
        self.record_family(row.family()).await;
        let name = row.name().to_string();
        let family = row.family();
        let mut rows = self.rows.write().await;
        // 跨族同名的覆盖是**记账错误**而不是正常覆盖（表键是全表唯一的名字空间，
        // 见 `Registry::put`）：后写者胜——但静默被顶掉的那一族不会以任何
        // 别的方式表现出来，所以这里留一条告警再覆盖。
        if let Some(old) = rows.get(&name) {
            if old.family() != family {
                symbio_sdk::plugin_warn!(
                    "core",
                    "能力表键 `{name}` 跨族同名覆盖：`{}` 行被 `{}` 行顶掉（族不参与键，见 Registry::put）",
                    old.family().name(),
                    family.name()
                );
            }
        }
        // 单槽移交：带生效位的行入表时，同族旧生效行先降位（模型族的「唯一生效者」）。
        // 行是 `Arc`，降位是换一个副本入表（`demote`），不是就地改位——
        // 否则 `list` 与 `get` 的读者会读到「两个生效者」的中间态。
        if row.active() {
            let stale: Vec<(String, Arc<CapabilityRow>)> = rows
                .values()
                .filter(|r| r.family() == family && r.active())
                .map(|r| (r.name().to_string(), Arc::new(r.demote())))
                .collect();
            for (name, demoted) in stale {
                rows.insert(name, demoted);
            }
        }
        // 同名覆盖（保留先注册槽位，IndexMap 语义与机制化之前逐字一致）
        rows.insert(name, Arc::new(row));
    }

    async fn get(&self, name: &str) -> Option<Arc<CapabilityRow>> {
        let rows = self.rows.read().await;
        rows.get(name).cloned()
    }

    async fn list(&self, family: CapabilityFamily) -> Vec<Arc<CapabilityRow>> {
        let rows = self.rows.read().await;
        rows.values()
            .filter(|row| row.family() == family)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
#[path = "tool_visitor.test.rs"]
mod tests;
