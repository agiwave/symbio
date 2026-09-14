//! 统一地址空间门面 —— 一棵树的两半按前缀分流
//!
//! ## 唯一的地址规则
//!
//! ```text
//! .vdfs            根目录（系统资源清单）           ─┐
//! .vdfs/<子目录>/…  某插件子树                       ├─▶ 虚拟层（容器注册的根）
//!                                                   ─┘
//! README.md        工作目录相对地址                 ─┐
//! src/main.rs      同上                              ├─▶ 物理层（磁盘真实文件）
//! D:/tmp/a.txt     绝对路径                          ─┘
//! ```
//!
//! **判别只有一条**：规范化后的地址是否以 `.vdfs` 打头。是 → 虚拟；否 → 物理。
//! 前端协议、LLM 工具两条链路共用这一个 [`UnifiedFs`]，因此翻译只存在于本文件一处，
//! 页面与工具都不需要知道展示口径与树内口径的差异。
//!
//! ## 为什么需要「口径」转换
//!
//! 虚拟层内部（容器注册的根及其下各 provider）是通用机制，provider 只认
//! **自身子树内的相对路径**（core 不该知道宿主把根目录叫做 `.vdfs`）。所以本门面
//! 在进出两处各做一次轻量映射：
//!
//! | 方向 | 映射 |
//! |---|---|
//! | 请求进入虚拟层 | `.vdfs` → `""`；`.vdfs/session/x` → `session/x` |
//! | 结果回到消费者 | `session/x` → `.vdfs/session/x` |
//!
//! 物理层不需要映射：它的地址本来就是 `.vdfs` 之外的样子。
//!
//! ## 本层不认识目录清单
//!
//! 根目录下有多少子目录、叫什么，由容器与注册决定；本文件只做前缀分流，
//! 新增一类资源不需要改动这里任何一行。

use super::physical::PhysicalFs;
use crate::symbio_core::vdfs_provider::*;
use async_trait::async_trait;
use std::sync::Arc;

/// 虚拟根目录的字面名字：系统资源统一在此目录之下。
///
/// 前端与 LLM 两条链路同用此名（前端 `schemas/vdfs.ts` 的 `VFDS_ROOT` 与本常量同源）。
pub const VFDS_ADDR_ROOT: &str = ".vdfs";

/// 规范化后的地址落在哪一半
#[derive(Debug, Clone, PartialEq, Eq)]
enum Half {
    /// 虚拟层：树内相对路径（`""` = 根目录）
    Virtual(String),
    /// 物理层：工作目录相对地址（`""` = 工作目录根）
    Physical(String),
}

/// 规范化地址：折叠空段与 `.`、拒绝 `..` 穿越、去掉首部分隔符。
///
/// **不强加前导 `/`**——虚拟地址以 `.vdfs` 打头、物理相对地址以名字打头、
/// 绝对物理地址以盘符打头，三者形态各异，统一成「以 `/` 分隔的段序列」即可。
///
/// - `""` / `"."` / `"/"` → `""`（工作目录根）
/// - `".vdfs/session/"` → `".vdfs/session"`
/// - `"/src/main.rs"` → `"src/main.rs"`（首段分隔符只是分隔，不是文件系统根）
/// - 含 `..` 段 → [`VdfsError::Invalid`]
pub fn normalize_addr(raw: &str) -> VdfsResult<String> {
    let mut segs: Vec<&str> = Vec::new();
    for seg in raw.trim().split('/') {
        let seg = seg.trim();
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return Err(VdfsError::invalid(format!(
                "VDFS 地址不允许向上穿越：{raw}"
            )));
        }
        segs.push(seg);
    }
    Ok(segs.join("/"))
}

/// 规范化地址 → 那一半
///
/// `.vdfs` 必须**独占首段**：`.vdfsfoo` 不算虚拟地址（按物理地址处理）。
fn half_of(addr: &str) -> Half {
    match addr.strip_prefix(VFDS_ADDR_ROOT) {
        Some("") => Half::Virtual(String::new()),
        Some(rest) if rest.starts_with('/') => {
            // 虚拟层树内口径：剥掉 `.vdfs/` 前缀，剩余部分原样透传
            Half::Virtual(rest[1..].to_string())
        }
        _ => Half::Physical(addr.to_string()),
    }
}

/// 虚拟层树内口径 → 对外展示地址（`session/x` → `.vdfs/session/x`）
pub fn to_display(root_rel: &str) -> String {
    if root_rel.is_empty() {
        return VFDS_ADDR_ROOT.to_string();
    }
    format!("{VFDS_ADDR_ROOT}/{root_rel}")
}

/// 解析地址并路由（所有操作的共同第一步）
fn route(raw: &str) -> VdfsResult<Half> {
    Ok(half_of(&normalize_addr(raw)?))
}

/// 统一文件系统：虚拟根 + 物理磁盘，对外是一张脸
pub struct UnifiedFs {
    /// 虚拟层根（容器注册的 root 级 provider，树内相对路径）
    virtual_root: DynVdfsProvider,
    /// 物理层（磁盘真实文件）
    physical: DynVdfsProvider,
}

impl UnifiedFs {
    /// 以容器注册的虚拟根构造，物理层用默认的 [`PhysicalFs`]
    pub fn new(virtual_root: DynVdfsProvider) -> Self {
        Self::with_physical(virtual_root, Arc::new(PhysicalFs::new()))
    }

    /// 显式注入物理层（测试可换成内存实现）
    pub fn with_physical(virtual_root: DynVdfsProvider, physical: DynVdfsProvider) -> Self {
        Self {
            virtual_root,
            physical,
        }
    }

    /// 树内口径 → 展示口径（`session/x` → `.vdfs/session/x`）。
    ///
    /// **只改非空 `path`**：空 `path` 是「provider 未填」的信号，由访问层
    /// （`host::fill_paths`）按请求地址回填成 `<父地址>/<子名>`；若在这里把它
    /// 补成 `.vdfs`，信号即被破坏，子节点会被错认成根。本函数与 `stat` /
    /// `read` / `write` 三处的回填口径因此完全一致：**只翻译已填的，不代填。**
    fn retag(&self, nodes: &mut [VdfsNode]) {
        for n in nodes.iter_mut() {
            if !n.path.is_empty() {
                n.path = to_display(&n.path);
            }
        }
    }
}

#[async_trait]
impl VdfsProvider for UnifiedFs {
    fn label(&self) -> Option<&str> {
        Some("文件系统")
    }

    fn description(&self) -> Option<&str> {
        Some(".vdfs 下是系统资源；其余地址是磁盘上的真实文件")
    }

    fn root_access(&self) -> VdfsAccess {
        VdfsAccess::dir(true, true)
    }

    async fn list(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        match route(path)? {
            Half::Virtual(v) => {
                let mut items = self.virtual_root.list(ctx, &v).await?;
                self.retag(&mut items);
                Ok(items)
            }
            Half::Physical(p) => self.physical.list(ctx, &p).await,
        }
    }

    async fn stat(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        match route(path)? {
            Half::Virtual(v) => {
                let mut n = self.virtual_root.stat(ctx, &v).await?;
                if !n.path.is_empty() {
                    n.path = to_display(&n.path);
                }
                Ok(n)
            }
            Half::Physical(p) => self.physical.stat(ctx, &p).await,
        }
    }

    async fn read(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        match route(path)? {
            Half::Virtual(v) => {
                let mut c = self.virtual_root.read(ctx, &v).await?;
                if !c.path.is_empty() {
                    c.path = to_display(&c.path);
                }
                Ok(c)
            }
            Half::Physical(p) => self.physical.read(ctx, &p).await,
        }
    }

    async fn write(
        &self,
        ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        match route(path)? {
            Half::Virtual(v) => {
                let mut r = self.virtual_root.write(ctx, &v, content).await?;
                if !r.path.is_empty() {
                    r.path = to_display(&r.path);
                }
                Ok(r)
            }
            Half::Physical(p) => self.physical.write(ctx, &p, content).await,
        }
    }

    async fn delete(&self, ctx: &VdfsContext, path: &str, recursive: bool) -> VdfsResult<()> {
        match route(path)? {
            Half::Virtual(v) => self.virtual_root.delete(ctx, &v, recursive).await,
            Half::Physical(p) => self.physical.delete(ctx, &p, recursive).await,
        }
    }

    async fn mkdir(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        match route(path)? {
            Half::Virtual(v) => self.virtual_root.mkdir(ctx, &v).await,
            Half::Physical(p) => self.physical.mkdir(ctx, &p).await,
        }
    }

    /// 同一半内可移动；虚拟 ↔ 物理之间不允许（两者不是同一个存储）
    async fn move_item(&self, ctx: &VdfsContext, from: &str, to: &str) -> VdfsResult<()> {
        match (route(from)?, route(to)?) {
            (Half::Virtual(f), Half::Virtual(t)) => {
                self.virtual_root.move_item(ctx, &f, &t).await
            }
            (Half::Physical(f), Half::Physical(t)) => self.physical.move_item(ctx, &f, &t).await,
            _ => Err(VdfsError::invalid(format!(
                "不允许在系统资源与磁盘文件之间移动：{from} → {to}"
            ))),
        }
    }

    async fn action(
        &self,
        ctx: &VdfsContext,
        path: &str,
        action: &str,
        payload: Option<&serde_json::Value>,
    ) -> VdfsResult<VdfsActionResult> {
        match route(path)? {
            Half::Virtual(v) => self.virtual_root.action(ctx, &v, action, payload).await,
            Half::Physical(p) => self.physical.action(ctx, &p, action, payload).await,
        }
    }

    /// 只有虚拟层的资源会自发变更；物理层的订阅按 no-op 处理（trait 缺省）。
    ///
    /// 事件里的路径补成对外展示地址，消费者拿到的坐标系与请求时一致。
    /// 用 [`VdfsChange::map_paths`] 一次覆盖全部路径（含 `node` 载荷内的路径），
    /// 不逐字段重建——新增字段时不会漏转发。
    async fn watch(&self, ctx: &VdfsContext, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        match route(path)? {
            Half::Virtual(v) => {
                let wrapped: VdfsChangeSink =
                    Arc::new(move |c: VdfsChange| sink(c.map_paths(to_display)));
                self.virtual_root.watch(ctx, &v, wrapped).await
            }
            Half::Physical(p) => self.physical.watch(ctx, &p, sink).await,
        }
    }

    async fn unwatch(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        match route(path)? {
            Half::Virtual(v) => self.virtual_root.unwatch(ctx, &v).await,
            Half::Physical(p) => self.physical.unwatch(ctx, &p).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::vdfs_provider::VdfsContext;

    /// 记录收到路径与形状的虚拟层替身
    struct V {
        seen: std::sync::Mutex<Vec<String>>,
    }

    impl V {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                seen: std::sync::Mutex::new(Vec::new()),
            })
        }
        fn seen(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl VdfsProvider for V {
        async fn list(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
            self.seen.lock().unwrap().push(path.to_string());
            let mut n = VdfsNode::dir("session", "会话", VdfsAccess::dir(true, true));
            n.path = "session".to_string();
            Ok(vec![n])
        }
        async fn read(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
            self.seen.lock().unwrap().push(path.to_string());
            Ok(VdfsContent::text(path, "v"))
        }
        async fn write(
            &self,
            _ctx: &VdfsContext,
            path: &str,
            _c: &VdfsContent,
        ) -> VdfsResult<VdfsWriteResponse> {
            self.seen.lock().unwrap().push(path.to_string());
            Ok(VdfsWriteResponse {
                path: path.to_string(),
                created: true,
                etag: None,
            })
        }
    }

    /// 物理层替身：只记录收到的地址
    struct P {
        seen: std::sync::Mutex<Vec<String>>,
    }

    impl P {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                seen: std::sync::Mutex::new(Vec::new()),
            })
        }
        fn seen(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl VdfsProvider for P {
        async fn list(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
            self.seen.lock().unwrap().push(path.to_string());
            Ok(vec![VdfsNode::file("a.txt", "a.txt", VdfsAccess::READ)])
        }
        async fn read(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
            self.seen.lock().unwrap().push(path.to_string());
            Ok(VdfsContent::text(path, "p"))
        }
    }

    fn fs() -> (UnifiedFs, Arc<V>, Arc<P>) {
        let (v, p) = (V::new(), P::new());
        (
            UnifiedFs::with_physical(v.clone(), p.clone()),
            v,
            p,
        )
    }

    // ==================== 地址代数 ====================

    #[test]
    fn addresses_normalize_without_forcing_root_slash() {
        assert_eq!(normalize_addr("").unwrap(), "");
        assert_eq!(normalize_addr(".").unwrap(), "");
        assert_eq!(normalize_addr("/").unwrap(), "");
        assert_eq!(normalize_addr("a//b/").unwrap(), "a/b");
        assert_eq!(normalize_addr("./a").unwrap(), "a");
        assert_eq!(normalize_addr(".vdfs/session/").unwrap(), ".vdfs/session");
        assert_eq!(normalize_addr("/src/main.rs").unwrap(), "src/main.rs");
        assert_eq!(normalize_addr("D:/tmp/a.txt").unwrap(), "D:/tmp/a.txt");
    }

    #[test]
    fn traversal_is_rejected() {
        for bad in ["../x", "a/../b", ".vdfs/../../etc"] {
            assert!(
                normalize_addr(bad).is_err(),
                "应拒绝向上穿越：{bad}"
            );
        }
    }

    #[test]
    fn display_paths_are_virtual_prefixed() {
        assert_eq!(to_display(""), VFDS_ADDR_ROOT);
        assert_eq!(to_display("session"), ".vdfs/session");
        assert_eq!(to_display("session/abc"), ".vdfs/session/abc");
    }

    #[test]
    fn only_exact_first_segment_is_virtual() {
        assert_eq!(half_of(".vdfs"), Half::Virtual(String::new()));
        assert_eq!(half_of(".vdfs/x"), Half::Virtual("x".to_string()));
        assert_eq!(
            half_of(".vdfs/session/abc"),
            Half::Virtual("session/abc".to_string())
        );
        // `.vdfsfoo` 不是虚拟地址；物理地址原样透传
        assert_eq!(half_of(".vdfsfoo"), Half::Physical(".vdfsfoo".to_string()));
        assert_eq!(half_of(""), Half::Physical(String::new()));
        assert_eq!(
            half_of("README.md"),
            Half::Physical("README.md".to_string())
        );
    }

    // ==================== 分流 ====================

    #[tokio::test]
    async fn virtual_address_is_translated_both_ways() {
        let (f, v, p) = fs();
        let ctx = VdfsContext::empty();

        let items = f.list(&ctx, ".vdfs").await.unwrap();
        assert_eq!(v.seen(), vec![""], "根目录进树内口径是空串");
        assert_eq!(items[0].path, ".vdfs/session", "结果回到展示口径");
        assert!(p.seen().is_empty(), "不应触达物理层");
    }

    #[tokio::test]
    async fn deep_virtual_address_keeps_dir_prefix() {
        let (f, v, _p) = fs();
        let c = f.read(&VdfsContext::empty(), ".vdfs/session/abc").await.unwrap();
        assert_eq!(v.seen(), vec!["session/abc"]);
        assert_eq!(c.path, ".vdfs/session/abc");
    }

    #[tokio::test]
    async fn bare_address_goes_to_physical_untouched() {
        let (f, _v, p) = fs();
        let items = f.list(&VdfsContext::empty(), "src").await.unwrap();
        assert_eq!(p.seen(), vec!["src"]);
        assert_eq!(items[0].name, "a.txt");

        // 空地址 = 工作目录根，不是虚拟根
        let (f2, v2, p2) = fs();
        f2.list(&VdfsContext::empty(), "").await.unwrap();
        assert_eq!(p2.seen(), vec![""]);
        assert!(v2.seen().is_empty());
    }

    #[tokio::test]
    async fn write_response_path_is_display_form() {
        let (f, v, _p) = fs();
        let r = f
            .write(
                &VdfsContext::empty(),
                ".vdfs/setting/x",
                &VdfsContent::text("", "1"),
            )
            .await
            .unwrap();
        assert_eq!(v.seen(), vec!["setting/x"]);
        assert_eq!(r.path, ".vdfs/setting/x");
    }

    // ==================== 两半之间不可穿越 ====================

    #[tokio::test]
    async fn move_between_halves_is_rejected() {
        let (f, v, p) = fs();
        let err = f
            .move_item(&VdfsContext::empty(), "a.txt", ".vdfs/session/b")
            .await
            .unwrap_err();
        assert!(matches!(err, VdfsError::Invalid(_)), "两半之间不可移动");
        assert!(v.seen().is_empty() && p.seen().is_empty(), "判定在触达之前");
    }

    #[tokio::test]
    async fn move_within_physical_is_forwarded() {
        let (f, _v, _p) = fs();
        // P 未实现 move_item → 转发后由 trait 缺省报 NotImplemented，
        // 关键是地址按物理半原样送达
        let err = f
            .move_item(&VdfsContext::empty(), "a.txt", "b.txt")
            .await
            .unwrap_err();
        assert!(matches!(err, VdfsError::NotImplemented));
    }

    /// 事件路径同样回到展示口径：订阅者看到的坐标系与请求时一致
    #[tokio::test]
    async fn watch_retags_change_paths() {
        struct W;
        #[async_trait]
        impl VdfsProvider for W {
            async fn watch(
                &self,
                _ctx: &VdfsContext,
                _path: &str,
                sink: VdfsChangeSink,
            ) -> VdfsResult<()> {
                sink(VdfsChange::new("session/abc", VFDS_CHANGE_UPDATED));
                sink(VdfsChange::renamed("session/old", "session/new"));
                Ok(())
            }
        }
        let f = UnifiedFs::with_physical(Arc::new(W) as DynVdfsProvider, P::new());
        let got: Arc<std::sync::Mutex<Vec<(String, String, Option<String>)>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let out = got.clone();
        f.watch(
            &VdfsContext::empty(),
            ".vdfs/session",
            Arc::new(move |c: VdfsChange| {
                out.lock().unwrap().push((c.path, c.change, c.to));
            }),
        )
        .await
        .unwrap();

        let events = got.lock().unwrap().clone();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            (
                ".vdfs/session/abc".to_string(),
                VFDS_CHANGE_UPDATED.to_string(),
                None
            )
        );
        assert_eq!(events[1].0, ".vdfs/session/old");
        assert_eq!(events[1].2.as_deref(), Some(".vdfs/session/new"));
    }

    /// 未知操作地址（穿越）在分流阶段就失败，不会两半都试一遍
    #[test]
    fn route_reports_traversal_before_touching_either_half() {
        let err = route("a/../../b").unwrap_err();
        assert!(matches!(err, VdfsError::Invalid(_)));
        assert!(err.to_string().contains("向上穿越"));
    }
}
