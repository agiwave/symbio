//! `symbio/src/plugins/vdfs/provider.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use crate::providers::DefaultToolVisitor;
use crate::symbio_core::{PluginInvokeRequestExt, PluginSimpleRequest, WORKDIR};
use crate::symbio_core::{VdfsAccess, VdfsContext, VdfsProvider, VdfsResponse};
use async_trait::async_trait;
use std::sync::Mutex;

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
                    etag: None,
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
    let visitor: Arc<dyn CapabilityVisitor> = Arc::new(DefaultToolVisitor::new());
    visitor.register_vdfs_root(root).await;
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

    // 目录根：可列、不可读
    let items = vdfs.list(&ctx, "/").await.unwrap();
    assert_eq!(items[0].node.name, "hello.txt");
    assert!(matches!(
        vdfs.read(&ctx, "/").await.unwrap_err(),
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

// 曾经这里有两例 `move` 用例（跨半被拒 / 同半转发）。移动整条下线后
// `ToolVdfs` 不再有 `move_item`，跨半也不再可能（载荷里没有第二个地址）。

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
    let visitor: Arc<dyn CapabilityVisitor> = Arc::new(DefaultToolVisitor::new());
    let vdfs = ToolVdfs::new(visitor);
    let items = vdfs.list(&ctx(), ".vdfsv2").await.unwrap();
    assert!(items.is_empty(), "无容器 = 无资源类别，但仍是一棵合法的树");
}
