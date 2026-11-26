//! `dedup_declared_twins` 的判据：声明的副本去掉、未声明/本名缺席的保留、判定与顺序无关。
//!
//! 反向用例的关键在「不许猜」：一个长得像前缀副本的名字，只要没有声明，就必须留下。
//! 那条用例钉的是  的形状——前缀规则归子智能体插件，控制面只读声明。

use super::{candidate_tools, dedup_declared_twins};
use crate::plugin::SessionConfig;
use crate::types::Session;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use symbio_platform::kernel::capability::CapabilityMeta;
use symbio_platform::kernel::chat::chat_node::{ChatNode, MessageContent};
use symbio_platform::kernel::plugin::{
    Plugin, PluginInvokeRequest, PluginInvokeResponse, PluginMeta, PluginPayload,
    PluginSimpleRequest,
};
use symbio_platform::kernel::store::Store as KernelStore;
use symbio_platform::sdk::chat::session_chat;
use symbio_platform::sdk::plugin::fact_source::FactSource as _;
use symbio_plugin_api::port::{
    scoped_port_name, PortBindings, PortExecContext, PortExecutor, PortInput, PortOutcome,
    PORT_BINDINGS,
};
use symbio_plugin_api::{
    sandbox_publish, AgentScope, PluginDir, PluginInvokeRequestExt, SandboxFace, SandboxFact,
    AGENT_ID, AGENT_PERMS, CAPABILITY_VISITOR, SANDBOX,
};

fn meta(name: &str) -> CapabilityMeta {
    CapabilityMeta {
        name: name.to_string(),
        ..Default::default()
    }
}

fn pair(scoped: &str, base: &str) -> (String, String) {
    (scoped.to_string(), base.to_string())
}

fn kept_names(metas: Vec<CapabilityMeta>, twins: &[(String, String)]) -> Vec<String> {
    dedup_declared_twins(metas, twins)
        .into_iter()
        .map(|m| m.name)
        .collect()
}

/// 声明了的两份副本（含双重前缀）去掉；根工具保留；子树独有工具保留前缀。
#[test]
fn declared_twins_are_dropped_and_subtree_only_tools_are_kept() {
    let metas = vec![
        meta("vdfs_read"),
        meta("agent_run"),
        meta("agent_t59-agent_vdfs_read"),
        meta("agent_t59-agent_agent_run"),
        meta("agent_t59-agent_only_in_subtree"),
    ];
    let twins = vec![
        pair("agent_t59-agent_vdfs_read", "vdfs_read"),
        pair("agent_t59-agent_agent_run", "agent_run"),
        // 子树独有工具也声明本名，但本名不在表里 ⇒ 必须保留。
        pair("agent_t59-agent_only_in_subtree", "only_in_subtree"),
    ];
    assert_eq!(
        kept_names(metas, &twins),
        ["vdfs_read", "agent_run", "agent_t59-agent_only_in_subtree"]
    );
}

/// 反向用例：**没有声明就不许删**。名字长得再像前缀副本也不算副本——
/// 控制面不解析 `agent_` 或任何切点（ 之前正是这里按名字猜）。
#[test]
fn undeclared_names_are_never_inferred_as_twins() {
    let metas = vec![meta("vdfs_read"), meta("agent_x_vdfs_read")];
    assert_eq!(kept_names(metas, &[]), ["vdfs_read", "agent_x_vdfs_read"]);
}

/// 反向用例：声明存在但**本名缺席**（能力被卸载）⇒ 前缀那份仍要保留，
/// 否则卸载一个插件会静默丢掉一份能力（T3 的静默边）。
#[test]
fn declared_twin_with_absent_base_is_kept() {
    let metas = vec![meta("agent_x_vdfs_read")];
    assert_eq!(
        kept_names(metas, &[pair("agent_x_vdfs_read", "vdfs_read")]),
        ["agent_x_vdfs_read"]
    );
}

/// 声明指向一个不在本轮工具表里的前缀名 ⇒ 不影响任何一行。
#[test]
fn declaration_for_an_absent_scoped_name_is_a_noop() {
    let metas = vec![meta("vdfs_read"), meta("agent_x_vdfs_read")];
    assert_eq!(
        kept_names(metas, &[pair("agent_other_vdfs_read", "vdfs_read")]),
        ["vdfs_read", "agent_x_vdfs_read"]
    );
}

/// 判定与顺序无关：同一集合换序，留下的名字集合相同。
#[test]
fn dedup_is_order_independent() {
    let twins = [pair("agent_x_vdfs_read", "vdfs_read")];
    let a = kept_names(vec![meta("vdfs_read"), meta("agent_x_vdfs_read")], &twins);
    let b = kept_names(vec![meta("agent_x_vdfs_read"), meta("vdfs_read")], &twins);
    assert_eq!(a, ["vdfs_read"]);
    assert_eq!(b, ["vdfs_read"]);
}

// ── 候选暴露集合 ────────────────────────────────────────

use std::collections::HashSet;

fn bound(names: &[&str]) -> HashSet<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn candidate_names(metas: Vec<CapabilityMeta>, bound: &HashSet<String>) -> Vec<String> {
    candidate_tools(metas, bound)
        .into_iter()
        .map(|m| m.name)
        .collect()
}

/// 孤儿描述行（有说明、无有效绑定）不得进候选：候选 = 显式暴露 ∩ 有效绑定。
///
/// 协议命名与碰撞裁决在模型插件的投影里（`tool_projection.test.rs`）；
/// 本用例只钉会话侧那一半：它不猜工具，也不替投影造别名。
#[test]
fn orphan_description_row_is_not_exposed() {
    let metas = vec![meta("vdfs_read"), meta("ghost_tool")];
    assert_eq!(
        candidate_names(metas, &bound(&["vdfs_read"])),
        ["vdfs_read"]
    );
    // 空绑定集 = 空候选：仍要能正常装配，不报错、不猜工具。
    assert!(candidate_names(vec![meta("vdfs_read")], &bound(&[])).is_empty());
}

// ── 轮级执行上下文与失败回滚 ───────────────────────────

struct SelectiveFailureSource {
    inner: symbio_platform::kernel::store::MemoryStore<symbio_platform::kernel::event::Event>,
    fail_key: String,
    concurrent: symbio_platform::kernel::store::MemoryStore<symbio_platform::kernel::event::Event>,
}

impl SelectiveFailureSource {
    fn new(fail_key: &str) -> Self {
        Self {
            inner: Default::default(),
            fail_key: fail_key.into(),
            concurrent: Default::default(),
        }
    }
}

#[async_trait::async_trait]
impl symbio_platform::sdk::plugin::fact_source::FactSource for SelectiveFailureSource {
    fn append(&self, actor: String, draft: symbio_plugin_api::FactDraft) -> Result<u64, String> {
        if draft.key == self.fail_key {
            KernelStore::append(
                &self.concurrent,
                symbio_platform::sdk::plugin::fact_source::draft_into_event(
                    actor,
                    symbio_plugin_api::FactDraft {
                        key: "concurrent-writer".into(),
                        kind: "test.concurrent".into(),
                        entity: symbio_platform::kernel::event::Entity::Scope,
                        verb: symbio_platform::kernel::event::Verb::Asserted,
                        payload: serde_json::json!({}),
                        produced_by: None,
                    },
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
            return Err("selected append failed".into());
        }
        KernelStore::append(
            &self.inner,
            symbio_platform::sdk::plugin::fact_source::draft_into_event(actor, draft),
        )
        .map(|seq| seq.value())
        .map_err(|error| format!("{error:?}"))
    }

    fn facts(&self) -> Vec<symbio_platform::kernel::event::Event> {
        KernelStore::range(&self.inner, symbio_platform::kernel::event::Seq::new(0))
    }

    fn range(&self, from_seq: u64) -> Vec<symbio_platform::kernel::event::Event> {
        KernelStore::range(
            &self.inner,
            symbio_platform::kernel::event::Seq::new(from_seq),
        )
    }

    fn head(&self) -> u64 {
        KernelStore::head(&self.inner).value()
    }
}

struct DummyExecutor;

#[async_trait::async_trait]
impl PortExecutor for DummyExecutor {
    async fn execute(&self, _ctx: PortExecContext, _input: PortInput) -> PortOutcome {
        PortOutcome::ok()
    }
}

struct TestSandbox {
    permitted: bool,
}

impl SandboxFace for TestSandbox {
    fn permits(&self, _facts: &[SandboxFact]) -> bool {
        self.permitted
    }

    fn intercept(&self, _tool_name: &str, _args: &serde_json::Value) -> Option<serde_json::Value> {
        None
    }
}

struct RoundCollectingParent {
    root: std::path::PathBuf,
    pass: AtomicUsize,
}

#[async_trait::async_trait]
impl Plugin for RoundCollectingParent {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("round-context-test", "round-context-test")
    }

    async fn route(
        self: Arc<Self>,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Err(symbio_platform::kernel::plugin::PluginError::NotFound(
            "round-context-test".into(),
        ))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        if ctx.get(CAPABILITY_VISITOR).is_some() {
            assert!(ctx.get(AGENT_ID).is_some(), "收集上下文必须带会话绑定");
            let pass = self.pass.fetch_add(1, Ordering::SeqCst);
            if let Some(bindings) = ctx.get(PORT_BINDINGS) {
                let port = scoped_port_name(&ctx, &format!("round-{pass}"));
                bindings.bind(&[port], "round-context-test", Some(Arc::new(DummyExecutor)));
            }
            if let Some(agent_id) = ctx.get(AGENT_ID) {
                AgentScope {
                    id: agent_id,
                    root: self.root.clone(),
                    permissions: Default::default(),
                }
                .publish(&ctx);
            }
            assert!(sandbox_publish(
                &ctx,
                Arc::new(TestSandbox { permitted: true })
            ));
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

fn context_plugin(
    dir: &std::path::Path,
    parent: Option<Arc<dyn Plugin>>,
) -> crate::plugin::SessionPlugin {
    crate::plugin::SessionPlugin::new(
        parent.as_ref().map(Arc::downgrade),
        SessionConfig::default(),
        PluginDir::at(dir, "session"),
    )
}

#[tokio::test]
async fn each_round_gets_fresh_collection_slots_and_host_bindings() {
    let dir = tempfile::tempdir().unwrap();
    let parent: Arc<dyn Plugin> = Arc::new(RoundCollectingParent {
        root: dir.path().join("subagent").join("reviewer"),
        pass: AtomicUsize::new(0),
    });
    let p = context_plugin(dir.path(), Some(Arc::clone(&parent)));
    let mut session = Session::new("round-context");
    session.metadata["agent_id"] = serde_json::json!("reviewer");
    p.save_session(&session).await.unwrap();

    let base: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let host_bindings = PortBindings::default();
    host_bindings.bind(
        &["session.subagent".into()],
        "host-test",
        Some(Arc::new(DummyExecutor)),
    );
    base.set(PORT_BINDINGS, Arc::new(host_bindings));

    let stale_scope = AgentScope {
        id: "stale-agent".into(),
        root: dir.path().join("stale-agent"),
        permissions: Default::default(),
    };
    let stale_perms = Arc::new(OnceLock::new());
    stale_perms.set(stale_scope.clone()).unwrap();
    base.set(AGENT_PERMS, stale_perms.clone());
    let stale_sandbox = Arc::new(OnceLock::new());
    assert!(stale_sandbox
        .set(Arc::new(TestSandbox { permitted: false }) as Arc<dyn SandboxFace>)
        .is_ok());
    base.set(SANDBOX, stale_sandbox.clone());

    let first = p
        .prepare_execution_context(&base, &session.id)
        .await
        .unwrap();
    let second = p
        .prepare_execution_context(&base, &session.id)
        .await
        .unwrap();
    let store = p.get_store().await.unwrap();
    first.publish(&store, &session.id, Some("remote:first".into()));
    second.publish(&store, &session.id, Some("remote:second".into()));

    let first_names = first.ctx.get(PORT_BINDINGS).unwrap().bound();
    let second_names = second.ctx.get(PORT_BINDINGS).unwrap().bound();
    assert!(first_names.contains(&"session.subagent".into()));
    assert!(first_names.iter().any(|name| name.ends_with("round-0")));
    assert!(second_names.contains(&"session.subagent".into()));
    assert!(second_names.iter().any(|name| name.ends_with("round-1")));
    assert!(
        !second_names.iter().any(|name| name.ends_with("round-0")),
        "第二轮不得继承第一轮收集出的端口：{second_names:?}"
    );

    for round in [&first.ctx, &second.ctx] {
        assert_eq!(
            symbio_plugin_api::agent_perms_of(round.as_ref())
                .as_ref()
                .map(|scope| scope.id.as_str()),
            Some("reviewer")
        );
        assert!(
            symbio_plugin_api::sandbox_of(round.as_ref()).is_some_and(|face| face.permits(&[])),
            "每轮必须发布自己的沙箱句柄"
        );
    }
    assert_eq!(
        base.get(PORT_BINDINGS).unwrap().bound(),
        ["session.subagent"]
    );
    assert_eq!(
        base.get(AGENT_PERMS)
            .unwrap()
            .get()
            .map(|scope| scope.id.as_str()),
        Some("stale-agent"),
        "能力收集不得改写可信基底"
    );
    assert!(
        !base.get(SANDBOX).unwrap().get().unwrap().permits(&[]),
        "可信基底上的旧槽不得被本轮替换"
    );

    assert_eq!(
        *store.source_principal_cell(&session.id).read().unwrap(),
        Some("remote:second".into())
    );
    assert_eq!(
        store
            .agent_scope_cell(&session.id)
            .read()
            .unwrap()
            .as_ref()
            .map(|scope| scope.id.as_str()),
        Some("reviewer")
    );
}

#[tokio::test]
async fn first_append_failure_restores_fifo_even_if_concurrent_writer_advances_head() {
    let dir = tempfile::tempdir().unwrap();
    let p = context_plugin(dir.path(), None);
    let session = Session::new("round-first-append-failure");
    p.save_session(&session).await.unwrap();
    for (id, text) in [("i1", "第一条"), ("i2", "第二条")] {
        p.enqueue_inbox(
            &session.id,
            Some(id.into()),
            ChatNode {
                content: Some(MessageContent::Text(text.into())),
                ..Default::default()
            },
            session_chat::Request {
                source_principal: Some("remote:new".into()),
                ..Default::default()
            },
            None,
        )
        .await;
    }
    let store = p.get_store().await.unwrap();
    store.set_source_principal(&session.id, Some("remote:previous".into()));
    let source = Arc::new(SelectiveFailureSource::new("i1"));
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));

    let error = p
        .consume_inbox_turn(&ctx, &session.id, source.clone())
        .await
        .expect_err("首条 append 必须失败");
    assert!(error.to_string().contains("user.message 落格失败"));
    assert_eq!(source.concurrent_head(), 1, "另一写方必须推进 WAL head");
    assert!(source.facts().is_empty(), "失败条目不得留下 user.message");
    assert_eq!(
        p.inbox_items(&session.id)
            .await
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["i1", "i2"]
    );
    assert_eq!(
        *store.source_principal_cell(&session.id).read().unwrap(),
        Some("remote:previous".into())
    );
    let state = p.active_mgr.get_or_create(&session.id).await;
    assert!(!state.inner.read().await.is_working());
    assert!(state.abort_signal.read().unwrap().is_none());
}

impl SelectiveFailureSource {
    fn concurrent_head(&self) -> u64 {
        KernelStore::head(&self.concurrent).value()
    }
}

#[tokio::test]
async fn post_user_append_failure_keeps_item_and_settles_same_turn_failed() {
    let dir = tempfile::tempdir().unwrap();
    let p = context_plugin(dir.path(), None);
    let session = Session::new("round-post-user-failure");
    p.save_session(&session).await.unwrap();
    p.enqueue_inbox(
        &session.id,
        Some("landed".into()),
        ChatNode {
            content: Some(MessageContent::Text("已经落格".into())),
            ..Default::default()
        },
        session_chat::Request {
            source_principal: Some("remote:new".into()),
            ..Default::default()
        },
        None,
    )
    .await;
    let store = p.get_store().await.unwrap();
    store.set_source_principal(&session.id, Some("remote:previous".into()));
    let source = Arc::new(SelectiveFailureSource::new(
        &symbio_plugin_api::debt::session_config_id("landed"),
    ));
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));

    let error = p
        .consume_inbox_turn(&ctx, &session.id, source.clone())
        .await
        .expect_err("session.config append 必须失败");
    assert!(error.to_string().contains("session.config 落格失败"));
    assert!(
        p.inbox_items(&session.id).await.is_empty(),
        "已落事实不得重放"
    );
    assert_eq!(
        *store.source_principal_cell(&session.id).read().unwrap(),
        Some("remote:new".into())
    );
    let state = p.active_mgr.get_or_create(&session.id).await;
    for _ in 0..200 {
        if !state.inner.read().await.is_working() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    assert!(!state.inner.read().await.is_working());
    assert!(state.abort_signal.read().unwrap().is_none());
    assert_eq!(
        state.inner.read().await.last_outcome.as_deref(),
        Some("failed")
    );
    assert_eq!(
        source
            .facts()
            .iter()
            .filter(|event| event.kind == symbio_plugin_api::events::EVENT_USER_MESSAGE)
            .count(),
        1
    );
    assert!(source.facts().iter().any(|event| {
        event.kind == symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FALLBACK
            && event.payload["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("session.config"))
    }));
}

#[tokio::test]
async fn preparation_failure_restores_fifo_and_shared_slots_without_facts() {
    let dir = tempfile::tempdir().unwrap();
    let p = context_plugin(dir.path(), None);
    let mut session = Session::new("round-preparation-failure");
    session.metadata["agent_id"] = serde_json::json!("unpublished-reviewer");
    p.save_session(&session).await.unwrap();

    for (id, text) in [("i1", "第一条"), ("i2", "第二条")] {
        p.enqueue_inbox(
            &session.id,
            Some(id.into()),
            ChatNode {
                content: Some(MessageContent::Text(text.into())),
                ..Default::default()
            },
            session_chat::Request {
                source_principal: Some("remote:failed-request".into()),
                ..Default::default()
            },
            None,
        )
        .await;
    }

    let store = p.get_store().await.unwrap();
    store.set_source_principal(&session.id, Some("remote:previous".into()));
    store.set_agent_scope(
        &session.id,
        Some(AgentScope {
            id: "previous-reviewer".into(),
            root: dir.path().join("previous-reviewer"),
            permissions: Default::default(),
        }),
    );
    let source = p.fact_source(&session.id).await.unwrap();
    let head_before = source.head();
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));

    let error = p
        .consume_inbox_turn(&ctx, &session.id, Arc::clone(&source))
        .await
        .expect_err("绑定 Agent 却没有发布作用域时必须 fail closed");
    assert!(error.to_string().contains("unpublished-reviewer"));
    assert_eq!(source.head(), head_before, "准备失败不得落下任何事实");
    assert!(
        source
            .facts()
            .iter()
            .all(|event| event.kind != symbio_plugin_api::events::EVENT_USER_MESSAGE),
        "失败请求不得把消息事实留在 WAL"
    );
    assert_eq!(
        p.inbox_items(&session.id)
            .await
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["i1", "i2"],
        "准备失败必须按原 FIFO 顺序回队"
    );
    let state = p.active_mgr.get_or_create(&session.id).await;
    assert!(
        !state.inner.read().await.is_working(),
        "失败后会话必须回到空闲"
    );
    assert!(
        state.abort_signal.read().unwrap().is_none(),
        "失败轮不得留下中止信号"
    );
    assert_eq!(
        *store.source_principal_cell(&session.id).read().unwrap(),
        Some("remote:previous".into())
    );
    assert_eq!(
        store
            .agent_scope_cell(&session.id)
            .read()
            .unwrap()
            .as_ref()
            .map(|scope| scope.id.as_str()),
        Some("previous-reviewer")
    );
}
