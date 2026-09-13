//! VDFS 工具链路的封装 provider —— 持有能力管理器，直调其中注册的挂载 provider
//!
//! 大模型工具（`tools/` 下的 `vdfs_*`）不认识能力管理器、不认识挂载点，它们只持有
//! 本类型并把「对文件系统的操作」原样改为「对本类型的操作」：
//!
//! ```text
//! vdfs_* 工具 ──▶ ToolVdfs（翻译地址 + 选 provider + 透传 workdir）──▶ 挂载 provider
//! ```
//!
//! 本类型做三件事，仅此三件：
//!
//! 1. **地址翻译**（`docs/design/vdfs.md` 的 LLM 地址规则）——本地文件地址
//!    （无 `.vdfs/` 前缀）拼接 `local/` 前缀；虚拟地址（`.vdfs/<挂载名>/…`）剥掉前缀。
//!    随后 [`normalize_path`] 规范化并拒绝 `..` 穿越，[`split_mount`] 拆出
//!    「挂载名 + 子树相对路径」；
//! 2. **按挂载名直调**：从持有的 `CapabilityVisitor` 里 [`CapabilityVisitor::get_vdfs_provider`]
//!    取对应 provider，调用其方法——**不经 composite 的根，也不经协议信封**；
//! 3. **调用级参数与守卫**：把请求 ctx 的 `WORKDIR` 透传为 provider 参数
//!    （[`call_params`]）；挂载根本体不可读 / 写 / 删 / 建 / 移（与根 `VdfsMountTable`
//!    的守卫语义一致），跨挂载点移动拒绝。

use super::host::call_params;
use super::protocol::{VdfsEditResponse, VdfsSearchResult};
use crate::symbio_core::vdfs::vdfs_context;
use crate::symbio_core::vdfs_provider::*;
use crate::symbio_core::{CapabilityVisitor, InvokeRequest, PLUGIN_LOCAL};
use std::sync::Arc;

/// 本地工具链路的**默认挂载点**：所有非虚拟地址按本地文件规则解析，挂到它之下。
///
/// 用插件名（宿主内唯一）而非另起名字——`local` 插件的注册名就是 [`PLUGIN_LOCAL`]，
/// 两边共用同一常量，不会漂移。
pub const LOCAL_MOUNT: &str = PLUGIN_LOCAL;

/// 约定虚拟地址前缀：大模型用它表达「按挂载名路由到具体 provider」。
///
/// 以本前缀开头的地址剥掉前缀后按首段挂载名分发；其余地址视为本地文件地址，
/// 统一挂到 [`LOCAL_MOUNT`] 下。
pub const VIRTUAL_PREFIX: &str = ".vdfs/";

/// VDFS 工具链路的封装 provider。
///
/// 工具构造时持有 `Arc<Self>`；执行时它从能力管理器里按挂载名取 provider 直调，
/// 工具因此不必感知能力管理器与挂载拓扑。
pub struct ToolVdfs {
    visitor: Arc<dyn CapabilityVisitor>,
}

impl ToolVdfs {
    /// 持有能力管理器构造（工具注册广播的 ctx 中取到的即是它）
    pub fn new(visitor: Arc<dyn CapabilityVisitor>) -> Self {
        Self { visitor }
    }

    /// 把大模型给出的地址翻译成「(挂载 provider, 子树相对路径)」。
    ///
    /// 仅拼接 / 剥离而非解析：本地地址挂 [`LOCAL_MOUNT`]（`README.md` →
    /// `local/README.md`、`/` → `local//`），虚拟地址剥 [`VIRTUAL_PREFIX`]
    /// （`.vdfs/setting/appearance` → `setting/appearance`）。规范化后首段即挂载名。
    async fn target(&self, raw: &str) -> VdfsResult<(Arc<dyn VdfsProvider>, String)> {
        let scoped = match raw.strip_prefix(VIRTUAL_PREFIX) {
            Some(rest) => rest.to_string(),
            None => format!("{LOCAL_MOUNT}/{raw}"),
        };
        let full = normalize_path(&scoped)?;
        let (mount, rel) =
            split_mount(&full).ok_or_else(|| VdfsError::invalid(format!("无效地址：{raw}")))?;
        let provider = self.visitor.get_vdfs_provider(mount).await.ok_or_else(|| {
            VdfsError::not_found(format!(
                "未知挂载点 '{mount}'（可用 vdfs_mounts 查看全部挂载点）"
            ))
        })?;
        Ok((provider, rel.to_string()))
    }

    /// 调用级参数：把请求 ctx 的运行时状态（workdir）翻译成 provider 的约定键。
    ///
    /// 与前端链路共用同一份翻译（[`call_params`]），provider 不认识宿主 ctx 键名。
    fn vctx(&self, ctx: &Arc<dyn InvokeRequest>) -> VdfsContext {
        vdfs_context(ctx).with_params(call_params(ctx))
    }

    /// 列出目录的直接子节点（挂载根 = 工作目录根，允许）
    pub async fn list(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        path: &str,
    ) -> VdfsResult<Vec<VdfsNode>> {
        let (provider, rel) = self.target(path).await?;
        provider.list(&self.vctx(ctx), &rel).await
    }

    /// 读取节点元数据
    pub async fn stat(&self, ctx: &Arc<dyn InvokeRequest>, path: &str) -> VdfsResult<VdfsNode> {
        let (provider, rel) = self.target(path).await?;
        provider.stat(&self.vctx(ctx), &rel).await
    }

    /// 读取内容（`r` 位；挂载根不可读）
    pub async fn read(&self, ctx: &Arc<dyn InvokeRequest>, path: &str) -> VdfsResult<VdfsContent> {
        let (provider, rel) = self.target(path).await?;
        guard_not_root(&rel, "读取内容")?;
        provider.read(&self.vctx(ctx), &rel).await
    }

    /// 写入内容（`w` 位；挂载根不可写）
    pub async fn write(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let (provider, rel) = self.target(path).await?;
        guard_not_root(&rel, "写入")?;
        provider.write(&self.vctx(ctx), &rel, content).await
    }

    /// 删除节点（挂载根不可删）
    pub async fn delete(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        path: &str,
        recursive: bool,
    ) -> VdfsResult<()> {
        let (provider, rel) = self.target(path).await?;
        guard_not_root(&rel, "删除")?;
        provider.delete(&self.vctx(ctx), &rel, recursive).await
    }

    /// 新建目录（挂载根本身不可建）
    pub async fn mkdir(&self, ctx: &Arc<dyn InvokeRequest>, path: &str) -> VdfsResult<()> {
        let (provider, rel) = self.target(path).await?;
        guard_not_root(&rel, "创建目录")?;
        provider.mkdir(&self.vctx(ctx), &rel).await
    }

    /// 移动 / 重命名（同挂载点内；跨挂载点拒绝，挂载根不可移动）
    pub async fn move_item(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        from: &str,
        to: &str,
    ) -> VdfsResult<()> {
        let (src, from_rel) = self.target(from).await?;
        guard_not_root(&from_rel, "移动")?;
        let (dst, to_rel) = self.target(to).await?;
        guard_not_root(&to_rel, "移动")?;
        if !Arc::ptr_eq(&src, &dst) {
            return Err(VdfsError::invalid("不允许跨挂载点移动"));
        }
        let vctx = self.vctx(ctx);
        src.move_item(&vctx, &from_rel, &to_rel).await
    }

    /// 内容编辑（挂载根不可编辑）——**组合操作**：read → 精确替换 → write，
    /// 逻辑只在访问层一份（[`host::edit_via`]），provider 只出原子操作
    pub async fn edit(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        path: &str,
        old_string: &str,
        new_string: &str,
    ) -> VdfsResult<VdfsEditResponse> {
        let (provider, rel) = self.target(path).await?;
        guard_not_root(&rel, "编辑")?;
        super::host::edit_via(&provider, &self.vctx(ctx), &rel, old_string, new_string).await
    }

    /// 文件名 Glob 搜索（`path` 为可选搜索基目录，缺省 = 挂载根）——**组合操作**：
    /// 递归 `list` + 模式过滤，逻辑只在访问层一份（[`host::search_via`]）
    pub async fn search(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        path: &str,
        pattern: &str,
    ) -> VdfsResult<VdfsSearchResult> {
        let (provider, rel) = self.target(path).await?;
        super::host::search_via(&provider, &self.vctx(ctx), &rel, pattern).await
    }

    /// 全部挂载点：`(挂载名, provider)`，已按 provider `order` 升序稳定排序
    /// （[`CapabilityVisitor::list_vdfs_providers`] 的语义）
    pub async fn mounts(&self) -> Vec<(String, Arc<dyn VdfsProvider>)> {
        self.visitor.list_vdfs_providers().await
    }
}

/// 挂载根本体守卫：不可读 / 写 / 删 / 建 / 移（列目录与元数据不受限）。
///
/// 与根 `VdfsMountTable` 的守卫语义一致——绕过根直调 provider 后仍需保留它，
/// 否则 `delete("/", recursive=true)` 会删掉整个工作目录。
fn guard_not_root(rel: &str, op: &str) -> VdfsResult<()> {
    if rel.is_empty() {
        return Err(VdfsError::Forbidden(format!(
            "挂载根不支持{op}，请给出具体路径"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::{DefaultToolVisitor, SimpleRequest};
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// 记录收到路径的内存 provider（各操作全部直通，便于断言翻译结果）
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

    async fn visitor_with(mounts: Vec<(&str, Arc<Rec>)>) -> Arc<dyn CapabilityVisitor> {
        let v: Arc<dyn CapabilityVisitor> = Arc::new(DefaultToolVisitor::new());
        for (mount, provider) in mounts {
            v.register_vdfs_provider(mount, provider).await;
        }
        v
    }

    fn ctx() -> Arc<dyn InvokeRequest> {
        Arc::new(SimpleRequest::new(None, None))
    }

    /// 本地地址规则：无前缀地址挂到 `local` provider，相对路径原样到达
    #[tokio::test]
    async fn local_address_routes_to_local_provider() {
        let rec = Rec::new();
        let vdfs = ToolVdfs::new(visitor_with(vec![("local", rec.clone())]).await);
        vdfs.read(&ctx(), "README.md").await.unwrap();
        assert_eq!(rec.seen(), vec!["README.md"]);
    }

    /// 虚拟地址（`.vdfs/<挂载名>/…`）剥前缀后按挂载名分发
    #[tokio::test]
    async fn virtual_address_routes_to_named_mount() {
        let rec = Rec::new();
        let vdfs = ToolVdfs::new(visitor_with(vec![("setting", rec.clone())]).await);
        vdfs.read(&ctx(), ".vdfs/setting/appearance").await.unwrap();
        assert_eq!(rec.seen(), vec!["appearance"]);
    }

    /// `/` 即本地挂载根：可列出（provider 收到空相对路径），不可读内容
    #[tokio::test]
    async fn root_path_lists_but_never_reads() {
        let rec = Rec::new();
        let vdfs = ToolVdfs::new(visitor_with(vec![("local", rec.clone())]).await);
        vdfs.list(&ctx(), "/").await.unwrap();
        assert_eq!(rec.seen(), vec![""], "list(/) → provider 收到挂载根");

        let err = vdfs.read(&ctx(), "/").await.unwrap_err();
        assert!(matches!(err, VdfsError::Forbidden(_)), "挂载根不可读");
    }

    /// 挂载根本体不可写 / 删 / 建 / 移
    #[tokio::test]
    async fn mount_root_is_guarded() {
        let rec = Rec::new();
        let vdfs = ToolVdfs::new(visitor_with(vec![("local", rec.clone())]).await);

        assert!(vdfs
            .write(&ctx(), "/", &VdfsContent::text("", "x"))
            .await
            .is_err());
        assert!(vdfs.delete(&ctx(), "/", true).await.is_err());
        assert!(vdfs.mkdir(&ctx(), "/").await.is_err());
        assert!(vdfs.move_item(&ctx(), "/", "a.txt").await.is_err());
        assert!(rec.seen().is_empty(), "守卫在触达 provider 之前生效");
    }

    /// 未知挂载点 → NotFound（带挂载名提示）
    #[tokio::test]
    async fn unknown_mount_is_not_found() {
        let vdfs = ToolVdfs::new(visitor_with(vec![]).await);
        let err = vdfs.read(&ctx(), "README.md").await.unwrap_err();
        assert!(matches!(err, VdfsError::NotFound(_)));
        assert!(err.to_string().contains("local"));
    }

    /// 同挂载点内移动直通；跨挂载点拒绝
    #[tokio::test]
    async fn move_within_mount_forwarded_across_mounts_rejected() {
        let local = Rec::new();
        let other = Rec::new();
        let vdfs = ToolVdfs::new(
            visitor_with(vec![("local", local.clone()), ("other", other.clone())]).await,
        );

        vdfs.move_item(&ctx(), "a.txt", "b.txt").await.unwrap();
        assert_eq!(local.seen(), vec!["a.txt→b.txt"]);

        let err = vdfs
            .move_item(&ctx(), "a.txt", ".vdfs/other/b.txt")
            .await
            .unwrap_err();
        assert!(matches!(err, VdfsError::Invalid(_)), "跨挂载点移动拒绝");
        assert!(local.seen().len() == 1 && other.seen().is_empty());
    }

    /// edit = read → 精确替换 → write 的组合（provider 只出原子操作）
    #[tokio::test]
    async fn edit_composes_read_and_write() {
        let rec = Rec::new();
        let vdfs = ToolVdfs::new(visitor_with(vec![("local", rec.clone())]).await);
        let r = vdfs.edit(&ctx(), "a.txt", "hello", "world").await.unwrap();
        assert_eq!(r.replaced, 1);
        assert_eq!(rec.seen(), vec!["a.txt", "a.txt"], "先 read 后 write");

        // 匹配 0 次报错（不触达 write）
        let err = vdfs.edit(&ctx(), "a.txt", "nope", "x").await.unwrap_err();
        assert!(matches!(err, VdfsError::Invalid(_)));
    }

    /// search = 递归 list + glob 过滤的组合（走 provider 的 list，安全规则随之生效）
    #[tokio::test]
    async fn search_composes_list_walk() {
        let rec = Rec::new();
        let vdfs = ToolVdfs::new(visitor_with(vec![("local", rec.clone())]).await);
        let r = vdfs.search(&ctx(), "", "*.txt").await.unwrap();
        assert_eq!(r.results, vec!["a.txt"]);
        assert!(!r.truncated);
        assert_eq!(rec.seen(), vec![""], "搜索走 provider 的 list");
    }
}
