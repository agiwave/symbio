//! `crates/symbio-plugin-agent/src/host/scope.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use async_trait::async_trait;
use serde_json::Value;
use symbio_plugin_api::{
    capability_prompts, capability_vdfs_root, family, port_result, CapabilityMeta, CapabilityRow,
    ExecEnv, PluginError, PluginInvokeRequest, PortErrorClass, PortExecContext, PortExecutor,
    PortInput, PortOutcome, VdfsContent, VdfsContext, VdfsError, VdfsProvider, VdfsRequest,
    VdfsResponse, VdfsResult, SCOPE_TWIN,
};

#[test]
fn tool_name_uses_protocol_safe_chars() {
    // agent id 允许 `.`（工具名不许，进 function-calling 协议）→ 压成 `_`；
    // `-` 在协议允许集内（`[A-Za-z0-9_-]`），保留以维持可读。
    let v = SubAgentVisitor::new(
        Arc::new(symbio_providers::DefaultToolVisitor::new()),
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
        Arc::new(symbio_providers::DefaultToolVisitor::new()),
        "reviewer",
    );
    assert_eq!(v.seg("work"), "agent/reviewer/work");
    assert_eq!(v.tool("read_skill"), "agent_reviewer_read_skill");
}

// ==================== 代理的分界：单槽丢弃 vs 多槽转发 ====================
//
// 本组钉的是 [`SubAgentVisitor`] 的**分界**（[DE-011](../../../../docs/003-design/011-Agent目录规范.md "DE-011-Agent目录规范")）：
//
// | 注册项 | 形态 | 代理动作 |
// |---|---|---|
// | VDFS 根 / 模型服务（生效者 + 目录） | **单槽** | **丢弃**——子 Agent 不得劫持父会话 |
// | 工具 / 系统提示词 / VDFS 挂载点 | **按名多槽** | **带前缀并集**——两份都生效 |
//
// ## 为什么这两条负向用例是硬要求
//
// 「丢弃」此前**只有模块文档在说**，全仓没有一条测试钉住它。把根行
// （`CapabilityRow::mount_root`）的丢弃改成转发（一次看起来很自然的"简化"：既然其它注册都转发了，这两条为什么不转？）
// 会让子 Agent 劫持整个会话的根与模型服务，而**没有任何用例会变红**——正是
// 「破的时候没有信号」。下面两条就是那个信号。
//
// 正向那条同样必要：它挡住另一种假绿——「代理把**所有**注册都丢弃」也会让上面两条通过。

/// 可辨认的 VDFS provider 桩：`read` 回自己的标签。
///
/// 让「`capability_vdfs_root()` 拿到的到底是哪一份」成为**可判**的事实——两个都是
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

/// 最小工具桩：只需一个名字，执行从不发生。
struct EchoTool(&'static str);

impl EchoTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: self.0.to_string(),
            ..Default::default()
        }
    }
}

#[async_trait]
impl PortExecutor for EchoTool {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let Some(ctx) = exec.ctx else {
            return PortOutcome::failed(
                PortErrorClass::NonRetryable,
                "工具端口没有调用方上下文，无法拆信封".to_string(),
            );
        };
        let env = ExecEnv::from_request(&*ctx);
        port_result(self.run(input.payload, &env, ctx).await)
    }
}

impl EchoTool {
    async fn run(
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
    let inner: Arc<dyn Registry> = Arc::new(symbio_providers::DefaultToolVisitor::new());
    inner
        .put(CapabilityRow::mount_root(Arc::new(LabeledRoot("系统根"))))
        .await;

    let scoped = SubAgentVisitor::new(Arc::clone(&inner), "reviewer");
    scoped
        .put(CapabilityRow::mount_root(Arc::new(LabeledRoot(
            "子 Agent 根",
        ))))
        .await;

    let root = capability_vdfs_root(&*inner)
        .await
        .expect("系统根应仍在槽位里");
    assert_eq!(
        label_of(&root).await,
        "系统根",
        "子 Agent 的根注册必须被丢弃——否则它劫持整个会话的 <根>"
    );
}

/// **模型族整体丢弃**：子 Agent 的 model 插件实例逐行 `put`，内层的生效者不得被改掉，
/// 也不得往内层目录里塞自己的 provider。
///
/// 两条一起断言，因为**目录也是可劫持面**：写侧被挡住但目录漏了的话，
/// 插件按 id 就能取到子 Agent 塞进来的实例（`scope.rs` 的文档明写这一条）。
#[tokio::test]
async fn sub_agent_model_registration_is_discarded() {
    let inner: Arc<dyn Registry> = Arc::new(symbio_providers::DefaultToolVisitor::new());
    inner.put(CapabilityRow::model("sys", true)).await;

    let scoped = SubAgentVisitor::new(Arc::clone(&inner), "reviewer");
    scoped.put(CapabilityRow::model("child", true)).await;

    let actives: Vec<String> = inner
        .list(family::MODEL)
        .await
        .into_iter()
        .filter(|row| row.active())
        .map(|row| row.name().to_string())
        .collect();
    assert_eq!(actives, ["sys"], "子 Agent 不得改掉会话的生效模型");
    assert!(
        inner
            .list(family::MODEL)
            .await
            .iter()
            .all(|row| row.name() != "child"),
        "子 Agent 的 provider 不得进目录——否则插件按 id 就能取到它"
    );
}

/// **多槽转发**：工具 / 系统提示词**确实**带前缀并进内层。
///
/// 与上面两条是一对：缺了它，「代理什么都不转发」也能让那两条通过。
#[tokio::test]
async fn sub_agent_multi_slot_registrations_join_parent_with_prefix() {
    let inner: Arc<dyn Registry> = Arc::new(symbio_providers::DefaultToolVisitor::new());
    let scoped = SubAgentVisitor::new(Arc::clone(&inner), "reviewer");

    // 注册走生产形状（`register_tool` 先端口行后工具行）：前缀化 meta 的行键 =
    // 前缀化名，执行体随前缀化的端口行一起进内表（派发侧按前缀名查得到它）。
    symbio_providers::register_tool(
        &scoped,
        EchoTool("read_skill").meta(),
        None,
        Arc::new(EchoTool("read_skill")),
    )
    .await;
    scoped
        .put(CapabilityRow::prompt(
            "persona",
            "你是评审子智能体".to_string(),
        ))
        .await;

    // 工具：前缀进 function-calling 协议允许的字符集
    assert!(
        inner
            .get("agent_reviewer_read_skill")
            .await
            .is_some_and(|row| row.name() == "agent_reviewer_read_skill"),
        "子 Agent 的工具应带前缀进内层（并集，两份都生效）"
    );
    // 前缀化的端口行随同批写入：派发侧按前缀名查得到执行体（注册收口点过闸后的
    // 那一份——改名的只是 meta，执行面随行）。
    assert!(
        symbio_plugin_api::port_executor_of(inner.as_ref(), "agent_reviewer_read_skill")
            .await
            .is_some(),
        "前缀名必须查得到执行体，否则子 Agent 的工具不可派发"
    );
    // 本名声明（D-428）：每个前缀工具都交出「前缀名 → 本名」一对，装配面据此折叠，
    // 不需要认识 `agent_` 这套前缀形状。声明行的键不与工具行同名，读侧只取载荷。
    let declared: Vec<(String, String)> = inner
        .list(SCOPE_TWIN)
        .await
        .into_iter()
        .filter_map(|row| row.payload::<(String, String)>().cloned())
        .collect();
    assert_eq!(
        declared,
        [(
            "agent_reviewer_read_skill".to_string(),
            "read_skill".to_string()
        )],
        "子 Agent 的每个前缀工具都要声明本名：{declared:?}"
    );
    // 系统提示词：前缀在 `agent/<id>/` 段下
    let prompts = capability_prompts(&*inner).await;
    assert!(
        prompts
            .iter()
            .any(|(k, v)| k == "agent/reviewer/persona" && v == "你是评审子智能体"),
        "子 Agent 的提示词段应带作用域前缀进内层：{prompts:?}"
    );
}

/// **读侧转发**：作用域代理不改变「谁在看」——所有取值口原样透传内层。
///
/// 若读侧也被作用域化（例如 `capability_vdfs_root()` 返回子树的根），
/// 上一条的写侧丢弃就白做了：**劫持改从读侧发生**。
#[tokio::test]
async fn read_side_forwards_to_inner_unchanged() {
    let inner: Arc<dyn Registry> = Arc::new(symbio_providers::DefaultToolVisitor::new());
    inner.put(CapabilityRow::model("sys", true)).await;
    inner
        .put(CapabilityRow::mount_root(Arc::new(LabeledRoot("系统根"))))
        .await;

    let scoped = SubAgentVisitor::new(Arc::clone(&inner), "reviewer");

    let row = scoped.get("sys").await.expect("目录按 id 透传（只读）");
    assert_eq!(row.family(), family::MODEL);
    assert!(row.active(), "生效位随行透传");
    let root = capability_vdfs_root(&scoped).await.expect("根应透传");
    assert_eq!(label_of(&root).await, "系统根");
}
