//! `plugin.rs` 模块根的单元测试（结构体 / `impl Plugin` / 配置定义）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：测试跟着
//! **被测试的实现文件**走——`plugin/nodes.rs` 与 `plugin/vdfs_provider.rs` 的测试
//! 分别在 `plugin/nodes.test.rs`、`plugin/vdfs_provider.test.rs`。

use super::*;
// 未装配容器时没有 PLUGIN_DIR，配置文件落盘目标指个临时目录
use crate::plugins::session::test_dir;
use crate::symbio_core::{
    CapabilityVisitor, DefaultToolVisitor, CAPABILITY_VISITOR, PATH, TRAVERSE_AVAILABLE_TOOLS,
};

/// 验证 session 存储目录**只**从 HomedirRegistry 派生，不依赖 config；
/// 且它就是宿主层的资源类别根（`category_dir(PLUGIN_SESSION)`）——
/// 会话因此不再手拼一份 类别根布局。
#[test]
fn test_session_storage_dir_from_homedir() {
    let dir = SessionPlugin::session_storage_dir();
    let expected = crate::symbio_core::HomedirRegistry::get().join("session");
    assert_eq!(dir, expected, "session_storage_dir 必须等于 <本插件目录>");
    assert_eq!(
        dir,
        crate::providers::vdfs_service::entry::category_dir(PLUGIN_SESSION),
        "会话存储根必须与 VDFS 资源类别根同一条构造式"
    );
    assert!(
        dir.is_absolute(),
        "session_storage_dir 必须是绝对路径: {}",
        dir.display()
    );
}

/// 验证 SessionConfig 不再包含已删除的死字段：
/// - `storage_dir`（存储根由 HomedirRegistry 统一决定）
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

// ==================== 配置契约一致性（复杂度审计 P0-1 / P1-①）====================

/// `max_tool_rounds` 的契约翻译：0 → `None`（不限制），>0 → `Some(n)`（软上限）。
/// 修复前编排层三处无条件 `Some(...)`，使 chat_loop 的 `None` = 无限语义在主路径不可达。
#[test]
fn max_tool_rounds_zero_maps_to_unlimited() {
    let cfg = SessionConfig {
        max_tool_rounds: 15,
        ..Default::default()
    };
    assert_eq!(
        cfg.model_chat_max_tool_rounds(),
        Some(15),
        "显式 >0 的值应作为软上限下发"
    );

    let cfg = SessionConfig {
        max_tool_rounds: 0,
        ..Default::default()
    };
    assert_eq!(
        cfg.model_chat_max_tool_rounds(),
        None,
        "0 必须翻译为 None（不限制），而非 Some(0)（0 轮即熔断）"
    );
}

/// 默认配置的产品意图锁定：默认**不设硬性轮次上限**，以显式语义 `0`（不限制）
/// 表达，而非魔法数 65535（旧默认，行为等价但语义含糊）。
#[test]
fn default_max_tool_rounds_is_effectively_unlimited() {
    let cfg = SessionConfig::default();
    assert_eq!(
        cfg.max_tool_rounds, 0,
        "默认轮次上限必须是 0 = 不限制（用户明确要求不设硬上限）"
    );
    assert_eq!(
        cfg.model_chat_max_tool_rounds(),
        None,
        "默认配置翻译到 Request 层必须是 None（无限轮次）"
    );
}

// ==================== 配置文档（`.vdfs/session/PLUGIN.yml`） ====================

/// **定义与配置同源**：面板字段的默认值一律来自 `SessionConfig::default()`，
/// 且 serde 默认值函数与 `Default` impl 不漂移（两处各自书写必然漂移）。
#[test]
fn config_definition_defaults_come_from_session_config() {
    let defaults = serde_json::to_value(SessionConfig::default()).unwrap();
    let def = config_definition();
    let fields = &def.sections[0].fields;
    assert!(!fields.is_empty(), "会话配置必须有字段");
    for f in fields {
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

    let from_empty: SessionConfig =
        serde_json::from_str("{}").expect("空对象应能反序列化出默认配置");
    assert_eq!(
        serde_json::to_value(&from_empty).unwrap(),
        defaults,
        "SessionConfig 的 serde 默认值与 Default impl 漂移了（两处各自书写）"
    );
}

// ==================== 会话记忆（`.vdfs/session/<id>/AGENTS.md`）====================
//
// 机制（读写 / 限容 / 截断 / 排版）已在 `symbio_core::memory.test.rs` 与
// `session/memory.test.rs` 钉住；这里只测**收集期**这一侧：什么情况下注入、
// 注入的那一段长什么样。

/// 构造带能力收集器的上下文；`session_id` 为 `None` 即「没有会话上下文」
fn collect_ctx(session_id: Option<&str>) -> (Arc<dyn InvokeRequest>, Arc<DefaultToolVisitor>) {
    let ctx: Arc<dyn InvokeRequest> = Arc::new(crate::symbio_core::SimpleRequest::new(None, None));
    if let Some(sid) = session_id {
        ctx.set(SESSION_ID, sid.to_string());
    }
    ctx.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
    let visitor = Arc::new(DefaultToolVisitor::new());
    ctx.set(
        CAPABILITY_VISITOR,
        Arc::clone(&visitor) as Arc<dyn CapabilityVisitor>,
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
    visitor
        .list_system_prompts()
        .await
        .into_iter()
        .find(|(n, _)| n == crate::plugins::session::memory::SEGMENT_NAME)
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

    p.traverse(String::new(), ctx).await.unwrap();

    assert!(
        memory_segment(&visitor).await.is_none(),
        "没有会话 id 时不得凭空造一份记忆出来"
    );
    // 但挂载点照旧交出：挂载点的存在不依赖某次请求的作用域
    assert!(visitor.get_vdfs_provider(PLUGIN_SESSION).await.is_some());
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
        seg.contains(&format!(".vdfs/session/{id}/AGENTS.md")),
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
    let dir = SessionPlugin::session_storage_dir().join(&id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(crate::symbio_core::AGENTS_FILE),
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
