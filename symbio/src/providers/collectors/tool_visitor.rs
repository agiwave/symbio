//! 默认能力收集器 —— [`CapabilityVisitor`] 的内存实现
//!
//! ## 为什么在 `providers/` 而不是 `symbio_core`
//!
//! 本类型的**写入者是全体插件**（经 `ctx` 的 `CAPABILITY_VISITOR` 键注册工具 / 模型服务 /
//! 系统提示词 / VDFS 挂载点），安装它的宿主（session 的 `collect_capabilities`）只是
//! 其中之一。它不是任何插件的内部物，故按「不存在第二种实现的机制底座直接组合具体类型」
//! 落在实现层——完整判据见 [`crate::providers::collectors`] 的模块文档。

use super::approval::ApprovalGate;
use crate::symbio_core::{CapabilityRow, CapabilityRowKind, CapabilityVisitor, VdfsProvider};
use async_trait::async_trait;
use indexmap::IndexMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 默认能力管理器：内存注册表实现，一次会话请求一个实例
///
/// 除行表外，同时承载两组单族注册（与工具同一 traverse 收集机制）：
/// - `rows`：**统一行表**（AR-025 §3）——装**工具族 / 模型族 / 提示词族**，按注册
///   顺序保序。这条顺序就是送给模型的 `tools` 数组顺序：注册序来自能力收集的广播序，
///   写成哈希表时**每起一个进程重洗一次**，而工具面是 prompt 里最大的一段
///   （`docs/006-ref/PROMPT_SHAPE.md` 实测 18KB 对正文 1.1KB），换序等于每轮把这段
///   前缀作废——这也是 ADR-057 一度记下「数组顺序每次不同」的根因。
///   模型行（4b）一行一个已启用 provider，**当次生效者带生效位**——生效者是目录里的
///   一员不再是纪律而是结构（见 `capability/model.rs`）；提示词行（4c）一行一段文字，
///   表序即送达顺序（见 `capability/prompt.rs`）。挂载 / 选项各族随 4d–4e 陆续入这张表
///   （每族一个 payload 变体，trait 三方法不变），目前仍住下方槽位。
/// - `vdfs_providers`：VDFS 挂载点（按挂载名去重，`order` 升序对外）。
///   这是 **LLM 可控挂载机制**的清单（子智能体经 `SubAgentVisitor` 在此加
///   `agent/<id>/` 作用域前缀，预留按作用域 / 白名单裁剪 LLM 可见资源）；
///   前端 / 系统链路**不经过它**（走 `Plugin::get_vfs_provider`）。当前
///   `vdfs_*` 工具取根走 `vdfs_root` 单槽，本清单暂无消费方——**预留，勿删**。
/// - `vdfs_root`：VDFS 根 provider（单槽；组合容器注册，访问层据此转发）
pub struct DefaultToolVisitor {
    rows: Arc<RwLock<IndexMap<String, Arc<CapabilityRow>>>>,
    vdfs_providers: Arc<RwLock<IndexMap<String, Arc<dyn VdfsProvider>>>>,
    vdfs_root: Arc<RwLock<Option<Arc<dyn VdfsProvider>>>>,
}

impl DefaultToolVisitor {
    pub fn new() -> Self {
        Self {
            rows: Arc::new(RwLock::new(IndexMap::new())),
            vdfs_providers: Arc::new(RwLock::new(IndexMap::new())),
            vdfs_root: Arc::new(RwLock::new(None)),
        }
    }
}

impl Default for DefaultToolVisitor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityVisitor for DefaultToolVisitor {
    async fn put(&self, row: CapabilityRow) {
        // 闸门住在收口点：每条进表的**工具**能力都裹一层 [`ApprovalGate`]——
        // `meta` / `name` / `risk_for` 原样透传（见 `approval.rs`），故 `get` 与
        // `list` 看到的名字、声明位（T-158 三位）与风险档都不变；判定只发生在
        // 执行那一步（`capability_invoke` → `Capability::execute`，T-138）。
        // 非工具行（4b–4e 落位的各族）不裹，原样入表。
        let row = match row.cap().cloned() {
            Some(cap) => CapabilityRow::tool(Arc::new(ApprovalGate::new(cap))),
            None => row,
        };
        let name = row.name().to_string();
        let mut rows = self.rows.write().await;
        // 跨族同名的覆盖是**记账错误**而不是正常覆盖（表键是全表唯一的名字空间，
        // 见 `CapabilityVisitor::put`）：后写者胜——但静默被顶掉的那一族不会以任何
        // 别的方式表现出来，所以这里留一条告警再覆盖。
        if let Some(old) = rows.get(&name) {
            if old.kind() != row.kind() {
                crate::plugin_warn!(
                    "core",
                    "能力表键 `{name}` 跨族同名覆盖：{:?} 行被 {:?} 行顶掉（族不参与键，见 CapabilityVisitor::put）",
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

    async fn register_vdfs_provider(&self, name: &str, provider: Arc<dyn VdfsProvider>) {
        let mut slot = self.vdfs_providers.write().await;
        // 同目录名覆盖（保留先注册槽位，IndexMap 语义与工具注册一致）
        slot.insert(name.to_string(), provider);
    }

    async fn list_vdfs_providers(&self) -> Vec<(String, Arc<dyn VdfsProvider>)> {
        let slot = self.vdfs_providers.read().await;
        let out: Vec<(String, Arc<dyn VdfsProvider>)> = slot
            .iter()
            .map(|(name, p)| (name.clone(), p.clone()))
            .collect();
        // 注册顺序即稳定顺序（导航排序归容器：按 `PluginMeta::order` 在合成目录时排）
        out
    }

    async fn get_vdfs_provider(&self, name: &str) -> Option<Arc<dyn VdfsProvider>> {
        let slot = self.vdfs_providers.read().await;
        slot.get(name).cloned()
    }

    async fn register_vdfs_root(&self, provider: Arc<dyn VdfsProvider>) {
        let mut slot = self.vdfs_root.write().await;
        *slot = Some(provider);
    }

    async fn get_vdfs_root(&self) -> Option<Arc<dyn VdfsProvider>> {
        let slot = self.vdfs_root.read().await;
        slot.clone()
    }
}

#[cfg(test)]
#[path = "tool_visitor.test.rs"]
mod tests;
