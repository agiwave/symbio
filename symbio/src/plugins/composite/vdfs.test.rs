//! `symbio/src/plugins/composite/vdfs.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use crate::symbio_core::vdfs::vdfs_context;
use crate::symbio_core::{
    InvokeRequest, PluginError, PluginPayload, SimpleRequest, CAPABILITY_VISITOR,
};

/// 只暴露一个 `a.txt` 的 provider；目录名 / 顺序 / 隐藏由**假插件的 meta** 决定
struct LeafProvider;

#[async_trait]
impl VdfsProvider for LeafProvider {
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        _path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => Ok(VdfsResponse::list(vec![VdfsNode::file(
                "a.txt",
                "A",
                VdfsAccess::READ,
            )])),
            VdfsRequest::Read => Ok(VdfsResponse::Read(VdfsContent::text(format!(
                "leaf:{_path}"
            )))),
            _ => Err(VdfsError::NotImplemented),
        }
    }
}

/// 假子插件：自述来自 meta；`traverse` 时按目录名注册 provider（约定 = 插件名）
struct FakeChild {
    dir: &'static str,
    label: &'static str,
    order: i32,
    hidden: bool,
}

#[async_trait]
impl Plugin for FakeChild {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new(self.dir, self.label)
            .with_order(self.order)
            .with_hidden(self.hidden)
    }

    async fn route(self: Arc<Self>, _ctx: Arc<dyn InvokeRequest>) -> InvokeResponse {
        Err(PluginError::NotFound(self.dir.to_string()))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse {
        if ctx.get(PATH).as_deref() == Some(TRAVERSE_AVAILABLE_TOOLS) {
            if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
                visitor
                    .register_vdfs_provider(self.dir, Arc::new(LeafProvider))
                    .await;
            }
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }

    /// 系统链路：直接暴露自己的 provider（目录名由容器实例表的挂载名给出）
    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn VdfsProvider>> {
        Some(Arc::new(LeafProvider))
    }
}

type InvokeResponse = crate::symbio_core::InvokeResponse<PluginPayload>;

/// 假子插件：把**给定的** provider 暴露在自己目录名下
///
/// 与 [`FakeChild`] 的区别只有一个：那个固定暴露 [`LeafProvider`]，这个让测试
/// 自带一个只关心某一两个方法的 provider（如「只实现 `write`」的替身）。
struct ProviderChild {
    dir: &'static str,
    provider: Arc<dyn VdfsProvider>,
}

#[async_trait]
impl Plugin for ProviderChild {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new(self.dir, self.dir)
    }

    async fn route(self: Arc<Self>, _ctx: Arc<dyn InvokeRequest>) -> InvokeResponse {
        Err(PluginError::NotFound(self.dir.to_string()))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse {
        if ctx.get(PATH).as_deref() == Some(TRAVERSE_AVAILABLE_TOOLS) {
            if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
                visitor
                    .register_vdfs_provider(self.dir, self.provider.clone())
                    .await;
            }
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }

    /// 系统链路：直接暴露给定的 provider（目录名由容器实例表的挂载名给出）
    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn VdfsProvider>> {
        Some(self.provider.clone())
    }
}

fn container(children: Vec<FakeChild>) -> CompositeVdfs {
    let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
    for c in children {
        map.insert(c.dir.to_string(), Arc::new(c));
    }
    CompositeVdfs::new(Arc::new(RwLock::new(map)))
}

/// 只含一个子插件的容器，子插件把 `provider` 暴露在 `dir` 下
fn container_of(dir: &'static str, provider: Arc<dyn VdfsProvider>) -> CompositeVdfs {
    let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
    map.insert(dir.to_string(), Arc::new(ProviderChild { dir, provider }));
    CompositeVdfs::new(Arc::new(RwLock::new(map)))
}

fn host_ctx() -> VdfsContext {
    let host: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
    vdfs_context(&host)
}

/// 子插件的子目录以**自己 meta 的 name** 作为标题出现在本目录下，并按 order 升序
#[tokio::test]
async fn self_listing_shows_child_dirs_by_registered_name() {
    let vdfs = container(vec![
        FakeChild {
            dir: "alpha",
            label: "甲",
            order: 20,
            hidden: false,
        },
        FakeChild {
            dir: "beta",
            label: "乙",
            order: 10,
            hidden: false,
        },
    ]);
    let ctx = host_ctx();

    let root = vdfs
        .dispatch(
            &ctx,
            "",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap();
    let VdfsResponse::List(root) = root else {
        panic!("应为 List 响应");
    };
    assert_eq!(root.len(), 2);
    assert_eq!(
        root.iter()
            .map(|it| it.node.name.as_str())
            .collect::<Vec<_>>(),
        vec!["beta", "alpha"],
        "目录名来自子插件自己的注册；顺序按 meta.order"
    );
    // 本层**不填条目地址**（容器不知道自己被挂在哪里）——地址由访问层按
    // 请求地址回填，见 `plugins/vdfs/host.rs::fill_paths`
    assert!(root[0].path.is_empty(), "条目地址留给分发层");
    assert_eq!(root[0].node.title, "乙");
    assert_eq!(root[0].node.kind, VDFS_KIND_DIR);

    let VdfsResponse::Stat(root_node) = vdfs.dispatch(&ctx, "", VdfsRequest::Stat).await.unwrap()
    else {
        panic!("应为 Stat 响应");
    };
    // 节点是纯自述：它不带地址，容器也不知道自己的挂载名
    assert!(root_node.name.is_empty(), "挂载名由使用方在注册时选定");
    assert!(root_node.is_dir());
}

/// 隐藏属性：`meta.hidden` 的子目录不出现在列表里，但**照常可寻址**
///
/// 与文件系统的隐藏属性同义——隐藏只影响列表，不是权限也不是卸载。
#[tokio::test]
async fn hidden_dirs_are_filtered_from_listing_but_still_reachable() {
    let vdfs = container(vec![
        FakeChild {
            dir: "shown",
            label: "看得见",
            order: 1,
            hidden: false,
        },
        FakeChild {
            dir: "masked",
            label: "看不见",
            order: 2,
            hidden: true,
        },
    ]);
    let ctx = host_ctx();

    // 列表里只有未隐藏的那个
    let VdfsResponse::List(root) = vdfs
        .dispatch(
            &ctx,
            "",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
    else {
        panic!("应为 List 响应");
    };
    assert_eq!(
        root.iter()
            .map(|it| it.node.name.as_str())
            .collect::<Vec<_>>(),
        vec!["shown"]
    );

    // 但按路径 stat 照常命中，且**如实报告**隐藏属性
    let VdfsResponse::Stat(n) = vdfs
        .dispatch(&ctx, "masked", VdfsRequest::Stat)
        .await
        .unwrap()
    else {
        panic!("应为 Stat 响应");
    };
    assert_eq!(n.name, "masked");
    assert_eq!(n.title, "看不见");
    assert!(n.hidden, "stat 不该替消费者隐瞒属性");

    // 子树内容也照常可读
    let VdfsResponse::List(items) = vdfs
        .dispatch(
            &ctx,
            "masked",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
    else {
        panic!("应为 List 响应");
    };
    assert_eq!(items[0].node.name, "a.txt");
    assert!(items[0].path.is_empty(), "条目地址留给分发层");
}

/// 隐藏属性是机制级的：子插件交回来的 `list` 里标了 `hidden` 的条目同样不出现
#[tokio::test]
async fn hidden_children_from_any_provider_are_filtered() {
    struct MixedProvider;

    #[async_trait]
    impl VdfsProvider for MixedProvider {
        async fn dispatch(
            &self,
            _ctx: &VdfsContext,
            _path: &str,
            req: VdfsRequest,
        ) -> VdfsResult<VdfsResponse> {
            match req {
                VdfsRequest::List { .. } => {
                    let mut masked = VdfsNode::file("secret.txt", "内部", VdfsAccess::READ);
                    masked.hidden = true;
                    Ok(VdfsResponse::list(vec![
                        VdfsNode::file("a.txt", "A", VdfsAccess::READ),
                        masked,
                    ]))
                }
                _ => Err(VdfsError::NotImplemented),
            }
        }
    }

    struct MixedChild;

    #[async_trait]
    impl Plugin for MixedChild {
        fn meta(&self) -> PluginMeta {
            PluginMeta::new("mixed", "mixed")
        }

        async fn route(self: Arc<Self>, _ctx: Arc<dyn InvokeRequest>) -> InvokeResponse {
            Err(PluginError::NotFound("mixed".into()))
        }

        async fn traverse(
            self: Arc<Self>,
            _path: String,
            ctx: Arc<dyn InvokeRequest>,
        ) -> InvokeResponse {
            if ctx.get(PATH).as_deref() == Some(TRAVERSE_AVAILABLE_TOOLS) {
                if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
                    visitor
                        .register_vdfs_provider("mixed", Arc::new(MixedProvider))
                        .await;
                }
            }
            Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
        }

        // 系统链路：直接暴露 provider（与 LLM 链路经 `register_vdfs_provider` 注册互不冲突）
        fn get_vfs_provider(
            self: Arc<Self>,
        ) -> Option<Arc<dyn crate::symbio_core::vdfs_provider::VdfsProvider>> {
            Some(Arc::new(MixedProvider))
        }
    }

    let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
    map.insert("mixed".to_string(), Arc::new(MixedChild));
    let vdfs = CompositeVdfs::new(Arc::new(RwLock::new(map)));
    let VdfsResponse::List(items) = vdfs
        .dispatch(
            &host_ctx(),
            "mixed",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
    else {
        panic!("应为 List 响应");
    };
    assert_eq!(
        items
            .iter()
            .map(|it| it.node.name.as_str())
            .collect::<Vec<_>>(),
        vec!["a.txt"],
        "机制级的隐藏属性与 provider 是谁无关"
    );
}

/// 每个子插件用**独立**收集器：provider 不会张冠李戴
#[tokio::test]
async fn per_child_collection_keeps_ownership() {
    let vdfs = container(vec![
        FakeChild {
            dir: "alpha",
            label: "甲",
            order: 1,
            hidden: false,
        },
        FakeChild {
            dir: "beta",
            label: "乙",
            order: 2,
            hidden: false,
        },
    ]);
    let ctx = host_ctx();
    let dirs = vdfs.children_of(&ctx).await.unwrap();

    for (dir, label) in [("alpha", "甲"), ("beta", "乙")] {
        let (name, p) = dirs.iter().find(|(n, _)| n == dir).expect("该子目录应存在");
        assert_eq!(name, dir);
        assert_eq!(p.meta().name, label, "插件与目录名一一对应");
    }
}

/// 子插件各以**挂载名**作为目录名出现：两个不同挂载名的插件各自一个目录，
/// 不合并（目录名唯一性由容器实例表的键保证）。列表顺序按 meta.order 升序。
#[tokio::test]
async fn distinct_mount_names_each_get_a_dir() {
    let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
    map.insert(
        "plugin_a".to_string(),
        Arc::new(FakeChild {
            dir: "a",
            label: "甲",
            order: 2,
            hidden: false,
        }),
    );
    map.insert(
        "plugin_b".to_string(),
        Arc::new(FakeChild {
            dir: "b",
            label: "乙",
            order: 1,
            hidden: false,
        }),
    );
    let vdfs = CompositeVdfs::new(Arc::new(RwLock::new(map)));
    let ctx = host_ctx();

    let dirs = vdfs.children_of(&ctx).await.unwrap();
    assert_eq!(dirs.len(), 2, "两个不同挂载名 → 两个目录");
    // 顺序按 meta.order：plugin_b(1) 先于 plugin_a(2)
    assert_eq!(dirs[0].0, "plugin_b");
    assert_eq!(dirs[0].1.meta().name, "乙");
    assert_eq!(dirs[1].0, "plugin_a");
    assert_eq!(dirs[1].1.meta().name, "甲");

    let VdfsResponse::List(root) = vdfs
        .dispatch(
            &ctx,
            "",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
    else {
        panic!("应为 List 响应");
    };
    assert_eq!(root.len(), 2);
}

/// 子树内的路径被拆回相对路径交给叶子 provider；返回项**不带地址**，
/// 由访问层按请求地址回填（本层不知道自己被挂在哪里）
#[tokio::test]
async fn delegates_relative_path_and_leaves_address_to_the_host() {
    let vdfs = container(vec![FakeChild {
        dir: "alpha",
        label: "甲",
        order: 1,
        hidden: false,
    }]);
    let ctx = host_ctx();

    let VdfsResponse::List(items) = vdfs
        .dispatch(
            &ctx,
            "alpha",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
    else {
        panic!("应为 List 响应");
    };
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].node.name, "a.txt", "provider 交回的是相对形态");
    assert!(items[0].path.is_empty(), "条目地址留给分发层");

    let VdfsResponse::Read(c) = vdfs
        .dispatch(&ctx, "alpha/a.txt", VdfsRequest::Read)
        .await
        .unwrap()
    else {
        panic!("应为 Read 响应");
    };
    assert_eq!(
        c.text.as_deref(),
        Some("leaf:a.txt"),
        "provider 收到的是相对路径"
    );
}

/// 无子插件 → 空树（自身目录仍可列出）
#[tokio::test]
async fn empty_container_is_an_empty_vfs() {
    let vdfs = container(Vec::new());
    let ctx = host_ctx();
    let VdfsResponse::List(items) = vdfs
        .dispatch(
            &ctx,
            "",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap()
    else {
        panic!("应为 List 响应");
    };
    assert!(items.is_empty());
    let VdfsResponse::Stat(n) = vdfs.dispatch(&ctx, "", VdfsRequest::Stat).await.unwrap() else {
        panic!("应为 Stat 响应");
    };
    assert!(n.name.is_empty(), "容器不知道自己的挂载名");
}

/// 自身目录与子目录的守卫：不可读 / 删 / 移，mkdir 报已存在
///
/// ⚠️ **写不在此列**：写子目录根 = 写在**挂载点目录自身**上，那是「新建」的
/// 机制形态（使用方只说建在哪个目录，不说叫什么）。容器**不做类型特判**，
/// 一律转发给子插件判定——这里 `LeafProvider` 没实现 `write`，
/// 所以落到 `NotImplemented`，而不是容器自己抛 `Forbidden`。
#[tokio::test]
async fn guards_self_and_child_dir_roots() {
    let vdfs = container(vec![FakeChild {
        dir: "alpha",
        label: "甲",
        order: 1,
        hidden: false,
    }]);
    let ctx = host_ctx();

    assert!(matches!(
        vdfs.dispatch(&ctx, "alpha", VdfsRequest::Read)
            .await
            .unwrap_err(),
        VdfsError::Forbidden(_)
    ));
    assert!(
        vdfs.dispatch(
            &ctx,
            "alpha",
            VdfsRequest::Write {
                content: VdfsContent::text("x")
            }
        )
        .await
        .unwrap_err()
        .is_not_implemented(),
        "容器不替子插件判「目录自身能不能写」，只转发"
    );
    assert!(matches!(
        vdfs.dispatch(&ctx, "alpha", VdfsRequest::Delete { recursive: true })
            .await
            .unwrap_err(),
        VdfsError::Forbidden(_)
    ));
    assert!(matches!(
        vdfs.dispatch(&ctx, "alpha", VdfsRequest::Mkdir)
            .await
            .unwrap_err(),
        VdfsError::Invalid(_)
    ));
    // 自身目录也不可操作
    assert!(matches!(
        vdfs.dispatch(&ctx, "", VdfsRequest::Read)
            .await
            .unwrap_err(),
        VdfsError::Invalid(_)
    ));
}

/// 写在挂载点目录自身：`rel` 原样（空串）转发；回执里只有**名字**，没有地址
///
/// 「新建 = 写目录自身」在容器层唯一要做的事就是转发——新条目落在哪，由调用方拿
/// 自己的请求地址 + 回执里的名字拼出来（见 `VdfsWriteResponse::name`）。容器不再
/// 替子插件拼前缀：它连自己挂在哪里都不知道。
#[tokio::test]
async fn dir_root_write_is_forwarded_verbatim() {
    /// 支持「无名字新建」的假 provider：只记录收到的 `rel`，返回自己生成的名字
    struct RootWritableProvider;

    #[async_trait]
    impl VdfsProvider for RootWritableProvider {
        async fn dispatch(
            &self,
            _ctx: &VdfsContext,
            path: &str,
            req: VdfsRequest,
        ) -> VdfsResult<VdfsResponse> {
            let VdfsRequest::Write { .. } = req else {
                return Err(VdfsError::NotImplemented);
            };
            assert_eq!(path, "", "容器必须原样转发空 rel，而不是替子插件拼名字");
            Ok(VdfsResponse::Write(VdfsWriteResponse {
                name: Some("generated-1".to_string()),
                created: true,
                etag: None,
            }))
        }
    }

    let vdfs = container_of("rw", Arc::new(RootWritableProvider));
    let VdfsResponse::Write(r) = vdfs
        .dispatch(
            &host_ctx(),
            "rw",
            VdfsRequest::Write {
                content: VdfsContent::text("{}").with_create(),
            },
        )
        .await
        .unwrap()
    else {
        panic!("应为 Write 响应");
    };
    assert!(r.created);
    assert_eq!(
        r.name.as_deref(),
        Some("generated-1"),
        "匿名写只交回名字；地址由调用方用请求地址拼"
    );
}

/// 读回的内容**不带地址**：内容总是「请求的那个节点」的内容
///
/// 三个 form 型插件（model / mcp / skill）的 `read` 都把内容原样交回
/// （`VdfsContent::text(…)`）。地址在请求里已经有了，回显没有信息量——从前容器
/// 还替它们补前缀，那是「内容自带地址」时代的遗留。
#[tokio::test]
async fn read_content_carries_no_address() {
    struct EchoPathProvider;

    #[async_trait]
    impl VdfsProvider for EchoPathProvider {
        async fn dispatch(
            &self,
            _ctx: &VdfsContext,
            _path: &str,
            req: VdfsRequest,
        ) -> VdfsResult<VdfsResponse> {
            let VdfsRequest::Read = req else {
                return Err(VdfsError::NotImplemented);
            };
            Ok(VdfsResponse::Read(VdfsContent::text("{}")))
        }
    }

    let vdfs = container_of("echo", Arc::new(EchoPathProvider));
    let VdfsResponse::Read(c) = vdfs
        .dispatch(&host_ctx(), "echo/item.json", VdfsRequest::Read)
        .await
        .unwrap()
    else {
        panic!("应为 Read 响应");
    };
    assert_eq!(c.text.as_deref(), Some("{}"));
}

/// 未知目录明确报错
#[tokio::test]
async fn rejects_unknown_dir() {
    let vdfs = container(vec![
        FakeChild {
            dir: "alpha",
            label: "甲",
            order: 1,
            hidden: false,
        },
        FakeChild {
            dir: "beta",
            label: "乙",
            order: 2,
            hidden: false,
        },
    ]);
    let ctx = host_ctx();

    let err = vdfs
        .dispatch(
            &ctx,
            "nope",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, VdfsError::NotFound(_)));
    assert!(err.to_string().contains("alpha"), "提示现有目录");
    // 这里曾断言「跨子目录移动被拒」——容器**用错误表达能力的缺失**（它先解析
    // `to` 属于哪个子目录，再拒绝跨目录）。移动下线后这个形态连同那句断言一起
    // 消失：载荷里没有第二个地址，容器无从也无需判定。
}

/// 宿主句柄缺失（provider 未随 symbio 上下文调用）→ 明确报错而非静默空树
#[tokio::test]
async fn missing_host_ctx_is_an_internal_error() {
    let vdfs = container(Vec::new());
    // Ok 侧不可 `Debug`，故不能用 unwrap_err
    let err = match vdfs
        .dispatch(
            &VdfsContext::empty(),
            "",
            VdfsRequest::List {
                limit: None,
                before: None,
            },
        )
        .await
    {
        Ok(_) => panic!("缺宿主句柄时应报错"),
        Err(e) => e,
    };
    assert_eq!(err.code(), "INTERNAL_ERROR");
}

// ==================== 嵌套装配：多段注册名 ====================
//
// 子智能体子树里的 provider 经 agent 插件的作用域代理（SubAgentVisitor）
// 以 **多段名** 注册进系统容器：`agent/<agent_id>/<name>`。这里的测试
// 用合成多段名模拟该形态，钉住三件事：可寻址（resolve 最长前缀）、
// 派发期父地址 = 完整挂载点、根清单的呈现形态。

/// 回声 provider：`read` 返回 `<标签>:<父地址>/<path>`，同时验证两件事——
/// 收到的地址是子树相对路径、上下文父地址已被派发改写为完整挂载点。
struct EchoProvider {
    label: &'static str,
}

#[async_trait]
impl VdfsProvider for EchoProvider {
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        let VdfsRequest::Read = req else {
            return Err(VdfsError::NotImplemented);
        };
        let parent = ctx.parent_addr();
        Ok(VdfsResponse::Read(VdfsContent::text(format!(
            "{}:{parent}/{path}",
            self.label
        ))))
    }
}

/// 子插件的 provider 经 `Plugin::get_vfs_provider` 直接取回（系统链路），
/// 目录名 = 实例表的挂载名；与 LLM 链路经 `CapabilityVisitor` 收集互不干扰。
#[tokio::test]
async fn sub_provider_fetched_via_trait_method() {
    let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();
    map.insert(
        "agent".to_string(),
        Arc::new(ProviderChild {
            dir: "agent",
            provider: Arc::new(EchoProvider { label: "outer" }),
        }),
    );
    let vdfs = CompositeVdfs::new(Arc::new(RwLock::new(map)));
    let ctx = host_ctx();

    // `get_vfs_provider` 直接给出 provider；`children_of` 用挂载名 "agent" 作目录名
    let dirs = vdfs.children_of(&ctx).await.unwrap();
    assert_eq!(dirs.len(), 1);
    assert_eq!(dirs[0].0, "agent");
    assert_eq!(dirs[0].1.meta().name, "agent");

    // 经组合视图读子 provider：父地址续接、相对路径透传
    let VdfsResponse::Read(c) = vdfs
        .dispatch(&ctx, "agent/x/f.md", VdfsRequest::Read)
        .await
        .unwrap()
    else {
        panic!("应为 Read 响应");
    };
    let text = c.text.as_deref().unwrap();
    assert!(text.starts_with("outer:"), "挂载名命中: {text}");
    assert!(
        text.ends_with("/agent/x/f.md"),
        "父地址 + 相对路径 = 完整挂载点地址（根名无关）: {text}"
    );
}

// ==================== 条目地址不在本层回填 ====================
//
// 地址是「某一份列表」给条目的定位，按 `<请求地址>/<name>` 推导即可，由**访问层**
// 统一回填（`plugins/vdfs/host.rs::fill_paths`）。容器**不碰**它——它连自己挂在
// 哪里都不知道，自己填必然缺前缀（嵌套时就是缺外层那一段）。
//
// 子 provider 只在「地址推不出来」时才自己填（`VdfsItem::path`，如设置页条目
// 指向另一个挂载点）；那种地址只有拥有者知道，分发层**原样透出**。

/// 显式给了地址的 provider（模拟「地址跨挂载点」的设置页条目）
struct ExplicitPather;

#[async_trait]
impl VdfsProvider for ExplicitPather {
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        _path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => Ok(VdfsResponse::list(vec![VdfsItem::new(VdfsNode::dir(
                "inner",
                "内层",
                VdfsAccess::LIST,
            ))
            .with_path("elsewhere/inner")])),
            _ => Err(VdfsError::NotImplemented),
        }
    }
}

/// 未填地址的 provider（常态：地址由分发层推导）
struct BlankPather;

#[async_trait]
impl VdfsProvider for BlankPather {
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        _path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => Ok(VdfsResponse::list(vec![VdfsNode::dir(
                "inner",
                "内层",
                VdfsAccess::LIST,
            )])),
            _ => Err(VdfsError::NotImplemented),
        }
    }
}

#[tokio::test]
async fn container_leaves_item_addresses_untouched() {
    let ctx = host_ctx();
    let list = VdfsRequest::List {
        limit: None,
        before: None,
    };

    // ① 未填 → 容器也不填（留给访问层按请求地址回填）
    let vdfs = container_of("outer", Arc::new(BlankPather));
    let VdfsResponse::List(items) = vdfs.dispatch(&ctx, "outer", list.clone()).await.unwrap()
    else {
        panic!("应为 List 响应");
    };
    assert!(
        items.iter().all(|it| it.path.is_empty()),
        "本层不填条目地址：交给访问层"
    );

    // ② 自己填过 → 原样透出（「地址推不出来」的条目，只有拥有者知道）
    let vdfs = container_of("outer", Arc::new(ExplicitPather));
    let VdfsResponse::List(items) = vdfs.dispatch(&ctx, "outer", list).await.unwrap() else {
        panic!("应为 List 响应");
    };
    assert_eq!(
        items.iter().map(|it| it.path.as_str()).collect::<Vec<_>>(),
        vec!["elsewhere/inner"],
        "拥有者显式给的地址不被改写"
    );
}
