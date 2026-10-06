//! `symbio/src/plugins/agent/host/scope.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use crate::symbio_core::{
    TurnOutput, VdfsContent, VdfsContext, VdfsError, VdfsRequest, VdfsResponse, VdfsResult,
};
use async_trait::async_trait;

#[test]
fn tool_name_uses_protocol_safe_chars() {
    // agent id 允许 `.`（工具名不许，进 function-calling 协议）→ 压成 `_`；
    // `-` 在协议允许集内（`[A-Za-z0-9_-]`），保留以维持可读。
    let v = SubAgentVisitor::new(
        Arc::new(crate::providers::DefaultToolVisitor::new()),
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
        Arc::new(crate::providers::DefaultToolVisitor::new()),
        "reviewer",
    );
    assert_eq!(v.seg("work"), "agent/reviewer/work");
    assert_eq!(v.tool("read_skill"), "agent_reviewer_read_skill");
}

// ==================== 代理的分界：单槽丢弃 vs 多槽转发 ====================
//
// 本组钉的是 [`SubAgentVisitor`] 的**分界**（[plan/11 §2-F](../../../../../docs/plan/11-多执行器与多主体加固实施方案.md)）：
//
// | 注册项 | 形态 | 代理动作 |
// |---|---|---|
// | VDFS 根 / 模型服务（生效者 + 目录） | **单槽** | **丢弃**——子 Agent 不得劫持父会话 |
// | 工具 / 系统提示词 / VDFS 挂载点 | **按名多槽** | **带前缀并集**——两份都生效 |
//
// ## 为什么这两条负向用例是硬要求
//
// 「丢弃」此前**只有模块文档在说**，全仓没有一条测试钉住它。把 `register_vdfs_root`
// 的丢弃改成转发（一次看起来很自然的"简化"：既然其它注册都转发了，这两条为什么不转？）
// 会让子 Agent 劫持整个会话的根与模型服务，而**没有任何用例会变红**——正是
// [plan/11 §4](../../../../../docs/plan/11-多执行器与多主体加固实施方案.md) 说的
// 「破的时候没有信号」。下面两条就是那个信号。
//
// 正向那条同样必要：它挡住另一种假绿——「代理把**所有**注册都丢弃」也会让上面两条通过。

/// 可辨认的 VDFS provider 桩：`read` 回自己的标签。
///
/// 让「`get_vdfs_root()` 拿到的到底是哪一份」成为**可判**的事实——两个都是
/// `Arc<dyn VdfsProvider>`，不比内容就分不出谁被登记了。
struct LabeledRoot(&'static str);

#[async_trait]
impl VdfsProvider for LabeledRoot {
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        _path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::Read => Ok(VdfsResponse::Read(VdfsContent::text(self.0))),
            _ => Err(VdfsError::NotImplemented),
        }
    }
}

/// 最小模型服务桩（形状同 `providers/collectors/tool_visitor.test.rs`）
struct LabeledProvider(&'static str);

#[async_trait]
impl ModelProvider for LabeledProvider {
    fn provider_id(&self) -> &str {
        self.0
    }

    fn api_protocol(&self) -> &str {
        "openai_chat"
    }

    fn rate_limit_ms(&self) -> u64 {
        0
    }

    fn max_context_tokens(&self) -> u32 {
        262_144
    }

    async fn effective_context_tokens(&self) -> u32 {
        262_144
    }

    async fn execute_turn(
        &self,
        _system_prompt: &str,
        _messages: &[crate::symbio_core::chat_message::ChatMessage],
        _tools: &[CapabilityMeta],
        _root_id: &str,
        _env: &ExecEnv,
    ) -> Result<TurnOutput, PluginError> {
        Err(PluginError::InternalError("mock".to_string()))
    }
}

fn provider(id: &'static str) -> Arc<dyn ModelProvider> {
    Arc::new(LabeledProvider(id))
}

/// 最小工具桩：只需一个名字，执行从不发生。
struct EchoTool(&'static str);

#[async_trait]
impl Capability for EchoTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: self.0.to_string(),
            ..Default::default()
        }
    }

    async fn execute(
        &self,
        _args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        Err(PluginError::InternalError("stub".to_string()))
    }
}

/// 读一份 provider 的标签（`dispatch(Read)` 的回声就是它的身份）
async fn label_of(root: &Arc<dyn VdfsProvider>) -> String {
    let resp = root
        .dispatch(&VdfsContext::empty(), "", VdfsRequest::Read)
        .await
        .expect("桩的 Read 必须成功");
    resp.into_read()
        .and_then(|c| c.text)
        .expect("桩的 Read 必须带正文")
}

/// **单槽丢弃（根）**：子 Agent 登记 VDFS 根**不得**改掉内层那份。
///
/// 内层那份是系统 Agent 的根——它被换掉等于**整个会话的挂载点全消失**
/// （子树的根只认得子树里的插件）。
#[tokio::test]
async fn sub_agent_root_registration_is_discarded() {
    let inner: Arc<dyn CapabilityVisitor> = Arc::new(crate::providers::DefaultToolVisitor::new());
    inner
        .register_vdfs_root(Arc::new(LabeledRoot("系统根")))
        .await;

    let scoped = SubAgentVisitor::new(Arc::clone(&inner), "reviewer");
    scoped
        .register_vdfs_root(Arc::new(LabeledRoot("子 Agent 根")))
        .await;

    let root = inner.get_vdfs_root().await.expect("系统根应仍在槽位里");
    assert_eq!(
        label_of(&root).await,
        "系统根",
        "子 Agent 的根注册必须被丢弃——否则它劫持整个会话的 <根>"
    );
}

/// **单槽丢弃（模型）**：子 Agent 的 `register_model_providers` 不得改掉内层的
/// 生效者，也不得往内层目录里塞自己的 provider。
///
/// 两条一起断言，因为**目录也是可劫持面**：写侧被挡住但目录漏了的话，
/// 插件按 id 就能取到子 Agent 塞进来的实例（`scope.rs` 的文档明写这一条）。
#[tokio::test]
async fn sub_agent_model_registration_is_discarded() {
    let inner: Arc<dyn CapabilityVisitor> = Arc::new(crate::providers::DefaultToolVisitor::new());
    inner
        .register_model_providers(Some("sys"), vec![provider("sys")])
        .await;

    let scoped = SubAgentVisitor::new(Arc::clone(&inner), "reviewer");
    scoped
        .register_model_providers(Some("child"), vec![provider("child")])
        .await;

    assert_eq!(
        inner
            .get_model_provider()
            .await
            .expect("系统生效者应仍在槽位里")
            .provider_id(),
        "sys",
        "子 Agent 不得改掉会话的生效模型"
    );
    assert!(
        inner.get_model_provider_by_id("child").await.is_none(),
        "子 Agent 的 provider 不得进目录——否则插件按 id 就能取到它"
    );
}

/// **多槽转发**：工具 / 系统提示词 / VDFS 挂载点**确实**带前缀并进内层。
///
/// 与上面两条是一对：缺了它，「代理什么都不转发」也能让那两条通过。
#[tokio::test]
async fn sub_agent_multi_slot_registrations_join_parent_with_prefix() {
    let inner: Arc<dyn CapabilityVisitor> = Arc::new(crate::providers::DefaultToolVisitor::new());
    let scoped = SubAgentVisitor::new(Arc::clone(&inner), "reviewer");

    scoped.register(Arc::new(EchoTool("read_skill"))).await;
    scoped
        .register_system_prompt("persona", "你是评审子智能体".to_string())
        .await;
    scoped
        .register_vdfs_provider("work", Arc::new(LabeledRoot("子 Agent 工作区")))
        .await;

    // 工具：前缀进 function-calling 协议允许的字符集
    assert!(
        inner.has_capability("agent_reviewer_read_skill").await,
        "子 Agent 的工具应带前缀进内层（并集，两份都生效）"
    );
    // 系统提示词：前缀在 `agent/<id>/` 段下
    let prompts = inner.list_system_prompts().await;
    assert!(
        prompts
            .iter()
            .any(|(k, v)| k == "agent/reviewer/persona" && v == "你是评审子智能体"),
        "子 Agent 的提示词段应带作用域前缀进内层：{prompts:?}"
    );
    // VDFS 挂载点：同前缀，且**内容**是子 Agent 那一份（不是内层被覆盖）
    let mount = inner
        .get_vdfs_provider("agent/reviewer/work")
        .await
        .expect("子 Agent 的挂载点应带前缀进内层");
    assert_eq!(label_of(&mount).await, "子 Agent 工作区");
    assert!(
        inner.get_vdfs_provider("work").await.is_none(),
        "挂载点走前缀而非覆盖——系统侧的 work 不能被盖掉"
    );
}

/// **读侧转发**：作用域代理不改变「谁在看」——所有取值口原样透传内层。
///
/// 若读侧也被作用域化（例如 `get_vdfs_root()` 返回子树的根），
/// 上一条的写侧丢弃就白做了：**劫持改从读侧发生**。
#[tokio::test]
async fn read_side_forwards_to_inner_unchanged() {
    let inner: Arc<dyn CapabilityVisitor> = Arc::new(crate::providers::DefaultToolVisitor::new());
    inner
        .register_model_providers(Some("sys"), vec![provider("sys")])
        .await;
    inner
        .register_vdfs_root(Arc::new(LabeledRoot("系统根")))
        .await;

    let scoped = SubAgentVisitor::new(Arc::clone(&inner), "reviewer");

    assert_eq!(
        scoped
            .get_model_provider()
            .await
            .expect("生效者应透传")
            .provider_id(),
        "sys"
    );
    assert_eq!(
        scoped
            .get_model_provider_by_id("sys")
            .await
            .expect("目录按 id 透传（只读）")
            .provider_id(),
        "sys"
    );
    let root = scoped.get_vdfs_root().await.expect("根应透传");
    assert_eq!(label_of(&root).await, "系统根");
}
