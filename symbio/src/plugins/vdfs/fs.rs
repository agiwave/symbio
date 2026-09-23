//! 统一地址空间门面 —— 一棵树的两半按前缀分流
//!
//! ## 唯一的地址规则
//!
//! ```text
//! <根>              根目录（系统资源清单）           ─┐
//! <根>/<子目录>/…    某插件子树                       ├─▶ 虚拟层（容器注册的根）
//!                                                   ─┘
//! README.md        工作目录相对地址                 ─┐
//! src/main.rs      同上                              ├─▶ 物理层（磁盘真实文件）
//! D:/tmp/a.txt     绝对路径                          ─┘
//! ```
//!
//! **判别只有一条**：规范化后的地址是否以根名（[`VDFS_ADDR_ROOT`]）打头。
//! 是 → 虚拟；否 → 物理。
//! 前端协议、LLM 工具两条链路共用这一个 [`UnifiedFs`]，因此翻译只存在于本文件一处，
//! 页面与工具都不需要知道展示口径与树内口径的差异。
//!
//! ## 根名归本插件，且只归本插件
//!
//! 根名是**本插件的挂载规则**：它就是这么挂在系统资源上的。字面量因此只允许出现在
//! 本插件内（[`VDFS_ADDR_ROOT`]，并经 `AddrRootDecl` 静态声明给 core 收集），
//! 对外只经两个口子出去，别的模块都不必知道它：
//!
//! | 消费者 | 拿到的形态 |
//! |---|---|
//! | 其它插件要印一个可编辑地址（提示词里的「地址：…」） | **上下文父地址 + 相对地址**（`absolute_addr`）：转发时由容器写入，插件不持有根名 |
//! | 前端要进入地址空间 | `vdfs/root`（[`crate::plugins::vdfs::protocol::VDFS_ROOT`]）：**不给地址**就能列出根，回包里的 `path` 即根地址 |
//!
//! 「改挂载名」因此只需改本文件这一行。
//!
//! ## 为什么需要「口径」转换
//!
//! 虚拟层内部（容器注册的根及其下各 provider）是通用机制，provider 只认
//! **自身子树内的相对路径**（core 不该知道宿主把根目录叫做什么）。所以本门面
//! 在进出两处各做一次轻量映射：
//!
//! | 方向 | 映射 |
//! |---|---|
//! | 请求进入虚拟层 | `<根>` → `""`；`<根>/session/x` → `session/x` |
//! | 结果回到消费者 | `session/x` → `<根>/session/x` |
//!
//! 物理层不需要映射：它的地址本来就是根名之外的样子。
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
/// **全项目唯一一处根名字面量**——它就是本插件的挂载规则，因此只允许写在这里。
/// 改这个值（如改成 `.vdfsv22`）即可整体换名，其余代码不持有它：
///
/// - 其它插件要印可编辑地址 → 用**上下文父地址**拼相对地址（core 的
///   `absolute_addr`，转发时容器已写入父地址），本插件之外无人认识这个名字；
/// - 前端要进入地址空间 → `vdfs/root`（[`super::protocol::VDFS_ROOT`]），
///   回包里的 `path` 即根地址，前端把它当**运行期数据**持有。
///
/// 根名同时经 `AddrRootDecl` 静态声明（编译期随二进制生效，容器装配子插件
/// 前即可读取）。
pub const VDFS_ADDR_ROOT: &str = ".vdfsv2";

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
/// **不强加前导 `/`**——虚拟地址以 `.vdfsv2` 打头、物理相对地址以名字打头、
/// 绝对物理地址以盘符打头，三者形态各异，统一成「以 `/` 分隔的段序列」即可。
///
/// - `""` / `"."` / `"/"` → `""`（工作目录根）
/// - `".vdfsv2/session/"` → `".vdfsv2/session"`
/// - `"/src/main.rs"` → `"src/main.rs"`（首段分隔符只是分隔，不是文件系统根）
/// - 含 `..` 段 → [`VdfsError::Invalid`]
pub fn normalize_addr(raw: &str) -> VdfsResult<String> {
    // 第一道：按**段**判 `..`，且两种分隔符都算。
    //
    // 只按 `/` 分段会被 Windows 形式的 `demo/..\..\escaped` 绕过——`\` 同样是
    // 路径分隔符，下游 `Path::join` 会照着它解析。规则本体在机制层
    // [`has_parent_segment`]，此处**复用而非再写一份**：本文件开头的地址规则
    // 说明早已定下「分段比较」，历史 bug 正是「按字符串前缀 / 单一分隔符代替
    // 按路径段比较」。
    if has_parent_segment(raw) {
        return Err(VdfsError::invalid(format!(
            "VDFS 地址不允许向上穿越：{raw}"
        )));
    }
    let mut segs: Vec<&str> = Vec::new();
    for seg in raw.trim().split('/') {
        let seg = seg.trim();
        if seg.is_empty() || seg == "." {
            continue;
        }
        // 第二道：覆盖被空白包裹的 `..`（`" .. "`）——逐段 `trim()` 才暴露它，
        // 上面的 `has_parent_segment` 按原样分段看不到。
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
/// `.vdfsv2` 必须**独占首段**：`.vdfsv2foo` 不算虚拟地址（按物理地址处理）。
fn half_of(addr: &str) -> Half {
    match addr.strip_prefix(VDFS_ADDR_ROOT) {
        Some("") => Half::Virtual(String::new()),
        Some(rest) if rest.starts_with('/') => {
            // 虚拟层树内口径：剥掉 `.vdfsv2/` 前缀，剩余部分原样透传
            Half::Virtual(rest[1..].to_string())
        }
        _ => Half::Physical(addr.to_string()),
    }
}

/// 虚拟层树内口径 → 对外展示地址（`session/x` → `<根>/session/x`）
///
/// 拼接规则复用机制层的那一份（`join_addr`），本插件只提供根名——「根 + 相对」
/// 的实现于是只有一处，本门面与其它插件的展示地址不会有第二种拼法。
pub fn to_display(root_rel: &str) -> String {
    crate::symbio_core::vdfs::join_addr(VDFS_ADDR_ROOT, root_rel)
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

    /// 树内口径 → 展示口径（`session/x` → `.vdfsv2/session/x`）。
    ///
    /// **只改非空 `path`**：空 `path` 是「provider 未填」的信号，由访问层
    /// （`host::fill_paths`）按请求地址回填成 `<父地址>/<子名>`；若在这里把它
    /// 补成 `.vdfsv2`，信号即被破坏，子节点会被错认成根。本函数与 `stat` /
    /// `read` / `write` 三处的回填口径因此完全一致：**只翻译已填的，不代填。**
    fn retag(&self, nodes: &mut [VdfsNode]) {
        for n in nodes.iter_mut() {
            Self::retag_one(n);
        }
    }

    /// 单个节点的口径翻译（`list` 与 `stat` 共用，避免两处口径漂移）
    fn retag_one(n: &mut VdfsNode) {
        if !n.path.is_empty() {
            n.path = to_display(&n.path);
        }
    }
}

#[async_trait]
impl VdfsProvider for UnifiedFs {
    fn label(&self) -> Option<&str> {
        Some("文件系统")
    }

    fn description(&self) -> Option<&str> {
        Some("虚拟根下是系统资源；其余地址是磁盘上的真实文件")
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
                Self::retag_one(&mut n);
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
            (Half::Virtual(f), Half::Virtual(t)) => self.virtual_root.move_item(ctx, &f, &t).await,
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
        (UnifiedFs::with_physical(v.clone(), p.clone()), v, p)
    }

    // ==================== 地址代数 ====================

    #[test]
    fn addresses_normalize_without_forcing_root_slash() {
        assert_eq!(normalize_addr("").unwrap(), "");
        assert_eq!(normalize_addr(".").unwrap(), "");
        assert_eq!(normalize_addr("/").unwrap(), "");
        assert_eq!(normalize_addr("a//b/").unwrap(), "a/b");
        assert_eq!(normalize_addr("./a").unwrap(), "a");
        assert_eq!(
            normalize_addr(".vdfsv2/session/").unwrap(),
            ".vdfsv2/session"
        );
        assert_eq!(normalize_addr("/src/main.rs").unwrap(), "src/main.rs");
        assert_eq!(normalize_addr("D:/tmp/a.txt").unwrap(), "D:/tmp/a.txt");
    }

    #[test]
    fn traversal_is_rejected() {
        for bad in [
            "../x",
            "a/../b",
            ".vdfsv2/../../etc",
            // 反斜杠形式：`\` 也是分隔符，只按 `/` 分段会放行（已实证可逃出条目目录）
            r"demo/..\..\escaped",
            r".vdfsv2/skill/demo/..\..\..\escaped",
            r"src\..\..\..\Windows",
            r"..\etc",
            // 被空白包裹的 `..`：逐段 trim 后才现形
            " .. ",
            "a/ .. /b",
        ] {
            assert!(normalize_addr(bad).is_err(), "应拒绝向上穿越：{bad}");
        }
    }

    /// 反斜杠包裹的合法名字**不是**穿越，不能被误伤
    #[test]
    fn backslash_names_that_are_not_traversal_still_pass() {
        assert_eq!(normalize_addr(r"a/b.c").unwrap(), r"a/b.c");
        assert_eq!(normalize_addr("a/..b/c").unwrap(), "a/..b/c");
        assert_eq!(normalize_addr("a/b..").unwrap(), "a/b..");
    }

    #[test]
    fn display_paths_are_virtual_prefixed() {
        assert_eq!(to_display(""), VDFS_ADDR_ROOT);
        assert_eq!(to_display("session"), ".vdfsv2/session");
        assert_eq!(to_display("session/abc"), ".vdfsv2/session/abc");
    }

    #[test]
    fn only_exact_first_segment_is_virtual() {
        assert_eq!(half_of(".vdfsv2"), Half::Virtual(String::new()));
        assert_eq!(half_of(".vdfsv2/x"), Half::Virtual("x".to_string()));
        assert_eq!(
            half_of(".vdfsv2/session/abc"),
            Half::Virtual("session/abc".to_string())
        );
        // `.vdfsv2foo` 不是虚拟地址；物理地址原样透传
        assert_eq!(
            half_of(".vdfsv2foo"),
            Half::Physical(".vdfsv2foo".to_string())
        );
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

        let items = f.list(&ctx, ".vdfsv2").await.unwrap();
        assert_eq!(v.seen(), vec![""], "根目录进树内口径是空串");
        assert_eq!(items[0].path, ".vdfsv2/session", "结果回到展示口径");
        assert!(p.seen().is_empty(), "不应触达物理层");
    }

    #[tokio::test]
    async fn deep_virtual_address_keeps_dir_prefix() {
        let (f, v, _p) = fs();
        let c = f
            .read(&VdfsContext::empty(), ".vdfsv2/session/abc")
            .await
            .unwrap();
        assert_eq!(v.seen(), vec!["session/abc"]);
        assert_eq!(c.path, ".vdfsv2/session/abc");
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
                ".vdfsv2/setting/x",
                &VdfsContent::text("", "1"),
            )
            .await
            .unwrap();
        assert_eq!(v.seen(), vec!["setting/x"]);
        assert_eq!(r.path, ".vdfsv2/setting/x");
    }

    // ==================== 两半之间不可穿越 ====================

    #[tokio::test]
    async fn move_between_halves_is_rejected() {
        let (f, v, p) = fs();
        let err = f
            .move_item(&VdfsContext::empty(), "a.txt", ".vdfsv2/session/b")
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
                sink(VdfsChange::bare("session/abc"));
                sink(VdfsChange::bare("session/old"));
                Ok(())
            }
        }
        let f = UnifiedFs::with_physical(Arc::new(W) as DynVdfsProvider, P::new());
        // 捕获到的变更路径（挂载名补全后的展示口径）
        type Captured = Arc<std::sync::Mutex<Vec<String>>>;
        let got: Captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        let out = got.clone();
        f.watch(
            &VdfsContext::empty(),
            ".vdfsv2/session",
            Arc::new(move |c: VdfsChange| {
                out.lock().unwrap().push(c.path); // grep-audit-allow S-002: temporary guard drops at this semicolon; await is outside the callback
            }),
        )
        .await
        .unwrap();

        let events = got.lock().unwrap().clone();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], ".vdfsv2/session/abc".to_string());
        // 兄弟子树的变更同样补上挂载前缀——坐标系始终只有一个
        assert_eq!(events[1], ".vdfsv2/session/old".to_string());
    }

    /// 未知操作地址（穿越）在分流阶段就失败，不会两半都试一遍
    #[test]
    fn route_reports_traversal_before_touching_either_half() {
        let err = route("a/../../b").unwrap_err();
        assert!(matches!(err, VdfsError::Invalid(_)));
        assert!(err.to_string().contains("向上穿越"));
    }
}
