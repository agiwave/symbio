//! 默认能力收集器 —— [`Registry`] 的内存实现
//!
//! ## 为什么在 `providers/` 而不是 `symbio_core`
//!
//! 本类型的**写入者是全体插件**（经 `ctx` 的 `CAPABILITY_VISITOR` 键注册工具 / 模型服务 /
//! 系统提示词 / VDFS 挂载点），安装它的宿主（session 的 `collect_capabilities`）只是
//! 其中之一。它不是任何插件的内部物，故按「不存在第二种实现的机制底座直接组合具体类型」
//! 落在实现层——完整判据见 [`crate::providers::collectors`] 的模块文档。

use super::approval::ApprovalGate;
use crate::symbio_core::{CapabilityRow, CapabilityRowKind, Registry};
use async_trait::async_trait;
use indexmap::IndexMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 默认能力管理器：内存注册表实现，一次会话请求一个实例
///
/// 全部注册通道共用**一张行表**（AR-025 §3）：
/// - `rows`：装**工具族 / 模型族 / 提示词族 / 挂载族**（挂载点与根同族，按根位区分），
///   按注册顺序保序。这条顺序就是送给模型的 `tools` 数组顺序：注册序来自能力收集的广播序，
///   写成哈希表时**每起一个进程重洗一次**，而工具面是 prompt 里最大的一段
///   （设计文档 实测 18KB 对正文 1.1KB），换序等于每轮把这段
/// 前缀作废——这也是 D-057 一度记下「数组顺序每次不同」的根因。
///   模型行（4b）一行一个已启用 provider，**当次生效者带生效位**——生效者是目录里的
///   一员不再是纪律而是结构（见 `capability/model.rs`）；提示词行（4c）一行一段文字，
///   表序即送达顺序（见 `capability/prompt.rs`）；挂载行（4d）一行一个目录，
///   **根是带根位的那一行**（见 `capability/mount.rs`）。选项族随 4e 入表。
pub struct DefaultToolVisitor {
    rows: Arc<RwLock<IndexMap<String, Arc<CapabilityRow>>>>,
}

impl DefaultToolVisitor {
    pub fn new() -> Self {
        Self {
            rows: Arc::new(RwLock::new(IndexMap::new())),
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
        // `list` 看到的名字、声明位（ 三位）与风险档都不变；判定只发生在
        // 执行那一步（`capability_invoke` → `Capability::execute`，）。
        // 非工具行（4b–4e 落位的各族）不裹，原样入表。
        let row = match row.cap().cloned() {
            Some(cap) => CapabilityRow::tool(Arc::new(ApprovalGate::new(cap))),
            None => row,
        };
        let name = row.name().to_string();
        let mut rows = self.rows.write().await;
        // 跨族同名的覆盖是**记账错误**而不是正常覆盖（表键是全表唯一的名字空间，
        // 见 `Registry::put`）：后写者胜——但静默被顶掉的那一族不会以任何
        // 别的方式表现出来，所以这里留一条告警再覆盖。
        if let Some(old) = rows.get(&name) {
            if old.kind() != row.kind() {
                crate::plugin_warn!(
                    "core",
                    "能力表键 `{name}` 跨族同名覆盖：{:?} 行被 {:?} 行顶掉（族不参与键，见 Registry::put）",
                    old.kind(),
                    row.kind()
                );
            }
        }
        // 单槽移交：带生效位的行入表时，同族旧生效行先降位（模型族的「唯一生效者」）。
        // 行是 `Arc`，降位是换一个副本入表（`demote`），不是就地改位——
        // 否则 `list` 与 `get` 的读者会读到「两个生效者」的中间态。
        if row.active() {
            let stale: Vec<(String, Arc<CapabilityRow>)> = rows
                .values()
                .filter(|r| r.kind() == row.kind() && r.active())
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

    async fn list(&self, kind: CapabilityRowKind) -> Vec<Arc<CapabilityRow>> {
        let rows = self.rows.read().await;
        rows.values()
            .filter(|row| row.kind() == kind)
            .cloned()
            .collect()
    }
}

#[cfg(test)]
#[path = "tool_visitor.test.rs"]
mod tests;
