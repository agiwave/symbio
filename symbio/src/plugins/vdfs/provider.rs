//! VDFS 工具链路的封装 provider —— 把「对文件系统的操作」原样交给 [`UnifiedFs`]
//!
//! 大模型工具（`tools/` 下的 `vdfs_*`）不认识能力管理器、不认识虚拟层与物理层的
//! 区分，它们只持有本类型：
//!
//! ```text
//! vdfs_* 工具 ──▶ ToolVdfs（取根 + 透传 workdir）──▶ UnifiedFs ──▶ 虚拟层 / 物理层
//! ```
//!
//! ## 本层只做两件事
//!
//! 1. **取虚拟层根**：从持有的 `CapabilityVisitor` 里取容器注册的组合根
//!    （[`host::root_of`]），与前端链路取到的是**同一个根**；
//! 2. **透传调用级参数**：把请求 ctx 的运行时状态（workdir）翻译成 provider 的
//!    约定键（[`call_params`]），两条链路共用同一份翻译。
//!
//! 地址规则、两半分流、路径回填、根守卫、跨半移动拒绝**全部在 [`UnifiedFs`] 一处**，
//! 本文件不再重复实现——这也是它此前最需要的收敛：曾经在这里做的
//! 「裸地址补 `local/` 前缀 → 再拆挂载名 → 按名取 provider」三步翻译，
//! 现在只需要把地址原样交给门面。
//!
//! [`host::root_of`]: super::host::root_of

use super::fs::UnifiedFs;
use super::host::call_params;
use super::protocol::{VdfsEditResponse, VdfsSearchResult};
use crate::symbio_core::vdfs::vdfs_context;
use crate::symbio_core::vdfs_provider::*;
use crate::symbio_core::{CapabilityVisitor, InvokeRequest};
use std::sync::Arc;

/// VDFS 工具链路的封装 provider。
///
/// 工具构造时持有 `Arc<Self>`；执行时它把操作交给统一文件系统，工具因此不必
/// 感知能力管理器与目录拓扑。
pub struct ToolVdfs {
    visitor: Arc<dyn CapabilityVisitor>,
}

impl ToolVdfs {
    /// 持有能力管理器构造（工具注册广播的 ctx 中取到的即是它）
    pub fn new(visitor: Arc<dyn CapabilityVisitor>) -> Self {
        Self { visitor }
    }

    /// 本次调用的统一文件系统（虚拟层根来自容器注册，物理层是磁盘）
    async fn fs(&self, ctx: &Arc<dyn InvokeRequest>) -> (DynVdfsProvider, VdfsContext) {
        let root = super::host::root_of(&self.visitor).await;
        let fs: DynVdfsProvider = Arc::new(UnifiedFs::new(root));
        (fs, vdfs_context(ctx).with_params(call_params(ctx)))
    }

    /// 列出目录的直接子节点
    pub async fn list(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        path: &str,
    ) -> VdfsResult<Vec<VdfsNode>> {
        let (fs, vctx) = self.fs(ctx).await;
        fs.list(&vctx, path).await
    }

    /// 读取节点元数据
    pub async fn stat(&self, ctx: &Arc<dyn InvokeRequest>, path: &str) -> VdfsResult<VdfsNode> {
        let (fs, vctx) = self.fs(ctx).await;
        fs.stat(&vctx, path).await
    }

    /// 读取内容（`r` 位）
    pub async fn read(&self, ctx: &Arc<dyn InvokeRequest>, path: &str) -> VdfsResult<VdfsContent> {
        let (fs, vctx) = self.fs(ctx).await;
        fs.read(&vctx, path).await
    }

    /// 写入内容（`w` 位）
    pub async fn write(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let (fs, vctx) = self.fs(ctx).await;
        fs.write(&vctx, path, content).await
    }

    /// 删除节点
    pub async fn delete(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        path: &str,
        recursive: bool,
    ) -> VdfsResult<()> {
        let (fs, vctx) = self.fs(ctx).await;
        fs.delete(&vctx, path, recursive).await
    }

    /// 新建目录
    pub async fn mkdir(&self, ctx: &Arc<dyn InvokeRequest>, path: &str) -> VdfsResult<()> {
        let (fs, vctx) = self.fs(ctx).await;
        fs.mkdir(&vctx, path).await
    }

    /// 移动 / 重命名（同一半内；跨半由 [`UnifiedFs`] 拒绝）
    pub async fn move_item(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        from: &str,
        to: &str,
    ) -> VdfsResult<()> {
        let (fs, vctx) = self.fs(ctx).await;
        fs.move_item(&vctx, from, to).await
    }

    /// 内容编辑——**组合操作**：read → 精确替换 → write，
    /// 逻辑只在访问层一份（[`host::edit_via`]），任何一层只出原子操作
    pub async fn edit(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        path: &str,
        old_string: &str,
        new_string: &str,
    ) -> VdfsResult<VdfsEditResponse> {
        let (fs, vctx) = self.fs(ctx).await;
        super::host::edit_via(&fs, &vctx, path, old_string, new_string).await
    }

    /// 文件名 Glob 搜索（`path` 为可选搜索基目录）——**组合操作**：
    /// 递归 `list` + 模式过滤，逻辑只在访问层一份（[`host::search_via`]）
    pub async fn search(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        path: &str,
        pattern: &str,
    ) -> VdfsResult<VdfsSearchResult> {
        let (fs, vctx) = self.fs(ctx).await;
        super::host::search_via(&fs, &vctx, path, pattern).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::{DefaultToolVisitor, InvokeRequestExt, SimpleRequest, WORKDIR};
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
        async fn list(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
            self.note(path);
            Ok(vec![VdfsNode::file("a.txt", "a.txt", VdfsAccess::READ)])
        }

        async fn read(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
            self.note(path);
            Ok(VdfsContent::text("", "hello"))
        }

        async fn write(
            &self,
            _ctx: &VdfsContext,
            path: &str,
            _content: &VdfsContent,
        ) -> VdfsResult<VdfsWriteResponse> {
            self.note(path);
            Ok(VdfsWriteResponse {
                path: String::new(),
                created: true,
                etag: None,
            })
        }

        async fn delete(&self, _ctx: &VdfsContext, path: &str, _r: bool) -> VdfsResult<()> {
            self.note(path);
            Ok(())
        }

        async fn mkdir(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
            self.note(path);
            Ok(())
        }

        async fn move_item(&self, _ctx: &VdfsContext, from: &str, to: &str) -> VdfsResult<()> {
            self.note(&format!("{from}→{to}"));
            Ok(())
        }
    }

    /// 把替身登记为 **`.vdfs` 的服务者**（工具链路取根的唯一途径），
    /// 收到的路径一律是树内相对路径（`""`、`<子目录>/…`）。
    async fn tool_with_root(root: Arc<Rec>) -> ToolVdfs {
        let visitor: Arc<dyn CapabilityVisitor> = Arc::new(DefaultToolVisitor::new());
        visitor.register_vdfs_root(root).await;
        ToolVdfs::new(visitor)
    }

    fn ctx() -> Arc<dyn InvokeRequest> {
        Arc::new(SimpleRequest::new(None, None))
    }

    /// 虚拟地址：`.vdfs/<子目录>/…` 进虚拟层时剥成树内相对路径 `<子目录>/…`
    #[tokio::test]
    async fn virtual_address_is_routed_to_root() {
        let rec = Rec::new();
        let vdfs = tool_with_root(rec.clone()).await;
        vdfs.read(&ctx(), ".vdfs/setting/appearance").await.unwrap();
        assert_eq!(rec.seen(), vec!["setting/appearance"]);
    }

    /// `.vdfs` 本体 = 根目录：列目录进树内口径就是空串
    #[tokio::test]
    async fn virtual_root_lists_through_the_same_root() {
        let rec = Rec::new();
        let vdfs = tool_with_root(rec.clone()).await;
        vdfs.list(&ctx(), ".vdfs").await.unwrap();
        assert_eq!(rec.seen(), vec![""]);
        // `.vdfs/` 与 `.vdfs` 等价
        vdfs.list(&ctx(), ".vdfs/").await.unwrap();
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
        let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        ctx.set(WORKDIR, dir.to_string_lossy().into_owned());

        let c = vdfs.read(&ctx, "hello.txt").await.unwrap();
        assert_eq!(c.text.as_deref(), Some("hi"));
        assert!(rec.seen().is_empty(), "物理地址不该出现在虚拟层");

        // 目录根：可列、不可读
        let items = vdfs.list(&ctx, "/").await.unwrap();
        assert_eq!(items[0].name, "hello.txt");
        assert!(matches!(
            vdfs.read(&ctx, "/").await.unwrap_err(),
            VdfsError::Forbidden(_)
        ));

        // 写入 / 建目录 / 删除走同一条物理通道
        vdfs.write(&ctx, "sub/new.txt", &VdfsContent::text("", "x"))
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

    /// 两半之间不可移动：物理 → 虚拟在触达任何一层之前就被拒
    #[tokio::test]
    async fn move_across_halves_is_rejected_before_dispatch() {
        let rec = Rec::new();
        let vdfs = tool_with_root(rec.clone()).await;
        let err = vdfs
            .move_item(&ctx(), "a.txt", ".vdfs/other/b.txt")
            .await
            .unwrap_err();
        assert!(matches!(err, VdfsError::Invalid(_)), "应为 {err:?}");
        assert!(rec.seen().is_empty(), "判定应先于触达");
    }

    /// 同一半内移动：地址原样送到虚拟层
    #[tokio::test]
    async fn move_within_virtual_is_forwarded() {
        let rec = Rec::new();
        let vdfs = tool_with_root(rec.clone()).await;
        vdfs.move_item(&ctx(), ".vdfs/x/a", ".vdfs/x/b")
            .await
            .unwrap();
        assert_eq!(rec.seen(), vec!["x/a→x/b"]);
    }

    /// `..` 穿越在分流阶段就失败，两条链路共享同一条守卫
    #[tokio::test]
    async fn traversal_is_rejected_on_both_links() {
        let rec = Rec::new();
        let vdfs = tool_with_root(rec.clone()).await;
        for bad in ["../../etc/passwd", ".vdfs/../../x"] {
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
            .edit(&ctx(), ".vdfs/a.txt", "hello", "world")
            .await
            .unwrap();
        assert_eq!(r.replaced, 1);
        assert_eq!(rec.seen(), vec!["a.txt", "a.txt"], "先 read 后 write");

        let err = vdfs
            .edit(&ctx(), ".vdfs/a.txt", "nope", "x")
            .await
            .unwrap_err();
        assert!(matches!(err, VdfsError::Invalid(_)));
    }

    /// search = 递归 list + glob 过滤的组合（走 `list`，各层的安全规则随之生效）
    #[tokio::test]
    async fn search_composes_list_walk() {
        let rec = Rec::new();
        let vdfs = tool_with_root(rec.clone()).await;
        let r = vdfs.search(&ctx(), ".vdfs/x", "*.txt").await.unwrap();
        assert_eq!(r.results, vec![".vdfs/x/a.txt"], "结果与请求地址同坐标系");
        assert!(!r.truncated);
        assert_eq!(rec.seen(), vec!["x"], "搜索走虚拟层的 list");
    }

    /// 无容器注册根时降级为空文件系统（而非报错），物理层照常可用
    #[tokio::test]
    async fn missing_root_degrades_to_empty_virtual_layer() {
        let visitor: Arc<dyn CapabilityVisitor> = Arc::new(DefaultToolVisitor::new());
        let vdfs = ToolVdfs::new(visitor);
        let items = vdfs.list(&ctx(), ".vdfs").await.unwrap();
        assert!(items.is_empty(), "无容器 = 无资源类别，但仍是一棵合法的树");
    }
}
