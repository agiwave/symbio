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
//! ## 单槽位注册不转发
//!
//! `register_vdfs_root` 与 `register_model_provider` 是**单槽**（后者覆盖前者）。
//! 子 Agent 的插件树里也有容器（它会把自己登记成 VDFS 根），原样转发会让子 Agent
//! **劫持**整个 `<根>` 根与模型服务——系统侧的全部挂载点瞬间消失。因此这两项
//! **丢弃**（记 debug）。
//!
//! 同理，VDFS 挂载点走前缀而非覆盖：VDFS 是用户浏览 / 编辑面，子 Agent 的 `work`
//! 若盖掉系统的 `work`，用户就改不到工作区记忆了——那是回归。

use crate::symbio_core::vdfs::VdfsProvider;
use crate::symbio_core::{
    Capability, CapabilityMeta, CapabilityVisitor, InvokeRequest, InvokeResponse, ModelProvider,
    PluginPayload,
};
use async_trait::async_trait;
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

    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        self.inner.execute(ctx).await
    }
}

#[async_trait]
impl CapabilityVisitor for SubAgentVisitor {
    async fn register(&self, tool: Arc<dyn Capability>) {
        let name = self.tool(&tool.name());
        self.inner
            .register(Arc::new(PrefixedCapability { inner: tool, name }) as Arc<dyn Capability>)
            .await
    }

    async fn list_capability(&self) -> Vec<CapabilityMeta> {
        self.inner.list_capability().await
    }

    async fn invoke(
        &self,
        name: &str,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        self.inner.invoke(name, ctx).await
    }

    async fn has_capability(&self, name: &str) -> bool {
        self.inner.has_capability(name).await
    }

    /// 子 Agent **不得**决定用哪个模型服务（单槽，被劫持等于整个会话换模型）
    async fn register_model_provider(&self, _provider: Arc<dyn ModelProvider>) {
        crate::plugin_debug!(
            "agent",
            "丢弃子 Agent `{}` 的模型服务注册（单槽，归系统 Agent）",
            self.agent_id
        );
    }

    async fn get_model_provider(&self) -> Option<Arc<dyn ModelProvider>> {
        self.inner.get_model_provider().await
    }

    async fn register_system_prompt(&self, name: &str, prompt: String) {
        self.inner
            .register_system_prompt(&self.seg(name), prompt)
            .await
    }

    async fn list_system_prompts(&self) -> Vec<(String, String)> {
        self.inner.list_system_prompts().await
    }

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
mod tests {
    use super::*;

    #[test]
    fn tool_name_uses_protocol_safe_chars() {
        // agent id 允许 `.`（工具名不许，进 function-calling 协议）→ 压成 `_`；
        // `-` 在协议允许集内（`[A-Za-z0-9_-]`），保留以维持可读。
        let v = SubAgentVisitor::new(
            Arc::new(crate::symbio_core::DefaultToolVisitor::new()),
            "com.acme.code-reviewer",
        );
        assert_eq!(
            v.tool("read_skill"),
            "agent_com_acme_code-reviewer_read_skill"
        );
        assert!(
            v.tool("read_skill")
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "工具名必须落在 function-calling 的字符集内"
        );
        assert_eq!(v.seg("work"), "agent/com.acme.code-reviewer/work");
    }

    #[test]
    fn seg_keeps_id_readable() {
        let v = SubAgentVisitor::new(
            Arc::new(crate::symbio_core::DefaultToolVisitor::new()),
            "reviewer",
        );
        assert_eq!(v.seg("work"), "agent/reviewer/work");
        assert_eq!(v.tool("read_skill"), "agent_reviewer_read_skill");
    }
}
