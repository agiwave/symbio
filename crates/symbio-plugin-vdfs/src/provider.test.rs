//! `crates/symbio-plugin-vdfs/src/provider.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use async_trait::async_trait;
use serde_json::json;
use std::sync::Mutex;
use symbio_plugin_api::AgentScope;
use symbio_plugin_api::{CapabilityRow, PluginInvokeRequestExt, PluginSimpleRequest, WORKDIR};
use symbio_plugin_api::{VdfsAccess, VdfsContext, VdfsProvider, VdfsResponse};
use symbio_providers::DefaultToolVisitor;

/// 记录收到地址的虚拟层替身（各操作全部直通，便于断言门面转出来的内部口径）
struct Rec {
    seen: Mutex<Vec<String>>,
}

impl Rec {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
        })
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    fn note(&self, p: &str) {
        self.seen.lock().unwrap().push(p.to_string());
    }
}

#[async_trait]
impl VdfsProvider for Rec {
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => {
                self.note(path);
                Ok(VdfsResponse::list(vec![VdfsNode::file(
                    "a.txt",
                    "a.txt",
                    VdfsAccess::READ,
                )]))
            }
            VdfsRequest::Read => {
                self.note(path);
                Ok(VdfsResponse::Read(VdfsContent::text("hello")))
            }
            VdfsRequest::Write { .. } => {
                self.note(path);
                Ok(VdfsResponse::Write(VdfsWriteResponse {
                    name: None,
                    created: true,
                }))
            }
            VdfsRequest::Delete { .. } => {
                self.note(path);
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Mkdir => {
                self.note(path);
                Ok(VdfsResponse::Unit)
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }
}

/// 把替身登记为 **`.vdfsv2` 的服务者**（工具链路取根的唯一途径），
/// 收到的路径一律是树内相对路径（`""`、`<子目录>/…`）。
async fn tool_with_root(root: Arc<Rec>) -> ToolVdfs {
    let visitor: Arc<dyn Registry> = Arc::new(DefaultToolVisitor::new());
    visitor.put(CapabilityRow::mount_root(root)).await;
    ToolVdfs::new(visitor)
}

fn ctx() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

/// 虚拟地址：`.vdfsv2/<子目录>/…` 进虚拟层时剥成树内相对路径 `<子目录>/…`
#[tokio::test]
async fn virtual_address_is_routed_to_root() {
    let rec = Rec::new();
    let vdfs = tool_with_root(rec.clone()).await;
    vdfs.read(&ctx(), ".vdfsv2/setting/appearance")
        .await
        .unwrap();
    assert_eq!(rec.seen(), vec!["setting/appearance"]);
}

/// `.vdfsv2` 本体 = 根目录：列目录进树内口径就是空串
#[tokio::test]
async fn virtual_root_lists_through_the_same_root() {
    let rec = Rec::new();
    let vdfs = tool_with_root(rec.clone()).await;
    vdfs.list(&ctx(), ".vdfsv2").await.unwrap();
    assert_eq!(rec.seen(), vec![""]);
    // `.vdfsv2/` 与 `.vdfsv2` 等价
    vdfs.list(&ctx(), ".vdfsv2/").await.unwrap();
    assert_eq!(rec.seen(), vec!["", ""]);
}

/// 裸地址属于物理层：**不触达**虚拟根，也不被补任何前缀
#[tokio::test]
async fn bare_address_never_reaches_the_virtual_root() {
    let rec = Rec::new();
    let vdfs = tool_with_root(rec.clone()).await;
    // 物理层没有 workdir 参数 → Internal（而不是「未知挂载点」）
    let err = vdfs.read(&ctx(), "README.md").await.unwrap_err();
    assert!(matches!(err, VdfsError::Internal(_)), "应为 {err:?}");
    assert!(
        rec.seen().is_empty(),
        "裸地址不得被翻译成虚拟层路径：{:?}",
        rec.seen()
    );
}

/// workdir 经 `call_params` 透传到物理层（缺了就解析不了相对地址）
#[tokio::test]
async fn workdir_reaches_the_physical_layer() {
    let dir = std::env::temp_dir().join(format!("symbio-toolvdfs-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hello.txt"), "hi").unwrap();

    let rec = Rec::new();
    let vdfs = tool_with_root(rec.clone()).await;
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(WORKDIR, dir.to_string_lossy().into_owned());

    let c = vdfs.read(&ctx, "hello.txt").await.unwrap();
    assert_eq!(c.text.as_deref(), Some("hi"));
    assert!(rec.seen().is_empty(), "物理地址不该出现在虚拟层");

    // 工作目录根：`.` 可列、不可读（`/` 已经不是它了 —— 见 D-172）
    let items = vdfs.list(&ctx, ".").await.unwrap();
    assert_eq!(items[0].node.name, "hello.txt");
    assert!(matches!(
        vdfs.read(&ctx, ".").await.unwrap_err(),
        VdfsError::Forbidden(_)
    ));

    // 写入 / 建目录 / 删除走同一条物理通道
    vdfs.write(&ctx, "sub/new.txt", &VdfsContent::text("x"))
        .await
        .unwrap();
    assert!(dir.join("sub/new.txt").exists());
    vdfs.mkdir(&ctx, "empty-dir").await.unwrap();
    assert!(dir.join("empty-dir").is_dir());
    vdfs.delete(&ctx, "sub/new.txt", false).await.unwrap();
    assert!(!dir.join("sub/new.txt").exists());
    vdfs.delete(&ctx, "sub", true).await.unwrap();
    assert!(!dir.join("sub").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

// 两条 `move` 用例（跨半被拒 / 同半转发）已随移动整条下线删除。
// `ToolVdfs` 不再有 `move_item`，跨半也不再可能（载荷里没有第二个地址）。

/// 审计只看**绝对地址**：相对地址恒在工作目录内；虚拟地址不是磁盘路径
#[test]
fn audit_flags_only_absolute_changes_outside_workdir() {
    let wd = std::env::temp_dir();
    let inside = wd.join("symbio-audit").join("a.txt");
    let outside = wd.parent().unwrap().join("symbio-outside-a.txt");

    assert!(!outside_workdir(&wd.to_string_lossy(), "README.md"));
    assert!(!outside_workdir(&wd.to_string_lossy(), ".vdfsv2/session"));
    assert!(!outside_workdir(
        &wd.to_string_lossy(),
        &inside.to_string_lossy()
    ));
    assert!(outside_workdir(
        &wd.to_string_lossy(),
        &outside.to_string_lossy()
    ));
}

/// `..` 穿越在分流阶段就失败，两条链路共享同一条守卫
#[tokio::test]
async fn traversal_is_rejected_on_both_links() {
    let rec = Rec::new();
    let vdfs = tool_with_root(rec.clone()).await;
    for bad in ["../../etc/passwd", ".vdfsv2/../../x"] {
        let err = vdfs.read(&ctx(), bad).await.unwrap_err();
        assert!(matches!(err, VdfsError::Invalid(_)), "{bad} 应被拒");
        assert!(err.to_string().contains("向上穿越"));
    }
    assert!(rec.seen().is_empty());
}

/// edit = read → 精确替换 → write 的组合（组合逻辑对两半同构）
#[tokio::test]
async fn edit_composes_read_and_write() {
    let rec = Rec::new();
    let vdfs = tool_with_root(rec.clone()).await;
    let r = vdfs
        .edit(&ctx(), ".vdfsv2/a.txt", "hello", "world")
        .await
        .unwrap();
    assert_eq!(r.replaced, 1);
    assert_eq!(rec.seen(), vec!["a.txt", "a.txt"], "先 read 后 write");

    let err = vdfs
        .edit(&ctx(), ".vdfsv2/a.txt", "nope", "x")
        .await
        .unwrap_err();
    assert!(matches!(err, VdfsError::Invalid(_)));
}

/// search = 递归 list + glob 过滤的组合（走 `list`，各层的安全规则随之生效）
#[tokio::test]
async fn search_composes_list_walk() {
    let rec = Rec::new();
    let vdfs = tool_with_root(rec.clone()).await;
    let r = vdfs.search(&ctx(), ".vdfsv2/x", "*.txt").await.unwrap();
    assert_eq!(r.results, vec![".vdfsv2/x/a.txt"], "结果与请求地址同坐标系");
    assert!(!r.truncated);
    assert_eq!(rec.seen(), vec!["x"], "搜索走虚拟层的 list");
}

/// 无容器注册根时降级为空文件系统（而非报错），物理层照常可用
#[tokio::test]
async fn missing_root_degrades_to_empty_virtual_layer() {
    let visitor: Arc<dyn Registry> = Arc::new(DefaultToolVisitor::new());
    let vdfs = ToolVdfs::new(visitor);
    let items = vdfs.list(&ctx(), ".vdfsv2").await.unwrap();
    assert!(items.is_empty(), "无容器 = 无资源类别，但仍是一棵合法的树");
}

// ── Agent 权限路径闸（D-153）──────────────────────────────────────────────
//
// 判据落在 `ToolVdfs` 这一个入口上（九个工具共用），所以反向用例也只需要一条：
// **越界必拒，且请求根本没有到 provider**（`seen` 空）——后者是「零副作用」的
// 可观测形状：闸门在派发之前，不是拦在落盘那一刻。

/// 带权限作用域的请求上下文（agent 插件发布的那一份的直接等价物）。
fn agent_ctx(
    id: &str,
    root: &std::path::Path,
    perms: serde_json::Value,
) -> Arc<dyn PluginInvokeRequest> {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let scope = AgentScope {
        id: id.to_string(),
        root: root.to_path_buf(),
        permissions: serde_json::from_value(perms).expect("声明可解析"),
    };
    ctx.set(
        symbio_plugin_api::AGENT_PERMS,
        std::sync::Arc::new(std::sync::OnceLock::from(scope)),
    );
    ctx
}

/// 自己包里的路径按 glob 判：清单外的一律拒，且**不触达**虚拟层。
#[tokio::test]
async fn a_path_outside_the_declared_globs_never_reaches_the_provider() {
    let rec = Rec::new();
    let vdfs = tool_with_root(rec.clone()).await;
    let ctx = agent_ctx(
        "reviewer",
        std::path::Path::new("/nowhere/agent/reviewer"),
        json!({ "fs": { "read": ["assets/**"] } }),
    );

    // 清单内：放行
    vdfs.read(&ctx, ".vdfsv2/agent/reviewer/assets/icon.svg")
        .await
        .unwrap();
    // 清单外：拒。人格文件与根本身（条目概览）都不在 `assets/**` 里。
    for bad in [
        ".vdfsv2/agent/reviewer/AGENTS.md",
        ".vdfsv2/agent/reviewer", // 根本身（条目概览）也要读权限
    ] {
        let err = vdfs.read(&ctx, bad).await.unwrap_err();
        assert!(
            matches!(err, VdfsError::Forbidden(_)),
            "{bad} 应为拒绝（实际: {err:?}）"
        );
        assert!(
            err.to_string().contains("fs.read"),
            "拒绝理由须点名是哪个清单：{err}"
        );
    }
    // **别人的包**不归本声明管（没有谁的 manifest 能描述别人的树）：照常放行。
    // 这是作用域边界，不是漏洞——把它写进用例，免得下一个人以为是漏判。
    vdfs.read(&ctx, ".vdfsv2/agent/other/AGENTS.md")
        .await
        .unwrap();
    assert_eq!(
        rec.seen(),
        vec!["agent/reviewer/assets/icon.svg", "agent/other/AGENTS.md"],
        "被拒的请求一个都没到 provider 那儿（零副作用）"
    );
}

/// 读与写是两份清单：能读不代表能写（人格文件可读不可写是这个字段的典型形状）。
#[tokio::test]
async fn read_and_write_lists_are_judged_separately() {
    let rec = Rec::new();
    let vdfs = tool_with_root(rec.clone()).await;
    let ctx = agent_ctx(
        "reviewer",
        std::path::Path::new("/nowhere/agent/reviewer"),
        json!({ "fs": { "read": ["**"], "write": ["memory/**"] } }),
    );

    vdfs.read(&ctx, ".vdfsv2/agent/reviewer/AGENTS.md")
        .await
        .unwrap();
    let err = vdfs
        .write(
            &ctx,
            ".vdfsv2/agent/reviewer/AGENTS.md",
            &VdfsContent::text("改写我"),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("fs.write"), "{err}");
    // 写清单内：放行（子智能体改自己的记忆是声明出来的能力，不是漏洞）。
    vdfs.write(
        &ctx,
        ".vdfsv2/agent/reviewer/memory/AGENTS.md",
        &VdfsContent::text("新记忆"),
    )
    .await
    .unwrap();
    // edit / delete / mkdir 都走写侧（`edit` 先读后写，两关都要过）。
    assert!(vdfs
        .edit(&ctx, ".vdfsv2/agent/reviewer/AGENTS.md", "a", "b")
        .await
        .is_err());
    assert!(vdfs
        .delete(&ctx, ".vdfsv2/agent/reviewer/AGENTS.md", false)
        .await
        .is_err());
    assert!(vdfs
        .mkdir(&ctx, ".vdfsv2/agent/reviewer/skill")
        .await
        .is_err());
    assert_eq!(
        rec.seen(),
        vec![
            "agent/reviewer/AGENTS.md",       // 读（读清单放行）
            "agent/reviewer/memory/AGENTS.md", // 写（写清单放行）
        ],
        "写侧三次调用只有清单内那次到了 provider；edit 连它的 read 都没发生——         闸门在派发之前，不在落盘那一刻"
    );
}

/// 空清单 = 一个授权都没要：自己包里**任何**路径都拒（fail-closed）。
#[tokio::test]
async fn an_empty_fs_list_denies_the_whole_package() {
    let rec = Rec::new();
    let vdfs = tool_with_root(rec.clone()).await;
    let ctx = agent_ctx(
        "reviewer",
        std::path::Path::new("/nowhere/agent/reviewer"),
        json!({}),
    );
    assert!(vdfs
        .read(&ctx, ".vdfsv2/agent/reviewer/AGENTS.md")
        .await
        .is_err());
    assert!(rec.seen().is_empty(), "一个都没放行");
}

/// 没有选中 Agent（宿主态）⇒ 闸门恒过：没有 manifest 可声明。
/// 与「选中了但声明为空 ⇒ 全拒」是两件事，这条用例钉住这个区别。
#[tokio::test]
async fn no_agent_scope_means_host_mode() {
    let rec = Rec::new();
    let vdfs = tool_with_root(rec.clone()).await;
    vdfs.read(&ctx(), ".vdfsv2/agent/reviewer/AGENTS.md")
        .await
        .unwrap();
    assert_eq!(rec.seen(), vec!["agent/reviewer/AGENTS.md"]);
}

/// 换成**物理绝对路径**必须同案：模型从概览的 `dir` 属性看得见真实目录，
/// 「绕过虚拟地址直读磁盘」不能成为一条旁路。
#[tokio::test]
async fn a_physical_path_into_the_package_is_judged_too() {
    let dir = std::env::temp_dir().join(format!("symbio-perms-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("memory")).unwrap();
    std::fs::write(dir.join("AGENTS.md"), "人格").unwrap();
    std::fs::write(dir.join("memory/AGENTS.md"), "记忆").unwrap();

    let rec = Rec::new();
    let vdfs = tool_with_root(rec.clone()).await;
    let ctx = agent_ctx("reviewer", &dir, json!({ "fs": { "read": ["memory/**"] } }));
    // 物理层要求 workdir（`call_params` 透传）：闸门判的是绝对路径，但放行后的
    // 派发照样走物理通道，不能不給。
    ctx.set(WORKDIR, dir.to_string_lossy().into_owned());
    // 绝对路径落在包里：清单外 ⇒ 拒（`AGENTS.md` 不在 `memory/**` 里）。
    let abs = dir.join("AGENTS.md");
    assert!(vdfs.read(&ctx, &abs.to_string_lossy()).await.is_err());
    // 清单内 ⇒ 过（`memory/AGENTS.md`）。
    vdfs.read(&ctx, &dir.join("memory/AGENTS.md").to_string_lossy())
        .await
        .unwrap();
    // 目录外的绝对路径不归它管：闸门放行，物理层照常读（它确实存在）。
    let outside = std::env::temp_dir().join(format!("symbio-outside-{}.txt", std::process::id()));
    std::fs::write(&outside, "包外").unwrap();
    vdfs.read(&ctx, &outside.to_string_lossy()).await.unwrap();
    let _ = std::fs::remove_file(&outside);
}
