//! `symbio/src/plugins/vdfs/fs.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

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
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => {
                self.seen.lock().unwrap().push(path.to_string());
                let mut n = VdfsNode::dir("session", "会话", VdfsAccess::dir(true, true));
                n.path = "session".to_string();
                Ok(VdfsResponse::List(vec![n]))
            }
            VdfsRequest::Read => {
                self.seen.lock().unwrap().push(path.to_string());
                Ok(VdfsResponse::Read(VdfsContent::text(path, "v")))
            }
            VdfsRequest::Write { .. } => {
                self.seen.lock().unwrap().push(path.to_string());
                Ok(VdfsResponse::Write(VdfsWriteResponse {
                    path: path.to_string(),
                    created: true,
                    etag: None,
                }))
            }
            _ => Err(VdfsError::NotImplemented),
        }
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
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => {
                self.seen.lock().unwrap().push(path.to_string());
                Ok(VdfsResponse::List(vec![VdfsNode::file(
                    "a.txt",
                    "a.txt",
                    VdfsAccess::READ,
                )]))
            }
            VdfsRequest::Read => {
                self.seen.lock().unwrap().push(path.to_string());
                Ok(VdfsResponse::Read(VdfsContent::text(path, "p")))
            }
            _ => Err(VdfsError::NotImplemented),
        }
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

    let items = f
        .dispatch(
            &ctx,
            ".vdfsv2",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
        .into_list()
        .unwrap();
    assert_eq!(v.seen(), vec![""], "根目录进树内口径是空串");
    assert_eq!(items[0].path, ".vdfsv2/session", "结果回到展示口径");
    assert!(p.seen().is_empty(), "不应触达物理层");
}

#[tokio::test]
async fn deep_virtual_address_keeps_dir_prefix() {
    let (f, v, _p) = fs();
    let c = f
        .dispatch(
            &VdfsContext::empty(),
            ".vdfsv2/session/abc",
            VdfsRequest::Read,
        )
        .await
        .unwrap()
        .into_read()
        .unwrap();
    assert_eq!(v.seen(), vec!["session/abc"]);
    assert_eq!(c.path, ".vdfsv2/session/abc");
}

#[tokio::test]
async fn bare_address_goes_to_physical_untouched() {
    let (f, _v, p) = fs();
    let items = f
        .dispatch(
            &VdfsContext::empty(),
            "src",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
        .into_list()
        .unwrap();
    assert_eq!(p.seen(), vec!["src"]);
    assert_eq!(items[0].name, "a.txt");

    // 空地址 = 工作目录根，不是虚拟根
    let (f2, v2, p2) = fs();
    f2.dispatch(
        &VdfsContext::empty(),
        "",
        VdfsRequest::List {
            limit: None,
            before: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(p2.seen(), vec![""]);
    assert!(v2.seen().is_empty());
}

#[tokio::test]
async fn write_response_path_is_display_form() {
    let (f, v, _p) = fs();
    let r = f
        .dispatch(
            &VdfsContext::empty(),
            ".vdfsv2/setting/x",
            VdfsRequest::Write {
                content: VdfsContent::text("", "1"),
            },
        )
        .await
        .unwrap()
        .into_write()
        .unwrap();
    assert_eq!(v.seen(), vec!["setting/x"]);
    assert_eq!(r.path, ".vdfsv2/setting/x");
}

// ==================== 两半之间不可穿越 ====================

#[tokio::test]
async fn move_between_halves_is_rejected() {
    let (f, v, p) = fs();
    let err = f
        .dispatch(
            &VdfsContext::empty(),
            "a.txt",
            VdfsRequest::Move {
                to: ".vdfsv2/session/b".to_string(),
            },
        )
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
        .dispatch(
            &VdfsContext::empty(),
            "a.txt",
            VdfsRequest::Move {
                to: "b.txt".to_string(),
            },
        )
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
        async fn dispatch(
            &self,
            _ctx: &VdfsContext,
            _path: &str,
            req: VdfsRequest,
        ) -> VdfsResult<VdfsResponse> {
            if let VdfsRequest::Watch { sink } = req {
                sink(VdfsChange::bare("session/abc"));
                sink(VdfsChange::bare("session/old"));
            }
            Ok(VdfsResponse::Unit)
        }
    }
    let f = UnifiedFs::with_physical(Arc::new(W) as DynVdfsProvider, P::new());
    // 捕获到的变更路径（挂载名补全后的展示口径）
    type Captured = Arc<std::sync::Mutex<Vec<String>>>;
    let got: Captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let out = got.clone();
    f.dispatch(
        &VdfsContext::empty(),
        ".vdfsv2/session",
        VdfsRequest::Watch {
            sink: Arc::new(move |c: VdfsChange| {
                out.lock().unwrap().push(c.path); // grep-audit-allow S-002: temporary guard drops at this semicolon; await is outside the callback
            }),
        },
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
