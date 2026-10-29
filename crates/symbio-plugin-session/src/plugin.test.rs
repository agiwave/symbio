//! `plugin.rs` 模块根的单元测试（结构体 / `impl Plugin` / 路由分发 / 配置定义）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：测试跟着
//! **被测试的实现文件**走——`plugin/nodes.rs` 与 `plugin/vdfs_provider.rs` 的测试
//! 分别在 `plugin/nodes.test.rs`、`plugin/vdfs_provider.test.rs`。
//!
//! 末尾一节锁**已退役路由不得被加回来**：被测对象是 [`Plugin::route`] 的默认分支，
//! 因此住在本文件（原 `handlers.test.rs` 的全部内容——handlers 解散后路由表
//! 只剩 `plugin.rs` 一处）。

use super::*;
// 未装配容器时没有 PLUGIN_DIR，配置文件落盘目标指个临时目录
use crate::test_dir;
use symbio_core::{
    capability_prompts, Registry, CAPABILITY_VISITOR, PATH, TRAVERSE_AVAILABLE_TOOLS,
    VDFS_PARENT_ADDR,
};
use symbio_providers::DefaultToolVisitor;

/// 会话存储根**只**来自构造时父插件经 `PLUGIN_DIR` 告知的插件目录——
/// **不读全局 homedir**（那在子智能体里会指错作用域）。
#[test]
fn test_session_storage_dir_comes_from_plugin_dir() {
    let root = std::env::temp_dir().join("symbio-test-session-storage");
    let plugin = SessionPlugin::new(
        None,
        SessionConfig::default(),
        PluginDir::at(&root, PLUGIN_ID_SESSION),
    );
    assert_eq!(plugin.storage_dir(), root, "存储根必须等于被传入的插件目录");
    assert!(
        plugin.storage_dir().is_absolute(),
        "必须是绝对路径: {}",
        plugin.storage_dir().display()
    );
}

/// 验证 SessionConfig 不再包含已删除的死字段：
/// - `storage_dir`（存储根 = 本插件自己的目录，经 `PLUGIN_DIR` 告知）
/// - `session_id`（零消费者；id 由会话目录名决定，配置内自指冗余）
#[test]
fn test_session_config_has_no_dead_fields() {
    let cfg = SessionConfig::default();
    let json = serde_json::to_value(&cfg).unwrap();
    for key in ["storage_dir", "session_id"] {
        assert!(
            json.get(key).is_none(),
            "SessionConfig 不应再包含 {key} 字段, got: {json}"
        );
    }
}

/// 验证从含旧字段的配置反序列化时，未知键被静默忽略（旧 session_config.json
/// 无需迁移即可继续加载）。
#[test]
fn test_session_config_deserialize_ignores_legacy_keys() {
    let json = serde_json::json!({
        "storage_dir": "/tmp/should_be_ignored",
        "session_id": "stale-id-should-be-ignored",
        "max_messages": 42,
    });
    let cfg: SessionConfig = serde_json::from_value(json).unwrap();
    assert_eq!(cfg.max_messages, 42, "max_messages 应被正确反序列化");
}

// ==================== 配置契约一致性 ====================

// `max_tool_rounds`（工具轮软上限）整条链已退役：它在 v2 判的是一个恒 0 的计数，
// 裁决与去处见 [D-074](../../../docs/003-design/014-单环一表.md#d-074-恒-0-的轮内计数退出主循环工具轮软上限整条退役抽干点搬到批次边界 "DE-014-单环一表")。这里曾有的两条契约测试（0 → None 的翻译、默认值 =
// 不限制）随被测对象一起删除，不是覆盖丢失。

// ==================== 配置文档（`<根>/session/PLUGIN.yml`） ====================

// ==================== 配置文档（`<根>/session/PLUGIN.yml`） ====================

/// **定义与配置同源**：面板字段的默认值一律来自 `SessionConfig::default()`，
/// 且 serde 默认值函数与 `Default` impl 不漂移（两处各自书写必然漂移）。
///
/// 遍历**全部**分区——定义是多分区的（基础 / 上下文与工具 / 记忆 / 对话面机制 /
/// 自主与学习），只看 `sections[0]` 会让后面四个分区的默认值无人看守。
#[test]
fn config_definition_defaults_come_from_session_config() {
    let defaults = serde_json::to_value(SessionConfig::default()).unwrap();
    let def = config_definition();
    let mut checked = 0usize;
    for f in def.sections.iter().flat_map(|s| &s.fields) {
        checked += 1;
        let declared = f
            .default
            .clone()
            .unwrap_or_else(|| panic!("字段 {} 缺少 default", f.key));
        let actual = defaults
            .get(&f.key)
            .unwrap_or_else(|| panic!("SessionConfig 不存在字段 {}", f.key));
        assert_eq!(
            &declared, actual,
            "面板 {}.default={declared} 与 SessionConfig::default().{}={actual} 不一致",
            f.key, f.key
        );
    }
    assert!(checked > 0, "会话配置必须有字段");

    let from_empty: SessionConfig =
        serde_json::from_str("{}").expect("空对象应能反序列化出默认配置");
    assert_eq!(
        serde_json::to_value(&from_empty).unwrap(),
        defaults,
        "SessionConfig 的 serde 默认值与 Default impl 漂移了（两处各自书写）"
    );
}

/// **覆盖判据**：面板定义必须登记 `SessionConfig` 的**每一个**字段——键集合
/// **完全相等**，无豁免。
///
/// ## 为什么「少一个字段」是要害，不是样式问题
///
/// 少登记的字段就是「能力在后端存在、产品里既设不了也看不见」：`skill_compile_enabled`
/// （技能编译）与 `conation_enabled`（自主层）这两个**高阶能力的开关**曾长期
/// 只躺在 `session/PLUGIN.yml` 里，前端设置页没有任何控件——用户根本开不了它们，
/// 于是那些能力的「存在」在导航层不可达，等于虚假实现。
///
/// 判据与 [`config_definition_defaults_come_from_session_config`] 分工：那条查
/// 「同名字段的默认值一致」，本条查「字段一个不少、也不多、不重复」。两条都过，
/// 才成立「定义 = 配置的完整镜像」。
///
/// 反向也查：声明了 `SessionConfig` 不存在的字段 ⇒ 保存时该字段必然失败（后端
/// 反序列化会丢弃未知键），是另一种形态的假开关。
#[test]
fn config_definition_covers_every_session_config_field() {
    use std::collections::BTreeSet;

    let def = config_definition();
    let mut declared: BTreeSet<String> = BTreeSet::new();
    for f in def.sections.iter().flat_map(|s| &s.fields) {
        assert!(
            declared.insert(f.key.clone()),
            "字段 {} 在多处重复声明（同一键两个控件会互相覆盖）",
            f.key
        );
    }
    let actual: BTreeSet<String> = serde_json::to_value(SessionConfig::default())
        .unwrap()
        .as_object()
        .expect("SessionConfig 序列化必须是 JSON 对象")
        .keys()
        .cloned()
        .collect();

    let missing: Vec<&String> = actual.difference(&declared).collect();
    assert!(
        missing.is_empty(),
        "以下 SessionConfig 字段没有面板控件（后端存在、产品里开不了）: {missing:?}"
    );
    let extra: Vec<&String> = declared.difference(&actual).collect();
    assert!(
        extra.is_empty(),
        "面板声明了 SessionConfig 不存在的字段（保存必被丢弃）: {extra:?}"
    );
}

// ==================== 会话记忆（`<根>/session/<id>/MEMORY.md`）====================
//
// 机制（读写 / 限容 / 截断 / 排版）已在 `providers/memory/tests.rs` 与
// `session/memory.test.rs` 钉住；这里只测**收集期**这一侧：什么情况下注入、
// 注入的那一段长什么样。

/// 构造带能力收集器的上下文；`session_id` 为 `None` 即「没有会话上下文」
fn collect_ctx(
    session_id: Option<&str>,
) -> (Arc<dyn PluginInvokeRequest>, Arc<DefaultToolVisitor>) {
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(symbio_core::PluginSimpleRequest::new(None, None));
    if let Some(sid) = session_id {
        ctx.set(SESSION_ID, sid.to_string());
    }
    ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    // 合成父地址：模拟容器转发时写入的当前父地址（不依赖真实挂载名）
    ctx.set(VDFS_PARENT_ADDR, "@vfs/session".to_string());
    let visitor = Arc::new(DefaultToolVisitor::new());
    ctx.set(
        CAPABILITY_VISITOR,
        Arc::clone(&visitor) as Arc<dyn Registry>,
    );
    (ctx, visitor)
}

/// 本用例独占的会话 id（会话存储目录取自全局 homedir，复用 id 会串扰）
fn scratch_id(tag: &str) -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{tag}-{}-{n}", std::process::id())
}

/// 取本插件注册的那一段系统提示词
async fn memory_segment(visitor: &Arc<DefaultToolVisitor>) -> Option<String> {
    capability_prompts(&**visitor)
        .await
        .into_iter()
        .find(|(n, _)| n == crate::memory::SEGMENT_NAME)
        .map(|(_, t)| t)
}

/// 没有会话上下文 → 不注入（「本会话的记忆」无从谈起，这不是故障）
#[tokio::test]
async fn no_session_context_injects_no_memory() {
    let p = Arc::new(SessionPlugin::new(
        None,
        SessionConfig::default(),
        test_dir(),
    ));
    let (ctx, visitor) = collect_ctx(None);

    p.clone().traverse(String::new(), ctx).await.unwrap();

    assert!(
        memory_segment(&visitor).await.is_none(),
        "没有会话 id 时不得凭空造一份记忆出来"
    );
    // 但视图照旧交出（容器据此派生挂载行）：挂载点的存在不依赖某次请求的作用域
    assert!(p.get_vfs_provider().is_some());
}

/// 空串 / 纯空白的会话 id 与「没有会话」同解（闸门在 `store` 一处收口）
#[tokio::test]
async fn blank_session_id_is_treated_as_no_session() {
    let p = Arc::new(SessionPlugin::new(
        None,
        SessionConfig::default(),
        test_dir(),
    ));
    for blank in ["", "   "] {
        let (ctx, visitor) = collect_ctx(Some(blank));
        p.clone().traverse(String::new(), ctx).await.unwrap();
        assert!(memory_segment(&visitor).await.is_none(), "blank={blank:?}");
    }
}

/// 还没写过 → 仍然注入一段（教会模型「你可以往这里写」），
/// 头信息里带**地址**与**容量**（这是它区别于只读指令的地方）
#[tokio::test]
async fn empty_memory_still_teaches_where_and_how_big() {
    let p = Arc::new(SessionPlugin::new(
        None,
        SessionConfig::default(),
        test_dir(),
    ));
    let id = scratch_id("mem-empty");
    let (ctx, visitor) = collect_ctx(Some(&id));

    p.traverse(String::new(), ctx).await.unwrap();

    let seg = memory_segment(&visitor).await.expect("有会话就应注入");
    assert!(seg.contains("【会话记忆】"), "{seg}");
    assert!(
        seg.contains(&format!("@vfs/session/{id}/MEMORY.md")),
        "地址必须真实可达：{seg}"
    );
    assert!(
        seg.contains(&format!(
            "{}字节",
            SessionConfig::default().memory_max_bytes
        )),
        "上限来自本插件配置：{seg}"
    );
    assert!(
        seg.contains("暂无内容"),
        "空记忆也要教会模型怎么建立：{seg}"
    );
    assert!(
        seg.contains("与【工作区记忆】【智能体记忆】相互独立"),
        "三层同名不同域，必须点明：{seg}"
    );
}

/// 已写入的内容**原样**进片段（不加工、不摘要）
#[tokio::test]
async fn session_memory_is_injected_verbatim() {
    let p = Arc::new(SessionPlugin::new(
        None,
        SessionConfig::default(),
        test_dir(),
    ));
    let id = scratch_id("mem-content");
    // 用**实例自己的目录**（装配态的权威来源），不是无实例回退
    let dir = p.storage_dir().join(&id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(crate::memory::SESSION_MEMORY_FILE),
        "本会话约定：所有时间用 UTC。",
    )
    .unwrap();

    let (ctx, visitor) = collect_ctx(Some(&id));
    p.traverse(String::new(), ctx).await.unwrap();

    let seg = memory_segment(&visitor).await.expect("有会话就应注入");
    assert!(seg.contains("本会话约定：所有时间用 UTC。"), "{seg}");
    assert!(
        seg.contains("改写前先 vdfs_read"),
        "必须教会模型「先读后写」，否则会整篇覆盖：{seg}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

// ==================== 退役路由：不得被加回来 ====================
//
// 被测对象是 `Plugin::route` 的默认分支（`plugin.rs`），所以这里只造插件 + 一个
// 带 `PATH` 的请求上下文：路由在碰任何存储之前就该报错，因此不给它独占存储根。

/// 构造带 payload 的请求上下文（`PluginInvokeRequest::payload` 读的正是 `"payload"` 桶）。
fn ctx_with(payload: serde_json::Value) -> Arc<dyn PluginInvokeRequest> {
    let req = symbio_core::PluginSimpleRequest::new(None, None);
    req.extensions
        .write()
        .unwrap()
        .insert("payload".to_string(), Arc::new(payload));
    Arc::new(req)
}

/// `session/clear` 路由已退役：删除会话的唯一入口是
/// `vdfs/delete(<根>/session/<id>)`。
///
/// 为什么值得锁：退役一条路由**不会**让任何既有测试变红——调用方全改完了，剩下的
/// 只是一个不再被解析的字符串。若哪天有人"顺手"把它加回来，同一件事就又有了两个
/// 入口、两条会各自漂移的实现，而没有任何测试会覆盖它们的一致性。
///
/// 断言**错误消息**而不只是错误类型：变异测试时发现，把 `"clear"` 加回去却让它
/// 返回 `NotFound` 也能骗过"只查类型"的断言——而那种写法与"没有这条路由"行为完全
/// 相同，根本不是回归。真正要锁的是「`clear` 落到了**默认分支**」，那正是
/// `未知路径` 这条消息的出处。
#[tokio::test]
async fn session_clear_route_is_retired() {
    let p = Arc::new(SessionPlugin::new(
        None,
        SessionConfig::default(),
        test_dir(),
    ));
    let ctx = ctx_with(serde_json::json!({ "session_id": "s1" }));
    ctx.set(PATH, "clear".to_string());

    let err = Plugin::route(p, ctx)
        .await
        .expect_err("session/clear 已退役，不该再被解析");

    match err {
        PluginError::NotFound(msg) => assert!(
            msg.contains("未知路径"),
            "clear 应落到默认分支（未知路径），实际消息：{msg}"
        ),
        other => panic!("应报 NotFound（未知路径），实际：{other:?}"),
    }
}

/// 已退役的会话路由**不得被加回来**：2026-09-18 迁往 VDFS 的五条 + 2026-09-23 的
/// `get_messages`（存在性校验改走进程内 VDFS 纯接口 `get_vfs_provider` + `stat`）
/// 与 `update`（会话 metadata 写入收敛为 `vdfs/write`）。
///
/// 每条路径现在都有一个 VDFS 入口
/// ——除了 `chat/clear_messages`：它的替代入口（`action(<sid>/message, "clear")`）
/// 后来也随功能一起下掉了，因为「清空历史」与「删除会话」在用户眼里是同一件事。
/// 与 `session/clear` 同理：退役不会让任何既有测试变红，因此需要一条**正向**的
/// 断言把「这些字符串不再被解析」钉住，否则它们会悄悄长回来。
#[tokio::test]
async fn migrated_session_routes_stay_retired() {
    let p = Arc::new(SessionPlugin::new(
        None,
        SessionConfig::default(),
        test_dir(),
    ));
    for (path, successor) in [
        // 「发言」始终只走聊天协同：
        ("append", "open_chat_session + append_messages"),
        // 无消费方，整条链路（路由 + 实现 + schema）已删
        ("open", "（无替代：本就不需要）"),
        ("chat/update_message", "vdfs/write(<sid>/message/<mid>)"),
        (
            "chat/delete_message",
            "vdfs/action(<sid>/message/<mid>, \"truncate\")",
        ),
        (
            "chat/clear_messages",
            "（无替代：清空与「删除会话」重叠，已整体下线）",
        ),
        (
            "get_messages",
            "进程内 vdfs/stat（get_vfs_provider + stat(<挂载名>/<sid>)）",
        ),
        // 客户端指定会话 id 由「具名目标 + create」承担，不再需要专用路由
        (
            "update",
            "vdfs/write(<根>/session/<id>, {create:true, metadata})",
        ),
    ] {
        let ctx = ctx_with(serde_json::json!({ "session_id": "s1" }));
        ctx.set(PATH, path.to_string());

        let err = Plugin::route(Arc::clone(&p), ctx)
            .await
            .expect_err("已退役的路由不该再被解析");

        match err {
            PluginError::NotFound(msg) => assert!(
                msg.contains("未知路径"),
                "{path} 应落到默认分支（未知路径），实际消息：{msg}\
                 （它的后继是 {successor}）"
            ),
            other => panic!("{path} 应报 NotFound（未知路径），实际：{other:?}"),
        }
    }
}

// ==================== 停机：常驻任务必须真的退出 ====================

/// [D-075](../../../docs/003-design/002-架构总览.md#d-075-卸载能力先于目录分离session-切分立三条前置会话记忆与存储留守 "DE-002-架构总览") 前置 P1 的判据。原判据是「每个插件可独立卸载且系统仍
/// 完整」，而 session 当时不满足它的一半：`stop` 是默认 no-op，心跳 loop 与收件箱消费者
/// 一旦 `tokio::spawn` 就再没有人能取消它们——停用只是从名册里划掉一个名字，线程照跑。
///
/// 断言落在**句柄**上而不是「有没有新回合」：后者要等一个心跳间隔才验得出，而句柄
/// `is_finished()` 直接钉住「这条 loop 已退出」这一事实本身。
///
/// 2b 切片后本插件只剩**收件箱消费者**一条（心跳扫描搬进了 `crates/symbio-plugin-heartbeat`，
/// 判据跟着搬到那边的 `plugin.test.rs`）：这里的 `2 → 1` 是**在册任务变了**，
/// 不是判据放宽——每条常驻 loop 都由它的**所有者**记下句柄，一处也不许漏。
#[tokio::test]
async fn stop_terminates_resident_loops() {
    let plugin = Arc::new(SessionPlugin::new(
        None,
        SessionConfig::default(),
        test_dir(),
    ));
    plugin.start_resident_tasks();
    assert_eq!(
        plugin.resident_tasks.len(),
        1,
        "收件箱消费者该在册（心跳扫描已随 2b 切进 crates/symbio-plugin-heartbeat）"
    );

    Plugin::stop(plugin.clone(), PluginStopReason::Disabled)
        .await
        .expect("停机不该失败");

    assert!(
        plugin.resident_tasks.all_finished(),
        "stop 返回时仍有常驻任务在跑"
    );
    assert!(plugin.is_stopping(), "停机位要对后续代码可见");
}
