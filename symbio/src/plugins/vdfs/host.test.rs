//! `symbio/src/plugins/vdfs/host.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use super::super::physical::PhysicalFs;
use crate::symbio_core::{
    CapabilityVisitor, DefaultToolVisitor, PluginError, PluginMeta, PluginSimpleRequest,
};
use crate::symbio_core::{VdfsActionResult, VdfsValidationError, VDFS_KIND_DIR};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Mutex;

fn ctx_with(payload: Value) -> Arc<dyn PluginInvokeRequest> {
    let ctx = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set_payload(payload).unwrap();
    ctx
}

fn ctx_empty() -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::new(None, None))
}

/// 测试便捷：不带调用级参数的 [`dispatch_with`]（被测链路都不依赖 params）
async fn dispatch(
    root: &DynVdfsProvider,
    path: &str,
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> Option<PluginInvokeResponse<PluginPayload>> {
    dispatch_with(root, path, ctx, VdfsParams::new()).await
}

/// 内存子树（**自身不含挂载名**）：记录收到的相对路径，便于断言转发语义。
///
/// `""`    → a.txt (file, rw) / sub (dir, lt)
/// `"sub"` → b.md (file, r)
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
                Ok(VdfsResponse::list(match path {
                    "" => vec![
                        VdfsNode::file("a.txt", "A", VdfsAccess::READ_WRITE),
                        VdfsNode::dir("sub", "子目录", VdfsAccess::LIST_TRAVERSE),
                    ],
                    "sub" => vec![VdfsNode::file("b.md", "B", VdfsAccess::READ)],
                    _ => return Err(VdfsError::not_found(format!("无此目录：{path}"))),
                }))
            }
            VdfsRequest::Stat => Ok(VdfsResponse::Stat(match path {
                "a.txt" => VdfsNode::file("a.txt", "A", VdfsAccess::READ_WRITE),
                "sub" => VdfsNode::dir("sub", "子目录", VdfsAccess::LIST_TRAVERSE),
                "sub/b.md" => VdfsNode::file("b.md", "B", VdfsAccess::READ),
                _ => return Err(VdfsError::not_found(path)),
            })),
            VdfsRequest::Read => {
                self.note(path);
                match path {
                    "a.txt" => Ok(VdfsResponse::Read(VdfsContent::text("hello"))),
                    "sub/b.md" => Ok(VdfsResponse::Read(VdfsContent::text("# b"))),
                    _ => Err(VdfsError::Forbidden("目录不可读".into())),
                }
            }
            VdfsRequest::Write { content } => {
                // 演示「provider 自持校验 + 字段级错误」
                if content.text.as_deref() == Some("bad") {
                    return Err(VdfsError::Invalid(
                        VdfsValidationError::new("内容不合法").with_field("text", "不允许 bad"),
                    ));
                }
                Ok(VdfsResponse::Write(VdfsWriteResponse {
                    name: None,
                    created: path == "new.txt",
                    etag: Some("v1".into()),
                }))
            }
            VdfsRequest::Delete { .. } => {
                if path == "sub" || path == "a.txt" {
                    Ok(VdfsResponse::Unit)
                } else {
                    Err(VdfsError::Forbidden(format!("不允许删除 {path}")))
                }
            }
            VdfsRequest::Action { action, .. } => {
                self.note(&format!("action:{action}"));
                Ok(VdfsResponse::Action(VdfsActionResult {
                    action: action.clone(),
                    ok: true,
                    message: format!("{path} 已执行 {action}"),
                    data: None,
                }))
            }
            VdfsRequest::Watch { .. } => {
                self.note(path);
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                self.note(path);
                Ok(VdfsResponse::Unit)
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }
}

/// 极简 provider：验证「什么都不实现」也能被访问层容错（见
/// `unimplemented_ops_surface_as_not_implemented`）
struct Bare;

#[async_trait]
impl VdfsProvider for Bare {
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        _path: &str,
        _req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        Err(VdfsError::NotImplemented)
    }
}

/// 测试用 root 级 provider：首段 = 子目录名，委派给对应 provider（模拟 composite）
struct TestRoot {
    dirs: Vec<(&'static str, DynVdfsProvider)>,
}

fn split_dir(path: &str) -> Option<(&str, &str)> {
    match path.find('/') {
        Some(i) => Some((&path[..i], &path[i + 1..])),
        None if path.is_empty() => None,
        None => Some((path, "")),
    }
}

impl TestRoot {
    fn resolve(&self, path: &str) -> VdfsResult<(&DynVdfsProvider, String)> {
        let (d, rel) = split_dir(path).ok_or_else(|| VdfsError::invalid("根目录不是可操作节点"))?;
        let p = self
            .dirs
            .iter()
            .find(|(n, _)| *n == d)
            .map(|(_, p)| p)
            .ok_or_else(|| {
                let hint = self
                    .dirs
                    .iter()
                    .map(|(n, _)| *n)
                    .collect::<Vec<_>>()
                    .join(", ");
                VdfsError::not_found(format!("目录不存在：{d}（现有：{hint}）"))
            })?;
        Ok((p, rel.to_string()))
    }
}

#[async_trait]
impl VdfsProvider for TestRoot {
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        // 根目录：可列 / 可 stat，其余拒绝
        let Some((d, _rel)) = split_dir(path) else {
            return match req {
                VdfsRequest::List { .. } => Ok(VdfsResponse::list(
                    self.dirs
                        .iter()
                        .map(|(n, _p)| {
                            VdfsNode::dir(n.to_string(), "内存子树", VdfsAccess::LIST_TRAVERSE)
                        })
                        .collect::<Vec<VdfsNode>>(),
                )),
                VdfsRequest::Stat => Ok(VdfsResponse::Stat(VdfsNode::dir(
                    "",
                    "系统",
                    VdfsAccess::LIST_TRAVERSE,
                ))),
                _ => Err(VdfsError::invalid("根目录不是可操作节点")),
            };
        };
        let (p, rel) = self.resolve(path)?;
        match req {
            // 与生产容器同构：**不填条目地址**（它不知道自己挂在哪），
            // 地址由访问层按请求地址回填（`host::fill_paths`）
            VdfsRequest::List { .. } => {
                p.dispatch(
                    ctx,
                    &rel,
                    VdfsRequest::List {
                        limit: None,
                        before: None,
                    },
                )
                .await
            }
            VdfsRequest::Stat => {
                if rel.is_empty() {
                    return Ok(VdfsResponse::Stat(VdfsNode::dir(
                        d.to_string(),
                        "内存子树",
                        VdfsAccess::LIST_TRAVERSE,
                    )));
                }
                p.dispatch(ctx, &rel, VdfsRequest::Stat).await
            }
            VdfsRequest::Read => {
                if rel.is_empty() {
                    return Err(VdfsError::Forbidden(
                        "目录不是可读文件；请读取其子节点".to_string(),
                    ));
                }
                p.dispatch(ctx, &rel, VdfsRequest::Read).await
            }
            VdfsRequest::Write { content } => {
                // 与生产容器同构：`rel` 为空 = 写在**挂载点目录自身**上（「新建」的
                // 机制形态），原样转发。回执里的名字也不加工——地址由调用方用
                // 自己的请求地址拼（`VdfsWriteResponse::name`）。
                p.dispatch(ctx, &rel, VdfsRequest::Write { content }).await
            }
            VdfsRequest::Delete { recursive } => {
                if rel.is_empty() {
                    return Err(VdfsError::Forbidden("目录不可删除".to_string()));
                }
                p.dispatch(ctx, &rel, VdfsRequest::Delete { recursive })
                    .await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Mkdir => {
                if rel.is_empty() {
                    return Err(VdfsError::invalid("目录已存在，无需创建"));
                }
                p.dispatch(ctx, &rel, VdfsRequest::Mkdir).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Action { action, payload } => {
                p.dispatch(ctx, &rel, VdfsRequest::Action { action, payload })
                    .await
            }
            VdfsRequest::Watch { sink } => {
                p.dispatch(ctx, &rel, VdfsRequest::Watch { sink }).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                p.dispatch(ctx, &rel, VdfsRequest::Unwatch).await?;
                Ok(VdfsResponse::Unit)
            }
        }
    }
}

/// 以 `rec` 为唯一子目录（mem）的虚拟层根（模拟 composite）
fn root_with(rec: &Arc<Rec>) -> DynVdfsProvider {
    let p: DynVdfsProvider = rec.clone();
    Arc::new(TestRoot {
        dirs: vec![("mem", p)],
    })
}

/// 被测的统一文件系统：虚拟层 = mem 子目录，物理层 = 磁盘
fn fs_roots() -> (DynVdfsProvider, Arc<Rec>) {
    let rec = Rec::new();
    (
        Arc::new(UnifiedFs::with_physical(
            root_with(&rec),
            Arc::new(PhysicalFs::new()),
        )),
        rec,
    )
}

#[tokio::test]
async fn dispatch_ignores_non_vdfs_path() {
    let (fs, _) = fs_roots();
    let ctx = ctx_empty();
    assert!(dispatch(&fs, "chat/send", &ctx).await.is_none());
    assert!(dispatch(&fs, "", &ctx).await.is_none());
}

/// `vdfs/action`：动作标识与展示地址原样转发，本层不解释语义
#[tokio::test]
async fn action_forwards_verb_and_relative_path() {
    let (fs, rec) = fs_roots();
    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/a.txt", "action": "ping" }));
    let resp = dispatch(&fs, VDFS_ACTION, &ctx).await.unwrap().unwrap();
    let data = resp.get::<VdfsActionResult>().unwrap();
    assert_eq!(data.action, "ping");
    assert!(data.ok);
    assert_eq!(rec.seen(), vec!["action:ping"], "provider 只收到动作标识");

    // 空动作标识 → 拒绝（不打扰 provider）
    let bad = ctx_with(json!({ "path": ".vdfsv2/mem/a.txt", "action": "  " }));
    assert!(dispatch(&fs, VDFS_ACTION, &bad).await.unwrap().is_err());
}

/// 类别根列表：门面把内部口径回填成 `.vdfsv2/...` 展示地址 + `ext` 推导
#[tokio::test]
async fn list_category_root_fills_paths_and_ext() {
    let (fs, rec) = fs_roots();
    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem" }));
    let resp = dispatch(&fs, VDFS_LIST, &ctx).await.unwrap().unwrap();
    let data = resp.get::<VdfsListResponse>().unwrap();
    assert_eq!(rec.seen(), vec![""], "provider 收到的是相对路径 \"\"");
    assert_eq!(data.path, ".vdfsv2/mem");
    assert_eq!(data.node.name, "mem", "目录自身节点 = 类别根");
    assert_eq!(data.items.len(), 2);
    assert_eq!(data.items[0].path, ".vdfsv2/mem/a.txt");
    assert_eq!(data.items[0].node.ext.as_deref(), Some("txt"));
    assert_eq!(data.items[1].path, ".vdfsv2/mem/sub");
    assert!(data.items[1].node.is_dir());
    assert!(data.items[1].node.ext.is_none());
}

/// 深层地址：门面在进出两处各做一次口径映射，provider 始终只见相对路径
#[tokio::test]
async fn deep_virtual_paths_pass_through_relative() {
    let (fs, rec) = fs_roots();
    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/sub" }));
    let resp = dispatch(&fs, VDFS_LIST, &ctx).await.unwrap().unwrap();
    let data = resp.get::<VdfsListResponse>().unwrap();
    assert_eq!(
        rec.seen(),
        vec!["sub"],
        "门面把 .vdfsv2/mem/sub 拆成相对路径 sub"
    );
    assert_eq!(data.items[0].node.name, "b.md");
    assert_eq!(data.items[0].path, ".vdfsv2/mem/sub/b.md");
    assert_eq!(data.node.name, "sub");
}

#[tokio::test]
async fn list_unknown_dir_is_not_found_with_hint() {
    let (fs, _) = fs_roots();
    let ctx = ctx_with(json!({ "path": ".vdfsv2/nope" }));
    let err = dispatch(&fs, VDFS_LIST, &ctx).await.unwrap().unwrap_err();
    assert!(matches!(err, PluginError::NotFound(_)));
    assert!(err.to_string().contains("mem"), "提示现有子目录");
}

/// 路径穿越在访问层被拦截，根与 provider 永远拿到安全路径
#[tokio::test]
async fn traversal_path_rejected() {
    let (fs, _) = fs_roots();
    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/../../etc" }));
    let err = dispatch(&fs, VDFS_LIST, &ctx).await.unwrap().unwrap_err();
    assert!(matches!(err, PluginError::ValidationError(_)));
}

#[tokio::test]
async fn stat_read_and_backfill() {
    let (fs, _) = fs_roots();

    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/a.txt" }));
    let resp = dispatch(&fs, VDFS_READ, &ctx).await.unwrap().unwrap();
    let c = resp.get::<VdfsContent>().unwrap();
    assert_eq!(c.text.as_deref(), Some("hello"));
    // 内容**不带地址**：地址在请求里已经有了，回显没有信息量

    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/sub/b.md" }));
    let n = dispatch(&fs, VDFS_STAT, &ctx)
        .await
        .unwrap()
        .unwrap()
        .get::<VdfsNode>()
        .unwrap();
    assert_eq!(n.effective_ext().as_deref(), Some("md"));
    assert_eq!(n.access.flags(), "r");

    // 类别根节点由组合视图合成（provider 不知道自己的类别名）
    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem" }));
    let n = dispatch(&fs, VDFS_STAT, &ctx)
        .await
        .unwrap()
        .unwrap()
        .get::<VdfsNode>()
        .unwrap();
    assert_eq!(n.kind, VDFS_KIND_DIR);
    assert_eq!(n.name, "mem");
    assert_eq!(n.title, "内存子树");
}

/// 机器级守卫：内容缺失即拒绝，provider 不会被调用
#[tokio::test]
async fn write_requires_content_and_maps_validation_fields() {
    let (fs, _) = fs_roots();

    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/a.txt" }));
    assert!(matches!(
        dispatch(&fs, VDFS_WRITE, &ctx).await.unwrap(),
        Err(PluginError::ValidationError(_))
    ));

    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/a.txt", "text": "x" }));
    let w = dispatch(&fs, VDFS_WRITE, &ctx)
        .await
        .unwrap()
        .unwrap()
        .get::<VdfsWriteResponse>()
        .unwrap();
    assert!(w.name.is_none(), "具名写没有名字可交回");
    assert_eq!(w.etag.as_deref(), Some("v1"));

    // 字段级校验错误：载荷序列化为 JSON 置于错误文案位，可解析还原
    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/a.txt", "text": "bad" }));
    let err = dispatch(&fs, VDFS_WRITE, &ctx).await.unwrap().unwrap_err();
    let PluginError::ValidationError(text) = err else {
        panic!("应为校验错误");
    };
    let parsed: VdfsValidationError = serde_json::from_str(&text).expect("应为结构化校验载荷");
    assert_eq!(parsed.fields[0].field, "text");
}

#[tokio::test]
async fn delete_mkdir_and_dir_root_guards() {
    let (fs, _) = fs_roots();

    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/sub", "recursive": true }));
    assert!(dispatch(&fs, VDFS_DELETE, &ctx).await.unwrap().is_ok());

    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem" }));
    assert!(matches!(
        dispatch(&fs, VDFS_DELETE, &ctx).await.unwrap(),
        Err(PluginError::Forbidden(_))
    ));

    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem" }));
    assert!(matches!(
        dispatch(&fs, VDFS_MKDIR, &ctx).await.unwrap(),
        Err(PluginError::ValidationError(_))
    ));
}

#[tokio::test]
async fn dir_root_is_not_readable() {
    let (fs, _) = fs_roots();
    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem" }));
    let err = dispatch(&fs, VDFS_READ, &ctx).await.unwrap().unwrap_err();
    assert!(matches!(err, PluginError::Forbidden(_)));
}

// 曾经这里有三个 `move` 用例（同类别转发 / 跨子目录拒绝 / 跨半拒绝）。移动整条
// 下线后它们没有对应的不变式可钉（守卫与操作一起没了），故一并删除——**不要**
// 因为「地址翻译」还想验而把它们改写成别的操作：那三例考的是 move 的地址解析，
// 与 list / stat 的翻译口径无关，后者另有用例。
//
// 但「跨子目录移动」那例顺带覆盖的**另一件事**被单独留了下来（下一个用例）：
// 一个只实现了一部分操作的 provider，其未实现的操作要以 `NotImplemented` 原样
// 穿出门面。那是「新 provider 只要实现一部分就能挂上去」的前提。

/// 「什么都不实现」的 provider：未实现的操作以 `NotImplemented` 原样穿出门面，
/// 不变成内部错误、不 panic。
#[tokio::test]
async fn unimplemented_ops_surface_as_not_implemented() {
    let fs: DynVdfsProvider = Arc::new(UnifiedFs::with_physical(
        Arc::new(TestRoot {
            dirs: vec![("bare", Arc::new(Bare))],
        }),
        Arc::new(PhysicalFs::new()),
    ));
    let err = fs
        .dispatch(&VdfsContext::empty(), ".vdfsv2/bare/x", VdfsRequest::Stat)
        .await
        .unwrap_err();
    assert!(matches!(err, VdfsError::NotImplemented), "实际：{err:?}");
}

#[tokio::test]
async fn watch_unwatch_forward_relative_path() {
    let (fs, rec) = fs_roots();
    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/sub" }));
    assert!(dispatch(&fs, ROUTE_VDFS_WATCH, &ctx).await.unwrap().is_ok());
    assert!(dispatch(&fs, ROUTE_VDFS_UNWATCH, &ctx)
        .await
        .unwrap()
        .is_ok());
    assert_eq!(rec.seen(), vec!["sub", "sub"], "provider 收到相对路径");
}

/// 树遍历从虚拟根均匀展开：类别本身也是树的一层
#[tokio::test]
async fn tree_walks_uniformly_from_root() {
    let (fs, _) = fs_roots();
    let ctx = ctx_with(json!({ "path": ".vdfsv2" }));
    let t = dispatch(&fs, VDFS_TREE, &ctx)
        .await
        .unwrap()
        .unwrap()
        .get::<VdfsTreeResponse>()
        .unwrap();
    let paths: Vec<&str> = t.nodes.iter().map(|n| n.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            ".vdfsv2/mem",
            ".vdfsv2/mem/a.txt",
            ".vdfsv2/mem/sub",
            ".vdfsv2/mem/sub/b.md"
        ]
    );
    assert!(!t.truncated);
}

#[tokio::test]
async fn tree_respects_depth_and_limit() {
    let (fs, _) = fs_roots();

    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem", "depth": 1 }));
    let t = dispatch(&fs, VDFS_TREE, &ctx)
        .await
        .unwrap()
        .unwrap()
        .get::<VdfsTreeResponse>()
        .unwrap();
    assert_eq!(
        t.nodes.iter().map(|n| n.path.as_str()).collect::<Vec<_>>(),
        vec![".vdfsv2/mem/a.txt", ".vdfsv2/mem/sub"],
        "depth=1 → 只到直接子节点"
    );

    let ctx = ctx_with(json!({ "path": ".vdfsv2", "limit": 2 }));
    let t = dispatch(&fs, VDFS_TREE, &ctx)
        .await
        .unwrap()
        .unwrap()
        .get::<VdfsTreeResponse>()
        .unwrap();
    assert!(t.truncated);
    assert_eq!(t.nodes.len(), 2);
}

/// 物理半经统一分发读写磁盘：WORKDIR 由调用级参数透传到物理层
#[tokio::test]
async fn physical_half_reads_and_writes_disk() {
    let dir = std::env::temp_dir().join(format!(
        "symbio-host-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let fs: DynVdfsProvider = Arc::new(UnifiedFs::new(empty_root()));
    let mk_ctx = |payload: Value| {
        let c: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
        c.set(WORKDIR, dir.to_string_lossy().into_owned());
        c.set_payload(payload).unwrap();
        c
    };

    let wctx = mk_ctx(json!({ "path": "hello.txt", "text": "hi" }));
    let w = dispatch_with(&fs, VDFS_WRITE, &wctx, call_params(&wctx))
        .await
        .unwrap()
        .unwrap()
        .get::<VdfsWriteResponse>()
        .unwrap();
    assert!(w.created);
    assert!(dir.join("hello.txt").exists());

    let rctx = mk_ctx(json!({ "path": "hello.txt" }));
    let c = dispatch_with(&fs, VDFS_READ, &rctx, call_params(&rctx))
        .await
        .unwrap()
        .unwrap()
        .get::<VdfsContent>()
        .unwrap();
    assert_eq!(c.text.as_deref(), Some("hi"));

    let _ = std::fs::remove_dir_all(&dir);
}

/// 变更事件：门面已把路径补成展示地址，本层**原样透传**（不再换信封）
#[test]
fn change_event_passes_display_paths_through() {
    // 物理半的地址原样保留
    let n = VdfsChange::bare("README.md");
    assert_eq!(n.path, "README.md");
    assert!(n.data.is_none());
}

// ==================== 根解析 ====================

/// 无能力管理器、无父插件 → 统一文件系统仍可用：虚拟层为空、物理层照常
#[tokio::test]
async fn resolve_fs_degrades_to_empty_vfs() {
    let fs = resolve_fs(None, &ctx_empty()).await;

    let ctx = ctx_with(json!({ "path": ".vdfsv2" }));
    let data = dispatch(&fs, VDFS_LIST, &ctx)
        .await
        .unwrap()
        .unwrap()
        .get::<VdfsListResponse>()
        .unwrap();
    assert_eq!(data.path, ".vdfsv2");
    assert!(data.items.is_empty(), "虚拟层降级为空目录而非报错");

    // 具体虚拟地址一律 NotFound
    let ctx = ctx_with(json!({ "path": ".vdfsv2/mem" }));
    assert!(matches!(
        dispatch(&fs, VDFS_LIST, &ctx).await.unwrap(),
        Err(PluginError::NotFound(_))
    ));

    // 物理半不受降级影响（缺 WORKDIR 是接线错误，不是 NotFound）
    let ctx = ctx_with(json!({ "path": "" }));
    assert!(matches!(
        dispatch(&fs, VDFS_LIST, &ctx).await.unwrap(),
        Err(PluginError::InternalError(_))
    ));
}

/// `ctx` 已带能力管理器 → 直接读其中的根（LLM 链路）
#[tokio::test]
async fn resolve_fs_prefers_visitor_slot() {
    let rec = Rec::new();
    let visitor: Arc<dyn CapabilityVisitor> = Arc::new(DefaultToolVisitor::new());
    visitor.register_vdfs_root(root_with(&rec)).await;

    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(CAPABILITY_VISITOR, visitor);

    let fs = resolve_fs(None, &ctx).await;
    let data = dispatch(&fs, VDFS_LIST, &ctx_with(json!({ "path": ".vdfsv2/mem" })))
        .await
        .unwrap()
        .unwrap()
        .get::<VdfsListResponse>()
        .unwrap();
    assert_eq!(data.node.name, "mem");
    assert_eq!(data.items.len(), 2);
}

/// 无能力管理器 → 经 `Plugin::get_vfs_provider` 直接取父插件的虚拟层根（前端链路）
#[tokio::test]
async fn resolve_fs_fetches_root_via_trait_method() {
    struct FakeContainer;

    #[async_trait]
    impl Plugin for FakeContainer {
        fn meta(&self) -> PluginMeta {
            PluginMeta::new("fake", "假容器")
        }

        async fn route(
            self: Arc<Self>,
            _ctx: Arc<dyn PluginInvokeRequest>,
        ) -> PluginInvokeResponse<PluginPayload> {
            Err(PluginError::NotFound("fake".into()))
        }

        async fn traverse(
            self: Arc<Self>,
            _path: String,
            _ctx: Arc<dyn PluginInvokeRequest>,
        ) -> PluginInvokeResponse<PluginPayload> {
            Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
        }

        /// 系统链路：直接暴露自己的虚拟层根（与 LLM 链路经 `register_vdfs_root` 收集互不干扰）
        fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn VdfsProvider>> {
            Some(root_with(&Rec::new()))
        }
    }

    let parent: Arc<dyn Plugin> = Arc::new(FakeContainer);
    let fs = resolve_fs(Some(&parent), &ctx_empty()).await;

    let items = dispatch(&fs, VDFS_LIST, &ctx_with(json!({ "path": ".vdfsv2" })))
        .await
        .unwrap()
        .unwrap()
        .get::<VdfsListResponse>()
        .unwrap()
        .items;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].node.name, "mem");
    assert_eq!(items[0].path, ".vdfsv2/mem");
}
