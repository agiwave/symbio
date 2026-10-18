//! 子 Agent 注册的**代理层**：给一切注册项加上来源前缀
//!
//! ## 为什么是前缀，而不是"后注册者胜"
//!
//! 插件容器遍历子插件用的是 `HashMap`，**顺序不确定**。若让系统树与子 Agent 树
//! 用**同一个名字**注册（例如 `skill` 插件的每个实例都注册 `read_skill`），
//! 谁生效就取决于容器那一次随机的遍历顺序——结果不可预测。
//!
//! 加前缀后**名字根本不冲突**，于是：
//!
//! - 并集天然成立（两份都注册、都生效）；
//! - 判定与顺序无关，不需要给注册体系引入优先级概念；
//! - 也不必修改 `symbio_core` 的架构定义——本模块只在 agent 插件内做一层代理。
//!
//! ## 两套前缀形态
//!
//! | 注册项 | 形态 | 理由 |
//! |---|---|---|
//! | 系统提示词段 / VDFS 挂载点 | `agent/<id>/<name>` | 只作键 / 目录名，`/` 可读且天然分层 |
//! | 工具 | `agent_<safe_id>_<name>` | 工具名进模型的 function-calling 协议，多数厂商只接受 `[A-Za-z0-9_-]` |
//!
//! ## 单槽位 / 系统级注册不转发
//!
//! VDFS 根（`register_vdfs_root`）与整个**模型族**（模型行的生效位 + 目录）都是
//! **单槽**或系统级事实。子 Agent 的插件树里也有容器（它会把自己登记成 VDFS 根）
//! 与 model 插件实例，原样转发会让子 Agent **劫持**整个 `<根>` 与模型服务——
//! 系统侧的全部挂载点瞬间消失。因此这两项**丢弃**（记 debug）。
//!
//! 模型族的丢弃写在 [`CapabilityVisitor::put`] 上（按行的族别判），因为 4b 之后
//! 模型服务**不再是另一套方法**：model 插件逐行 `put` 写入，作用域这一层拦在同一个
//! 入口上——写入口只有一个，作用域规则也就只有一处（见 `capability/model.rs`）。
//!
//! 同理，VDFS 挂载点走前缀而非覆盖：VDFS 是用户浏览 / 编辑面，子 Agent 的 `work`
//! 若盖掉系统的 `work`，用户就改不到工作区记忆了——那是回归。

use crate::symbio_core::VdfsProvider;
use crate::symbio_core::{
    Capability, CapabilityMeta, CapabilityRow, CapabilityRowKind, CapabilityVisitor, ExecEnv,
    PluginError, PluginInvokeRequest,
};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// 子 Agent 插件树的注册代理
pub struct SubAgentVisitor {
    inner: Arc<dyn CapabilityVisitor>,
    /// 智能体 id（目录名）
    agent_id: String,
}

impl SubAgentVisitor {
    pub fn new(inner: Arc<dyn CapabilityVisitor>, agent_id: impl Into<String>) -> Self {
        Self {
            inner,
            agent_id: agent_id.into(),
        }
    }

    /// 提示词段 / VDFS 挂载点的名字：`agent/<id>/<name>`
    fn seg(&self, name: &str) -> String {
        format!("agent/{}/{}", self.agent_id, name)
    }

    /// 工具名：`agent_<safe_id>_<name>`
    fn tool(&self, name: &str) -> String {
        format!("agent_{}_{}", safe_segment(&self.agent_id), name)
    }
}

/// 把 id 压进 `[A-Za-z0-9_-]`（工具名的协议字符集）
fn safe_segment(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// 改名包装：只改 `meta().name`，执行原样委托
struct PrefixedCapability {
    inner: Arc<dyn Capability>,
    name: String,
}

#[async_trait]
impl Capability for PrefixedCapability {
    fn meta(&self) -> CapabilityMeta {
        let mut meta = self.inner.meta();
        meta.name = self.name.clone();
        meta
    }

    /// 透传给被改名的工具：装饰器只换名字，不重判风险。
    ///
    /// 默认实现读 `self.meta().effective_risk()`——那是**静态**档。风险随参数变化
    /// 的工具（shell 按命令分级）覆写 `risk_for`，改名包装若不显式转发，注册收口点
    /// 的审批闸就读到默认档而丢掉动态分级（T-138：闸门住在 `DefaultToolVisitor`
    /// 之后，这条透传路径必须完整）。
    fn risk_for(&self, args: &serde_json::Value) -> crate::symbio_core::CapabilityRiskLevel {
        self.inner.risk_for(args)
    }

    async fn execute(
        &self,
        args: Value,
        env: &ExecEnv,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        // 装饰器只改名字，执行期环境与信封原样透传（不拆不装）。
        self.inner.execute(args, env, ctx).await
    }
}

#[async_trait]
impl CapabilityVisitor for SubAgentVisitor {
    async fn put(&self, row: CapabilityRow) {
        // 模型族**整体丢弃**（生效者与目录都归系统 Agent，见模块文档）：
        // 写入方（model 插件的 traverse）逐行 put，规则只需在这一个入口上挡一次。
        if row.kind() == CapabilityRowKind::Model {
            crate::plugin_debug!(
                "agent",
                "丢弃子 Agent `{}` 的模型服务注册（生效者与目录都归系统 Agent）",
                self.agent_id
            );
            return;
        }
        // 提示词段（4c）走**段名前缀**再入内表（`agent/<id>/<段名>`）：
        // 与机制化之前「拼好前缀再 register」逐字一致，只是入口从 register 换成 put。
        if let Some(text) = row.prompt_text() {
            let name = self.seg(row.name());
            self.inner
                .put(CapabilityRow::prompt(&name, text.to_string()))
                .await;
            return;
        }
        // 子作用域的工具加名前缀再入内表（`agent_<safe_id>_<name>`）：
        // 行的构造走词汇自己的 `tool()`，名前缀由改名装饰器承载——与机制化之前
        // 「包一层 PrefixedCapability 再 register」同一条路，只是入口从 register
        // 换成 put（AR-025 4a）。
        let Some(cap) = row.cap().cloned() else {
            return;
        };
        let name = self.tool(&cap.name());
        self.inner
            .put(CapabilityRow::tool(
                Arc::new(PrefixedCapability { inner: cap, name }) as Arc<dyn Capability>,
            ))
            .await
    }

    async fn list(&self, kind: CapabilityRowKind) -> Vec<Arc<CapabilityRow>> {
        self.inner.list(kind).await
    }

    async fn get(&self, name: &str) -> Option<Arc<CapabilityRow>> {
        self.inner.get(name).await
    }

    // 模型族读写不再是 trait 上的方法（4b）：写侧在 `put` 上按族丢弃，读侧经下面的
    // `get` / `list` 原样透传内层——子 Agent 作用域内的插件照常按 id 取自己的模型
    // （目录是只读的，写侧已被挡住），而会话的生效者归系统 Agent。

    async fn register_vdfs_provider(&self, name: &str, provider: Arc<dyn VdfsProvider>) {
        self.inner
            .register_vdfs_provider(&self.seg(name), provider)
            .await
    }

    async fn list_vdfs_providers(&self) -> Vec<(String, Arc<dyn VdfsProvider>)> {
        self.inner.list_vdfs_providers().await
    }

    async fn get_vdfs_provider(&self, name: &str) -> Option<Arc<dyn VdfsProvider>> {
        self.inner.get_vdfs_provider(name).await
    }

    /// 子 Agent 的容器会把自己登记成 VDFS 根——**丢弃**，否则劫持系统侧全部挂载点
    async fn register_vdfs_root(&self, _provider: Arc<dyn VdfsProvider>) {
        crate::plugin_debug!(
            "agent",
            "丢弃子 Agent `{}` 的 VDFS 根注册（单槽，归系统 Agent）",
            self.agent_id
        );
    }

    async fn get_vdfs_root(&self) -> Option<Arc<dyn VdfsProvider>> {
        self.inner.get_vdfs_root().await
    }
}

#[cfg(test)]
#[path = "scope.test.rs"]
mod tests;
