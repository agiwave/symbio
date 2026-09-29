//! fact_log 插件自测 —— 磁盘派生（公开布局）+ 平凡值（未接入 = 无事实源）。

use super::*;
use crate::plugins::fact_log::config::FactLogConfig;
use crate::symbio_core::PluginSimpleRequest;

fn plugin_at(root: &std::path::Path) -> FactLogPlugin {
    let dir = crate::symbio_core::PluginDir::at(root.join("fact_log"), "fact_log");
    FactLogPlugin::new(dir, FactLogConfig::default())
}

/// 构造一个最小 invoke 上下文（只有 `PATH`）。
fn ctx(path: &str) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(crate::symbio_core::PATH, path.to_string());
    Arc::new(req)
}

fn data_of(p: PluginPayload) -> serde_json::Value {
    match p {
        PluginPayload::Data(d) => d.serialize().unwrap(),
        other => panic!(
            "expected data payload, got {:?}",
            std::mem::discriminant(&other)
        ),
    }
}

/// 写一份最小会话目录（只依赖公开布局，不 import session）。
///
/// ⚠️ 落盘形状是 **`{"messages": [...]}`**（带一层包装，见 `session::store::MessagesFile`）
/// ——不是裸数组。这里手工包一层，正是为了锁住「本插件读的是**真实**磁盘形状」
/// 这条约定：`read_messages` 一旦误当裸数组读，这些测试会立刻全红。
fn write_session(root: &std::path::Path, id: &str, msgs_json: &str) {
    let dir = root.join("session").join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("messages.json"),
        format!(r#"{{"messages":{msgs_json}}}"#),
    )
    .unwrap();
}

/// 从磁盘派生：两条会话 → 事实序列，`seq` 严格递增且确定性。
#[tokio::test]
async fn derives_from_disk_layout() {
    let tmp = std::env::temp_dir().join(format!("factlog-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    write_session(
        &tmp,
        "s-b",
        r#"[{"id":"u1","role":"user","type":"text","content":"hi","seq":1,"timestamp":10}]"#,
    );
    write_session(
        &tmp,
        "s-a",
        r#"[{"id":"u1","role":"user","type":"text","content":"yo","seq":1,"timestamp":20}]"#,
    );

    let p = plugin_at(&tmp);
    let facts = p.facts(0).await.unwrap();
    assert_eq!(facts.len(), 2, "两条会话各一条用户事实");
    // 确定性：s-a 排在 s-b 前（字典序）
    assert_eq!(facts[0].principal.as_str(), "s-a");
    assert_eq!(facts[1].principal.as_str(), "s-b");
    assert!(facts[0].seq < facts[1].seq, "seq 应递增");

    let _ = std::fs::remove_dir_all(&tmp);
}

/// 无会话目录 → 空事实，**不报错**（平凡值）。
#[tokio::test]
async fn no_sessions_is_empty_not_error() {
    let tmp = std::env::temp_dir().join(format!("factlog-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let p = plugin_at(&tmp);
    let facts = p.facts(0).await.unwrap();
    assert!(facts.is_empty(), "无会话应返回空事实，而不是错误");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// 损坏的 messages.json → 该会话被跳过（不让一条坏数据毁掉整个派生）。
#[tokio::test]
async fn malformed_session_is_skipped() {
    let tmp = std::env::temp_dir().join(format!("factlog-bad-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    write_session(
        &tmp,
        "good",
        r#"[{"id":"u1","role":"user","type":"text","seq":1}]"#,
    );
    write_session(&tmp, "bad", "{ this is not json ");
    let p = plugin_at(&tmp);
    let facts = p.facts(0).await.unwrap();
    assert_eq!(facts.len(), 1, "坏会话应被跳过，好会话照常派生");
    assert_eq!(facts[0].principal.as_str(), "good");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// 目录里非目录项 / 缺 messages.json 的目录都不算会话。
#[tokio::test]
async fn only_real_sessions_count() {
    let tmp = std::env::temp_dir().join(format!("factlog-shape-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("session").join("no-msgs")).unwrap();
    std::fs::write(tmp.join("session").join("a-file.txt"), "x").unwrap();
    write_session(
        &tmp,
        "real",
        r#"[{"id":"u1","role":"user","type":"text","seq":1}]"#,
    );
    let p = plugin_at(&tmp);
    let facts = p.facts(0).await.unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].principal.as_str(), "real");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// `list` 路由：返回 **JSON 数组**（`Fact` 的线上形状），供审计/调试/测试观察。
#[tokio::test]
async fn list_route_returns_facts_json() {
    let tmp = std::env::temp_dir().join(format!("factlog-route-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    write_session(
        &tmp,
        "s1",
        r#"[{"id":"u1","role":"user","type":"text","seq":1,"timestamp":7},
            {"id":"a1","role":"assistant","type":"text","seq":2,"timestamp":8}]"#,
    );
    let p = Arc::new(plugin_at(&tmp));
    let resp = p.clone().route(ctx("list")).await.expect("list 必须成功");
    let v = data_of(resp);
    let arr = v.as_array().expect("list 返回应为数组");
    assert_eq!(arr.len(), 2, "两条消息 → 两条事实");
    // 形状：seq / kind / principal / caused_by / at_ms / payload 六字段齐备
    // seq = session_ordinal(=1) * SESSION_STRIDE + local_seq
    let base = 1u64 << 40;
    assert_eq!(arr[0]["seq"], serde_json::json!(base + 1));
    assert_eq!(arr[0]["principal"], serde_json::json!("s1"));
    assert!(arr[0].get("kind").is_some());
    // 助手事实溯源到用户事实（I2）
    assert_eq!(arr[1]["caused_by"], serde_json::json!(base + 1));
    let _ = std::fs::remove_dir_all(&tmp);
}

/// 未知子命令 → `NotFound`（本插件只有 `list` 一条路由）。
#[tokio::test]
async fn unknown_route_is_not_found() {
    let tmp = std::env::temp_dir().join(format!("factlog-nf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let p = Arc::new(plugin_at(&tmp));
    let r = p.route(ctx("bogus")).await;
    assert!(matches!(r, Err(PluginError::NotFound(_))));
    let _ = std::fs::remove_dir_all(&tmp);
}
