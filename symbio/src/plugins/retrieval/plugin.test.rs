//! retrieval 插件的单元测试：检索行登记 / 召回链路 / 平凡值退化 / 停用。
//!
//! ## 为什么每个用例都拿一把锁
//!
//! Actor 表是**进程级**的（`inventory` + `OnceLock`），cargo 并行跑测试。
//! 与 `actor` 插件测试同款：统一经 `actor_test_guard()` 把整段变成临界区。
//!
//! ## 为什么不用 `#[tokio::test]`
//!
//! 那把锁是 `std::sync::Mutex`，guard 会跨 `.await` 持有 ⇒ clippy 报
//! `await_holding_lock`。解法同 `actor` 插件：把 await 搬进私有 runtime 的 `block_on`。

use super::*;
use crate::symbio_core::{
    actor_get, ActorScope, ActorSpec, PluginDir, PluginInvokeRequestExt, PluginPayload,
    PluginSimpleRequest,
};
use std::fs;

/// 造一个临时系统根，并按需写入一个会话目录（`messages.json` + `MEMORY.md`）。
fn temp_root() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("symbio-test-retrieval-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_session(root: &Path, id: &str, messages: &str, memory: Option<&str>) {
    let dir = root.join(PLUGIN_ID_SESSION_DIR).join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("messages.json"), messages).unwrap();
    if let Some(m) = memory {
        fs::write(dir.join("MEMORY.md"), m).unwrap();
    }
}

fn test_dir(root: &Path) -> PluginDir {
    // `PluginDir::at` 存的**就是**这个目录（不追加 name）——生产里
    // `plugin_dir_from_ctx` 给的是 `<root>/retrieval`，故这里同样指向它，
    // 于是 `new()` 的 `parent()` 恰好回到 `root`（与生产一致）。
    PluginDir::at(root.join(PLUGIN_ID_RETRIEVAL), PLUGIN_ID_RETRIEVAL)
}

/// 干净状态 + 全局串行闸。
fn setup() -> std::sync::MutexGuard<'static, ()> {
    crate::symbio_core::actor_test_guard()
}

fn plugin(root: &Path, enabled: bool) -> Arc<RetrievalPlugin> {
    Arc::new(RetrievalPlugin::new(
        test_dir(root),
        RetrievalConfig { enabled },
    ))
}

fn ctx(path: &str) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(PATH, path.to_string());
    Arc::new(req)
}

fn data_of(p: PluginPayload) -> serde_json::Value {
    match p {
        PluginPayload::Data(d) => d.serialize().unwrap(),
        other => panic!("expected data, got {other:?}"),
    }
}

fn run<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("测试 runtime")
        .block_on(f)
}

/// 一段最小的落盘消息（存储形状：`{"messages":[…]}`，`seq` 由存储分配）。
///
/// `role` / `type` 用 **snake_case**（`ChatMessage` 的 serde 形态），
/// 不是 `Debug` 的大写——存储里就是这个形状。
fn messages_json() -> String {
    json!({
        "messages": [
            { "id": "u1", "role": "user", "type": "text", "seq": 1, "timestamp": 100 },
            { "id": "a1", "role": "assistant", "type": "text", "seq": 2, "timestamp": 200 }
        ]
    })
    .to_string()
}

#[test]
fn enabled_registers_retrieval_row() {
    let _g = setup();
    let root = temp_root();
    let _p = plugin(&root, true);
    let row = actor_get(ACTOR_NAME_SESSION_RETRIEVAL).expect("启用时必须登记检索者行");
    assert_eq!(row.pattern, ActorPattern::Translator);
    assert_eq!(row.budget_ms, RETRIEVAL_BUDGET_MS);
    assert_eq!(row.budget_ms, 500, "S06 §3 的检索者预算");
    assert_eq!(row.scope, ActorScope::Root);
    assert_eq!(row.principal, "", "登记期用占位符");
}

#[test]
fn disabled_registers_nothing() {
    let _g = setup();
    let root = temp_root();
    let _p = plugin(&root, false);
    assert!(
        actor_get(ACTOR_NAME_SESSION_RETRIEVAL).is_none(),
        "停用时不登记——J2 的退化路径"
    );
}

/// 召回链路：磁盘 → 派生事实（含 `memory.*`）→ 跑投影 → 回视图。
#[test]
fn list_route_runs_recall_projection() {
    let _g = setup();
    let root = temp_root();
    write_session(&root, "s1", &messages_json(), Some("结论 A\n约束 B\n"));
    let p = plugin(&root, true);

    let v = run(async move { data_of(p.clone().route(ctx("list")).await.unwrap()) });

    // B1：事实被派生出来（对话 2 条 + encoded 1 条 + recalled 2 条）
    let facts = v["facts"].as_array().expect("facts 必须是数组");
    assert_eq!(facts.len(), 5, "2 对话 + 1 编码 + 2 召回");

    // B2：投影跑了，且不是平凡值（有 memory.* 事实）
    assert_eq!(v["degraded"], false, "投影已登记 ⇒ 不应降级");
    let recall = &v["recall"];
    assert!(recall.is_object(), "recall 应是 View 对象");
    assert_eq!(recall["trivial"], false, "有记忆事实 ⇒ 非平凡值");
    let verbs: Vec<&str> = recall["value"]["memory_verbs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x.as_str())
        .collect();
    assert!(verbs.contains(&"memory.encoded"));
    assert!(verbs.contains(&"memory.recalled"));

    // B3：检索行在表里
    let actor = &v["actor"];
    assert!(actor.is_object(), "检索行应在 Actor 表里");
    assert_eq!(actor["name"], ACTOR_NAME_SESSION_RETRIEVAL);
    assert_eq!(actor["pattern"], "translator");
    assert_eq!(actor["budget_ms"], 500);
}

/// 无 `MEMORY.md` ⇒ 投影走平凡值，但链路不报错（J2）。
#[test]
fn no_memory_degrades_to_trivial_without_error() {
    let _g = setup();
    let root = temp_root();
    write_session(&root, "s1", &messages_json(), None);
    let p = plugin(&root, true);

    let v = run(async move { data_of(p.clone().route(ctx("list")).await.unwrap()) });
    let facts = v["facts"].as_array().unwrap();
    assert_eq!(facts.len(), 2, "只有对话事实");
    assert_eq!(v["recall"]["trivial"], true, "无记忆 ⇒ 平凡值");
    assert_eq!(v["degraded"], false, "投影在，只是折出平凡值——不是降级");
}

/// 双跑一致（A4）：同一份磁盘两次调用，`recall` 逐字节相同。
#[test]
fn recall_is_deterministic_across_calls() {
    let _g = setup();
    let root = temp_root();
    write_session(&root, "s1", &messages_json(), Some("A\nB\n"));
    let p = plugin(&root, true);

    let a = run(async {
        let p = p.clone();
        data_of(p.route(ctx("list")).await.unwrap())
    });
    let b = run(async {
        let p = p.clone();
        data_of(p.route(ctx("list")).await.unwrap())
    });
    assert_eq!(
        serde_json::to_string(&a["recall"]).unwrap(),
        serde_json::to_string(&b["recall"]).unwrap(),
        "同一份磁盘两次召回必须逐字节相同"
    );
}

#[test]
fn disabled_makes_route_unreachable() {
    let _g = setup();
    let root = temp_root();
    let p = plugin(&root, false);
    let err = run(async move { p.clone().route(ctx("list")).await.unwrap_err() });
    assert!(matches!(err, PluginError::NotFound(_)));
}

#[test]
fn unknown_route_is_not_found() {
    let _g = setup();
    let root = temp_root();
    let p = plugin(&root, true);
    let err = run(async move { p.clone().route(ctx("nope")).await.unwrap_err() });
    assert!(matches!(err, PluginError::NotFound(_)));
}

#[test]
fn config_definition_has_one_form_section_with_one_toggle() {
    let d: DetailDefinition = config_definition();
    assert_eq!(d.sections.len(), 1);
    assert_eq!(d.sections[0].fields.len(), 1);
    assert_eq!(d.sections[0].fields[0].key, "enabled");
}

#[test]
fn metadata_id_matches_plugin_id() {
    assert_eq!(RetrievalPlugin::metadata().id, PLUGIN_ID_RETRIEVAL);
}

/// 表是共享的：检索行与别的插件登记的行能同时出现（不各持一份）。
#[test]
fn list_sees_other_plugins_rows() {
    let _g = setup();
    let root = temp_root();
    crate::symbio_core::actor_register(&ActorScope::Root, ActorSpec::builtin("session.reasoner"))
        .unwrap();
    let _p = plugin(&root, true);
    let names: Vec<String> = actor_list().into_iter().map(|s| s.name).collect();
    assert!(names.contains(&"session.reasoner".to_string()));
    assert!(names.contains(&ACTOR_NAME_SESSION_RETRIEVAL.to_string()));
}
