//! worker 快照的单元测试：真实嵌套布局、状态推导、坏数据跳过、平凡值与确定性。
//!
//! 夹具按**公开布局**造盘：`<root>/session/<父>/sessions/<子>/{session.json,messages.json}`
//! ——这正是 `session` 存储的嵌套路由（见其 store 模块文档），本插件只依赖约定。
use super::*;
use serde_json::json;

fn temp_root(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("symbio-test-delegate-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 写一个 worker 会话目录（`parent` = 父会话 id；`parent: None` ⇒ 顶层会话，不该被扫到）
fn write_worker(
    root: &Path,
    parent: Option<&str>,
    child: &str,
    title: Option<&str>,
    messages: serde_json::Value,
) {
    let dir = match parent {
        Some(p) => root.join("session").join(p).join("sessions").join(child),
        None => root.join("session").join(child),
    };
    std::fs::create_dir_all(&dir).unwrap();
    let mut meta = json!({ "id": child, "metadata": {} });
    if let Some(p) = parent {
        meta["metadata"]["parent_session_id"] = json!(p);
    }
    if let Some(t) = title {
        meta["title"] = json!(t);
    }
    std::fs::write(dir.join("session.json"), meta.to_string()).unwrap();
    std::fs::write(dir.join("messages.json"), messages.to_string()).unwrap();
}

/// 一段最小消息序列：用户一问 + 助手一次工具调用（已收敛）
fn settled_messages() -> serde_json::Value {
    json!({
        "messages": [
            { "id": "u1", "role": "user", "type": "text", "content": "重构这个模块", "seq": 1, "status": "completed" },
            { "id": "t1", "role": "assistant", "type": "tool_call", "name": "vdfs_write", "seq": 2, "status": "completed" },
            { "id": "a1", "role": "assistant", "type": "text", "content": "已改好", "seq": 3, "status": "completed" }
        ]
    })
}

#[test]
fn scans_nested_worker_sessions() {
    let root = temp_root("scan");
    write_worker(
        &root,
        Some("main"),
        "w-2",
        Some("整理文档"),
        settled_messages(),
    );
    write_worker(&root, Some("main"), "w-1", None, settled_messages());

    let states = scan(&root);
    assert_eq!(states.len(), 2, "两个 worker 都应被扫到");
    assert_eq!(states[0].session_id, "w-1", "按会话 id 升序（确定性）");
    assert_eq!(states[1].session_id, "w-2");
    assert_eq!(states[1].title, "整理文档", "标题取 session.json 的投影");
    assert_eq!(
        states[0].title, "重构这个模块",
        "缺标题时回落到首个用户消息"
    );
    assert_eq!(states[0].rounds, 1);
    assert_eq!(states[0].state, "idle", "无在途消息 ⇒ 已收敛");
    assert_eq!(states[0].last_step, "已改好", "末条助手文本即最新一步");
}

/// 工具调用优先于助手文本：正在用**什么手段**比它刚说了什么更能反映进展。
#[test]
fn tool_call_wins_over_assistant_text() {
    let root = temp_root("step");
    write_worker(
        &root,
        Some("main"),
        "w-tool",
        None,
        json!({
            "messages": [
                { "id": "u1", "role": "user", "type": "text", "content": "跑构建", "seq": 1, "status": "completed" },
                { "id": "a1", "role": "assistant", "type": "text", "content": "我先看看", "seq": 2, "status": "completed" },
                { "id": "t1", "role": "assistant", "type": "tool_call", "name": "shell", "seq": 3, "status": "completed" }
            ]
        }),
    );
    let states = scan(&root);
    assert_eq!(
        states[0].last_step, "调用工具 shell",
        "末条是工具调用 ⇒ 报工具名"
    );
}

/// 在途消息 ⇒ `running`（这是"进展如何"与"已结束"的分界）。
#[test]
fn streaming_message_means_running() {
    let root = temp_root("running");
    write_worker(
        &root,
        Some("main"),
        "w-run",
        None,
        json!({
            "messages": [
                { "id": "u1", "role": "user", "type": "text", "content": "长任务", "seq": 1, "status": "completed" },
                { "id": "a1", "role": "assistant", "type": "text", "content": "正在处理", "seq": 2, "status": "streaming" }
            ]
        }),
    );
    assert_eq!(scan(&root)[0].state, "running");
}

/// 非 worker 目录（无 `parent_session_id`）不进快照——顶层会话不是后台任务。
#[test]
fn top_level_session_is_not_a_worker() {
    let root = temp_root("toplevel");
    write_worker(&root, None, "plain", Some("普通会话"), settled_messages());
    assert!(scan(&root).is_empty());
}

/// 坏数据只跳过它自己：一处坏 JSON 不该毁掉整段快照。
#[test]
fn broken_worker_is_skipped_without_failing() {
    let root = temp_root("broken");
    write_worker(&root, Some("main"), "w-ok", None, settled_messages());
    write_worker(
        &root,
        Some("main"),
        "w-bad",
        None,
        json!({ "messages": "不是数组" }),
    );

    let states = scan(&root);
    assert_eq!(states.len(), 1, "坏的那个被跳过，好的仍在");
    assert_eq!(states[0].session_id, "w-ok");
}

/// 平凡值：没有 worker ⇒ 空表 + 渲染为空串（不注入空标题）。
#[test]
fn no_workers_render_nothing() {
    let root = temp_root("empty");
    let states = scan(&root);
    assert!(states.is_empty());
    assert_eq!(render(&states), "");
}

/// 渲染：一行一个 worker，含状态/轮次/最新一步；双跑逐字节相同（A4）。
#[test]
fn render_is_stable_and_informative() {
    let root = temp_root("render");
    write_worker(
        &root,
        Some("main"),
        "w-1",
        Some("整理文档"),
        settled_messages(),
    );
    let states = scan(&root);
    let out = render(&states);
    assert!(out.starts_with(PROGRESS_TITLE), "{out}");
    assert!(out.contains("w-1（整理文档）"), "{out}");
    assert!(out.contains("已收敛"), "{out}");
    assert!(out.contains("第 1 轮"), "{out}");
    assert!(out.contains("最新一步 已改好"), "{out}");
    assert_eq!(out, render(&scan(&root)), "同一份磁盘双跑渲染逐字节相同");
}

/// 归属过滤：主会话只该看到**自己名下**的 worker（串台 = 把别人的活说成自己的）。
#[test]
fn scan_of_filters_by_parent() {
    let root = temp_root("parent");
    write_worker(
        &root,
        Some("main"),
        "w-mine",
        Some("我的活"),
        settled_messages(),
    );
    write_worker(
        &root,
        Some("other"),
        "w-theirs",
        Some("别人的活"),
        settled_messages(),
    );

    let mine = scan_of(&root, "main");
    assert_eq!(mine.len(), 1, "只报本主会话的 worker");
    assert_eq!(mine[0].session_id, "w-mine");
    assert_eq!(
        mine[0].parent, "main",
        "归属来自磁盘的 parent_session_id，不是目录位置"
    );

    assert_eq!(scan(&root).len(), 2, "不过滤时是全局视图（诊断面用）");
}

/// 空父 id ⇒ 空表（"没说清是谁"时宁可不说，绝不退化成全局）。
#[test]
fn blank_parent_yields_nothing() {
    let root = temp_root("blank");
    write_worker(&root, Some("main"), "w-1", None, settled_messages());
    assert!(scan_of(&root, "  ").is_empty());
}

/// 老布局（消息内联在 `session.json`、无 `messages.json`）也要看得见——
/// 看不到进展比看到旧格式更糟。
#[test]
fn legacy_inline_messages_layout_is_readable() {
    let root = temp_root("legacy");
    let dir = root
        .join("session")
        .join("main")
        .join("sessions")
        .join("w-old");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("session.json"),
        json!({
            "id": "w-old",
            "title": "老会话",
            "metadata": { "parent_session_id": "main" },
            "messages": [
                { "id": "u1", "role": "user", "type": "text", "content": "老任务", "seq": 1 },
                { "id": "a1", "role": "assistant", "type": "text", "content": "老结论", "seq": 2, "status": "completed" }
            ]
        })
        .to_string(),
    )
    .unwrap();
    let states = scan(&root);
    assert_eq!(states.len(), 1, "老布局的 worker 不该被静默丢弃");
    assert_eq!(states[0].rounds, 1);
    assert_eq!(states[0].title, "老会话");
    assert_eq!(states[0].last_step, "老结论");
}
