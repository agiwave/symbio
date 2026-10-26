//! `symbio_core::agent` 的判据单测。
//!
//! 这个模块的判据**全是安全边界**，所以每条都写成「越界必拒」的正反两形：
//! 空清单一律拒（grant 语义）、`*` 不跨段（否则 `assets/*` 会放行
//! `assets/../../etc` 之外的深层穿透）、标签边界比（`evil-example.com` 不中
//! `*.example.com`）。任何一条放松都是把边界改成装饰。

use super::*;
use crate::{PluginInvokeRequestExt, PluginSimpleRequest};

fn perms(yaml: serde_json::Value) -> AgentPermissions {
    serde_json::from_value(yaml).expect("声明可解析")
}

// ── glob：段语义 ───────────────────────────────────────────────────────────

#[test]
fn a_double_star_crosses_segments_and_a_single_star_does_not() {
    assert!(glob_matches("assets/**", "assets/icon.svg"));
    assert!(glob_matches("assets/**", "assets/deep/nested/x.bin"));
    // `*` 不跨段：这一条是「穿透防护」的判据——`assets/*` 放行 `assets/a/b`
    // 等于把整棵子树都放行了，而作者写的只是一个段。
    assert!(glob_matches("assets/*", "assets/icon.svg"));
    assert!(!glob_matches("assets/*", "assets/deep/x.bin"));
    // 前缀不是段：`assets/**` 不匹 `assetsx`。
    assert!(!glob_matches("assets/**", "assetsx/icon.svg"));
}

#[test]
fn the_agent_root_itself_is_matched_only_by_root_shaped_patterns() {
    // 空路径 = Agent 根本身。`.` 是规范里「相对 Agent 根」的写法，不是通配符。
    assert!(glob_matches(".", ""));
    assert!(glob_matches("**", ""));
    assert!(!glob_matches("assets/**", ""));
    assert!(!glob_matches("*", ""));
}

#[test]
fn question_mark_matches_exactly_one_character() {
    assert!(glob_matches("v?.md", "v1.md"));
    assert!(!glob_matches("v?.md", "v12.md"));
    assert!(!glob_matches("v?.md", "v.md"));
}

#[test]
fn a_declaration_with_a_parent_segment_is_invalid() {
    // `..` 一进清单，整份 glob 就失去意义（`../..` + `**` = 整个磁盘）。
    assert!(!glob_is_valid("../**"));
    assert!(!glob_is_valid("a/../../b"));
    assert!(!glob_is_valid("..\\..\\x"));
    assert!(!glob_is_valid("   "));
    assert!(glob_is_valid("assets/**"));
    assert!(glob_is_valid("."));
    assert!(glob_is_valid("memory/AGENTS.md"));
}

// ── fs 清单：空即拒 ────────────────────────────────────────────────────────

#[test]
fn an_empty_fs_list_denies_everything() {
    let p = perms(serde_json::json!({}));
    assert!(!p.path_allowed("AGENTS.md", false), "没声明读 ⇒ 读不了");
    assert!(!p.path_allowed("AGENTS.md", true), "没声明写 ⇒ 写不了");
}

#[test]
fn read_and_write_are_two_independent_lists() {
    let p = perms(serde_json::json!({
        "fs": { "read": ["**"], "write": ["memory/**"] }
    }));
    assert!(p.path_allowed("AGENTS.md", false), "读清单放行全部");
    assert!(
        p.path_allowed("memory/AGENTS.md", true),
        "写清单只放行 memory"
    );
    assert!(!p.path_allowed("AGENTS.md", true), "人格文件不在写清单里");
    assert!(!p.path_allowed("skill/x/SKILL.md", true), "技能目录不可写");
    // 读侧不受写侧收窄影响（两个清单不是同一个的补集）。
    assert!(p.path_allowed("skill/x/SKILL.md", false));
}

// ── network 清单：空即拒 + 标签边界 ────────────────────────────────────────

#[test]
fn an_empty_domain_list_denies_every_host() {
    let p = perms(serde_json::json!({ "network": { "domains": [] } }));
    assert!(!p.host_allowed("api.github.com"));
    assert!(!p.host_allowed("example.com"));
}

#[test]
fn a_wildcard_matches_the_bare_domain_and_subdomains_only() {
    let p =
        perms(serde_json::json!({ "network": { "domains": ["*.example.com", "API.GitHub.com"] } }));
    assert!(p.host_allowed("example.com"), "裸域名本身中");
    assert!(p.host_allowed("api.example.com"), "子域中");
    assert!(p.host_allowed("api.github.com"), "大小写不敏感");
    // 按**标签边界**比：字符串后缀会让这一个也中，那是穿透。
    assert!(!p.host_allowed("evil-example.com"));
    assert!(!p.host_allowed("notexample.com"));
    assert!(!p.host_allowed("example.com.evil.io"));
}

#[test]
fn a_bare_star_allows_every_host() {
    let p = perms(serde_json::json!({ "network": { "domains": ["*"] } }));
    assert!(p.host_allowed("anything.example.org"));
}

// ── limits：装载期判据 ─────────────────────────────────────────────────────

#[test]
fn limits_are_judged_against_declared_ceilings() {
    let p = perms(serde_json::json!({ "limits": { "max_tools": 2, "max_prompt_bytes": 10 } }));
    assert!(p.tools_ok(2), "等于上限 ⇒ 过");
    assert!(!p.tools_ok(3), "超一件 ⇒ 拒载");
    assert!(p.prompt_bytes_ok(10));
    assert!(!p.prompt_bytes_ok(11));
}

#[test]
fn undeclared_limits_fall_back_to_defaults_not_to_no_check() {
    // 没写 limits ≠ 不判：缺省值就是判据（否则「声明一个上限」可以靠省略绕过）。
    let p = perms(serde_json::json!({}));
    assert!(p.tools_ok(32));
    assert!(!p.tools_ok(33));
    assert!(p.prompt_bytes_ok(65_536));
    assert!(!p.prompt_bytes_ok(65_537));
}

// ── 作用域：自己的包 ───────────────────────────────────────────────────────

fn scope(id: &str, root: &str, yaml: serde_json::Value) -> AgentScope {
    AgentScope {
        id: id.to_string(),
        root: PathBuf::from(root),
        permissions: perms(yaml),
    }
}

#[test]
fn the_virtual_address_of_ones_own_package_is_judged() {
    let s = scope(
        "reviewer",
        "/home/me/.symbio/agent/reviewer",
        serde_json::json!({ "fs": { "read": ["assets/**"] } }),
    );
    assert!(s.path_allowed_addr("@vfs", "@vfs/agent/reviewer/assets/x.png", false));
    // 根本身（条目概览）也要读权限——`. ` 或 `**` 才放行。
    assert!(!s.path_allowed_addr("@vfs", "@vfs/agent/reviewer", false));
    assert!(!s.path_allowed_addr("@vfs", "@vfs/agent/reviewer/AGENTS.md", false));
    // 别人的包不归本声明管（没有谁的 manifest 能描述别人的树）。
    assert!(s.path_allowed_addr("@vfs", "@vfs/agent/other/AGENTS.md", false));
    // 用户的工作目录也不归它管。
    assert!(s.path_allowed_addr("@vfs", "src/main.rs", false));
}

#[test]
fn a_physical_path_into_the_own_package_is_judged_too() {
    // 模型看得见真实目录（`agent_dir_info` 把 `dir` 印在概览里），所以「换成
    // 绝对路径」必须同案处理——漏这一条就是一条能绕过的旁路。
    let (root, inside, outside) = if cfg!(windows) {
        (
            r"C:\U\agent\reviewer",
            r"C:\U\agent\reviewer\memory\AGENTS.md",
            r"C:\U\agent\reviewer\AGENTS.md",
        )
    } else {
        (
            "/home/me/.symbio/agent/reviewer",
            "/home/me/.symbio/agent/reviewer/memory/AGENTS.md",
            "/home/me/.symbio/agent/reviewer/AGENTS.md",
        )
    };
    let s = scope(
        "reviewer",
        root,
        serde_json::json!({ "fs": { "write": ["memory/**"] } }),
    );
    assert!(
        s.path_allowed_addr("@vfs", inside, true),
        "写清单放行 memory"
    );
    assert!(
        !s.path_allowed_addr("@vfs", outside, true),
        "人格文件不可写"
    );
    // 目录外的绝对路径不归它管。
    assert!(s.path_allowed_addr("@vfs", "/etc/passwd", true));
}

#[test]
fn a_sibling_directory_named_after_the_root_is_not_inside_it() {
    // `<root>-evil/x` 与 `<root>/x` 只差一个字符；按字符串前缀切就会把兄弟
    // 目录当成自己的子树（域名按标签边界比的同一条教训，路径侧的同形）。
    let (root, inside, sibling) = if cfg!(windows) {
        (
            r"C:\a\agent\reviewer",
            r"C:\a\agent\reviewer\x",
            r"C:\a\agent\reviewer-evil\x",
        )
    } else {
        (
            "/a/agent/reviewer",
            "/a/agent/reviewer/x",
            "/a/agent/reviewer-evil/x",
        )
    };
    let s = scope(
        "reviewer",
        root,
        serde_json::json!({ "fs": { "read": ["**"] } }),
    );
    assert!(s.own_rel("@vfs", inside).is_some(), "自己在里面");
    assert!(s.own_rel("@vfs", sibling).is_none(), "兄弟目录不在里面");
    // 虚拟侧同案：前缀带尾部分隔符，`reviewer-evil` 匹不上。
    assert!(s.own_rel("@vfs", "@vfs/agent/reviewer-evil/x").is_none());
}

// ── ctx 通道：建槽 → 发布 → 读 ────────────────────────────────────────────

#[test]
fn the_slot_round_trips_through_forks() {
    // fork 复制的是 Arc：收集期插件写在自己那份 ctx 上，轮 ctx 与工具 ctx
    // 都必须读得到——这是整条通道存在的理由。
    let collector: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    agent_perms_init_slot(&collector);
    assert!(
        agent_perms_of(collector.as_ref()).is_none(),
        "建了槽但没发布 ⇒ 读不到"
    );

    let child = collector.fork();
    scope("reviewer", "/a/agent/reviewer", serde_json::json!({})).publish(&child);
    let published = agent_perms_of(collector.as_ref()).expect("同一份 Arc ⇒ 收集方读得到");
    assert_eq!(published.id, "reviewer");

    // 轮 ctx 把同一个 Arc 挂回去（collect_capabilities 的那一步），工具再 fork。
    let turn = collector.fork();
    turn.set(AGENT_PERMS, collector.get(AGENT_PERMS).unwrap());
    let tool = turn.fork();
    assert_eq!(
        agent_perms_of(tool.as_ref()).map(|s| s.id).as_deref(),
        Some("reviewer"),
        "工具 ctx 读得到"
    );
}

#[test]
fn a_second_publish_is_ignored_not_silently_overwritten() {
    // 一次收集只应有一个 Agent 被选中；第二个发布若覆盖成功，「以谁的身份跑」
    // 就取决于插件遍历顺序（HashMap，不确定）。
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    agent_perms_init_slot(&ctx);
    scope("first", "/a/first", serde_json::json!({})).publish(&ctx);
    scope("second", "/a/second", serde_json::json!({})).publish(&ctx);
    assert_eq!(
        agent_perms_of(ctx.as_ref()).map(|s| s.id).as_deref(),
        Some("first")
    );
}

#[test]
fn no_slot_means_host_mode_not_deny_all() {
    // 没有选中 Agent 的会话（`agent:main`）无 manifest 可声明 ⇒ 宿主态，
    // 不做 Agent 级过滤。这与「选中了但声明为空 ⇒ 全拒」是两件事，不能混。
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    assert!(agent_perms_of(ctx.as_ref()).is_none());
}
