//! `symbio/src/symbio_core/vdfs_provider.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// **信封契约**：`vdfs_change_of` 必须能解出 `event_bus` 产出的帧。
///
/// 信封在测试里**手搓**（不调 `event_bus::build_envelope`）：它是跨模块契约，
/// 测试要独立于产帧方来钉形状——产帧方改了形状而这里没跟着改，本用例必须红。
/// 这正是 telegram 侧那次事故的形态（产帧方与解帧方各写一份，漂移无人发现）。
#[test]
fn vdfs_change_of_unwraps_the_bus_envelope() {
    let frame = crate::symbio_core::PluginFrame::data(serde_json::json!({
        "type": "bus_event",
        "data": {
            "kind": "vdfs",
            "session_id": null,
            "data": { "path": "a.md" }
        }
    }));
    let change = vdfs_change_of(&frame).expect("应能解出 VdfsChange");
    assert_eq!(change.path, "a.md");
    assert!(change.data.is_none());
}

/// 非 `kind = "vdfs"` 的帧返回 `None`——不是错误，只是不归本域消费。
#[test]
fn vdfs_change_of_rejects_other_kinds() {
    let frame = crate::symbio_core::PluginFrame::data(serde_json::json!({
        "type": "bus_event",
        "data": { "kind": "system", "session_id": null, "data": {} }
    }));
    assert!(vdfs_change_of(&frame).is_none());
}

/// 非 `Data` 帧（`Error`）返回 `None` 而**不 panic**。
///
/// `Error` 的载荷是 `Option<Value>` 且不是信封；「先解 `Data`，其余一律 `None`」
/// 是这条路径的安全前提。
#[test]
fn vdfs_change_of_handles_non_data_frames() {
    let err = crate::symbio_core::PluginFrame::Error("boom".to_string(), None);
    assert!(vdfs_change_of(&err).is_none());
}

#[test]
fn access_flags_roundtrip() {
    let a = VdfsAccess {
        read: true,
        write: false,
        list: true,
        traverse: true,
    };
    assert_eq!(a.flags(), "rlt");
    assert_eq!(VdfsAccess::parse("rlt"), a);
    assert_eq!(VdfsAccess::parse("tlrx"), a);
    assert_eq!(VdfsAccess::NONE.flags(), "");
    assert!(VdfsAccess::NONE.is_none());
}

#[test]
fn access_serde_is_compact_string() {
    let n = VdfsNode::file("a.md", "A", VdfsAccess::READ_WRITE);
    let v = serde_json::to_value(&n).unwrap();
    assert_eq!(v["access"], serde_json::json!("rw"));
    let back: VdfsNode = serde_json::from_value(v).unwrap();
    assert_eq!(back.access, VdfsAccess::READ_WRITE);
}

#[test]
fn contains_and_constructors() {
    assert!(VdfsAccess::READ_WRITE.contains(VdfsAccess::READ));
    assert!(!VdfsAccess::READ.contains(VdfsAccess::READ_WRITE));
    assert_eq!(VdfsAccess::dir(false, true).flags(), "lt");
    assert_eq!(VdfsAccess::dir(true, true).flags(), "wlt");
    assert_eq!(
        VdfsAccess::dir(true, true),
        VdfsAccess::LIST_WRITE_TRAVERSE,
        "常量与构造器同源"
    );
    assert_eq!(VdfsAccess::file(false).flags(), "r");
}

#[test]
fn derive_ext_from_name() {
    assert_eq!(derive_ext("config.json").as_deref(), Some("json"));
    assert_eq!(derive_ext("prompts/a.MD").as_deref(), Some("md"));
    assert_eq!(derive_ext("noext"), None);
    assert_eq!(derive_ext(".env"), None);
    assert_eq!(derive_ext("a."), None);
}

#[test]
fn effective_ext_prefers_explicit() {
    let mut n = VdfsNode::file("session", "会话设置", VdfsAccess::READ_WRITE);
    assert_eq!(n.effective_ext(), None);
    n.ext = Some("form".into());
    assert_eq!(n.effective_ext().as_deref(), Some("form"));

    let n = VdfsNode::file("note.md", "笔记", VdfsAccess::READ);
    assert_eq!(n.effective_ext().as_deref(), Some("md"));
}

#[test]
fn node_is_dir_by_access_not_kind() {
    assert!(VdfsNode::dir("prompts", "提示词", VdfsAccess::LIST_TRAVERSE).is_dir());
    assert!(!VdfsNode::file("a.md", "a", VdfsAccess::READ).is_dir());
}

/// 可接受的新建类型：目录节点携带、文件节点为空、空表不序列化
#[test]
fn new_types_are_dir_scoped_and_omitted_when_empty() {
    let file = VdfsNode::file("a.md", "a", VdfsAccess::READ);
    assert!(file.new_types.is_empty());
    assert!(
        serde_json::to_value(&file)
            .unwrap()
            .get("new_types")
            .is_none(),
        "空表不得序列化（不污染文件节点）"
    );

    let dir = VdfsNode::dir("session", "会话", VdfsAccess::LIST_WRITE_TRAVERSE)
        .with_new_type(VdfsNewType::new("session", "会话").with_description("新建会话"));
    let v = serde_json::to_value(&dir).unwrap();
    assert_eq!(v["new_types"][0]["ext"], serde_json::json!("session"));
    assert_eq!(v["new_types"][0]["title"], serde_json::json!("会话"));
    assert_eq!(
        v["new_types"][0]["description"],
        serde_json::json!("新建会话")
    );
    let back: VdfsNode = serde_json::from_value(v).unwrap();
    assert_eq!(back.new_types.len(), 1);
    assert_eq!(back.new_types[0].ext, "session");
}

/// 宿主方言的呈现描述经 `schema` 透传，VDFS 不解释其内容
#[test]
fn schema_is_opaque_passthrough() {
    let n = VdfsNode::file("session", "会话设置", VdfsAccess::READ_WRITE)
        .with_ext("form")
        .with_schema(serde_json::json!({ "binding": "config" }));
    let v = serde_json::to_value(&n).unwrap();
    assert_eq!(v["schema"]["binding"], serde_json::json!("config"));
    // 场景扩展字段 flatten 到顶层
    let n = n.with_attribute("config_type", serde_json::json!("session"));
    let v = serde_json::to_value(&n).unwrap();
    assert_eq!(v["config_type"], serde_json::json!("session"));
}

#[test]
fn validation_error_shape() {
    let e = VdfsValidationError::new("保存被拒绝").with_field("port", "必须在 1-65535");
    assert!(e.has_fields());
    // 载荷可序列化：宿主据此把它编码进 PluginError 文本/数据
    let json = serde_json::to_value(&e).expect("校验载荷必须可序列化");
    assert_eq!(json["message"], serde_json::json!("保存被拒绝"));
    assert_eq!(json["fields"][0]["field"], serde_json::json!("port"));
    assert_eq!(
        json["fields"][0]["message"],
        serde_json::json!("必须在 1-65535")
    );

    // VdfsError 自身只负责携带载荷，错误码由宿主层映射
    let err = VdfsError::Invalid(e);
    assert_eq!(err.code(), "VALIDATION_ERROR");
    assert!(!err.is_not_implemented());
}

#[test]
fn error_codes_and_helpers() {
    assert_eq!(VdfsError::NotImplemented.code(), "NOT_IMPLEMENTED");
    assert!(VdfsError::NotImplemented.is_not_implemented());
    assert_eq!(VdfsError::not_found("a").code(), "NOT_FOUND");
    assert_eq!(VdfsError::invalid("x").code(), "VALIDATION_ERROR");
    assert_eq!(VdfsError::Forbidden("x".into()).code(), "FORBIDDEN");
    assert_eq!(VdfsError::Conflict("x".into()).code(), "CONFLICT");
    assert_eq!(VdfsError::internal("x").code(), "INTERNAL_ERROR");
}

#[test]
fn change_constructors_are_mount_free() {
    // provider 只报子树内相对路径，不含挂载名
    let c = VdfsChange::bare("sub/x.md");
    assert_eq!(c.path, "sub/x.md");
    assert!(c.data.is_none(), "bare = 无载荷");
    // 补挂载前缀由使用方做，事件本身不知道自己挂在哪
    assert_eq!(
        c.map_paths(|p| format!("session/{p}")).path,
        "session/sub/x.md"
    );
    // with_data：载荷是生产者按自己的词汇序列化的业务数据
    let d = VdfsChange::with_data(
        "sub/x.md",
        serde_json::json!({ "id": "m1", "delta": "片段" }),
    );
    assert_eq!(d.data.as_ref().unwrap()["delta"], "片段");
}

/// 信封**没有操作枚举**；线上形状恰好 `path`（无载荷）或 `path` + `data`。
///
/// 这条断言锁两个具体的失败模式：
///
/// 1. **枚举复活**——`change` 取值（`created` / `updated` / `deleted`）曾被
///    消费端当分派键；S27 起语义全在 `data` 的字段上，`change` 字段若回来
///    而没有生产性生产者，就是又一次「无生产者也要留着」。
/// 2. **载荷在不需要时被序列化出去**——`data` 是 `Option` +
///    `skip_serializing_if`，所以绝大多数变更（资源信号）的线上形状仍是
///    逐字不变的单键 `path`。
#[test]
fn change_envelope_has_no_operation_enum_and_data_is_opt_in() {
    fn keys(v: &serde_json::Value) -> Vec<&str> {
        let mut k: Vec<&str> = v
            .as_object()
            .expect("变更事件序列化成对象")
            .keys()
            .map(String::as_str)
            .collect();
        k.sort_unstable();
        k
    }

    let bare = serde_json::to_value(VdfsChange::bare("a")).unwrap();
    assert_eq!(keys(&bare), ["path"], "无载荷时形状必须恰好是 path");
    assert!(bare.get("change").is_none(), "操作枚举已退役，不得复活");

    let with =
        serde_json::to_value(VdfsChange::with_data("a", serde_json::json!({"id": "m1"}))).unwrap();
    assert_eq!(keys(&with), ["data", "path"]);
    assert_eq!(with["data"]["id"], "m1");
}

/// `map_paths` 是路径翻译的**唯一入口**——使用方补前缀不必逐字段重建。
#[test]
fn map_paths_is_the_single_translation_point() {
    let c = VdfsChange::bare("abc/message/m1").map_paths(|p| format!("session/{p}"));
    assert_eq!(c.path, "session/abc/message/m1");
    assert!(c.data.is_none());
    // `data` 是**载荷**不是路径，翻译必须原样带过——逐字段重建会把它丢掉
    let d = VdfsChange::with_data(
        "abc/message/m1",
        serde_json::json!({ "id": "m1", "delta": "片段" }),
    )
    .map_paths(|p| format!("session/{p}"));
    assert_eq!(d.path, "session/abc/message/m1");
    assert_eq!(d.data.as_ref().unwrap()["delta"], "片段");
}

/// `..` 判定按**路径段**，与分隔符无关。
#[test]
fn parent_segment_is_separator_agnostic() {
    assert!(has_parent_segment("../etc/passwd"));
    assert!(has_parent_segment(".."));
    assert!(has_parent_segment("a/../b"));
    // `..` 收尾：按前缀实现的旧判定会放过
    assert!(has_parent_segment("a/.."));
    // Windows 分隔符：按 `../` 前缀实现的旧判定会放过
    assert!(has_parent_segment(r"src\..\..\..\Windows"));
    assert!(has_parent_segment(r"..\etc"));
    // 含 `..` 但不是独立段 → 合法
    assert!(!has_parent_segment("a/..b/c"));
    assert!(!has_parent_segment("src/main.rs"));
}

/// 前缀判定按**路径段**——`/etcfoo` 不在 `/etc` 之内。
#[test]
fn path_within_respects_segment_boundary() {
    assert!(path_within("/etc", "/etc"));
    assert!(path_within("/etc/passwd", "/etc"));
    assert!(path_within(r"C:\Users\a\.ssh\id", r"C:\Users\a\.ssh"));
    // 裸 starts_with 会误伤这两个
    assert!(!path_within("/etcfoo", "/etc"));
    assert!(!path_within("/etc2/x", "/etc"));
    assert!(!path_within("/usr/local", "/etc"));
    // 前缀尾部多余的斜杠不影响判定
    assert!(path_within("/etc/passwd", "/etc/"));
    assert!(!path_within("/anything", ""));
}

#[test]
fn context_downcast_and_require() {
    let ctx = VdfsContext::new(7u32);
    assert_eq!(ctx.host::<u32>(), Some(&7));
    assert!(ctx.host::<u64>().is_none());
    assert!(ctx.require::<u32>().is_ok());
    let err = ctx.require::<String>().unwrap_err();
    assert_eq!(err.code(), "INTERNAL_ERROR");

    // 无宿主状态的默认上下文
    let empty = VdfsContext::empty();
    assert!(empty.host::<u32>().is_none());
}

/// 唯一接口的最小契约：**path 是独立参数**（分发先按 path、再按操作）；
/// 载荷内的次要地址（`Move.to`）用 [`VdfsRequest::map_paths`] 翻译。
#[test]
fn request_carries_only_payload_and_map_paths_rewrites_to() {
    let req = VdfsRequest::Move { to: "b".into() };
    match req.map_paths(|p| format!("sub/{p}")) {
        VdfsRequest::Move { to } => assert_eq!(to, "sub/b"),
        _ => panic!("map_paths 不得改变变体"),
    }

    // 非地址载荷原样保留
    let req = VdfsRequest::List {
        limit: Some(10),
        before: None,
    };
    match req.map_paths(|p| format!("mount/{p}")) {
        VdfsRequest::List { limit, .. } => {
            assert_eq!(limit, Some(10));
        }
        _ => panic!("变体不变"),
    }
}

/// 「什么都不支持」的 provider：每个变体都显式 `NotImplemented`
/// （match 臂编译期穷尽——新增变体而不处理，编译器立刻指出每一个实现体）。
#[tokio::test]
async fn unsupported_ops_report_not_implemented() {
    struct P;

    #[async_trait]
    impl VdfsProvider for P {
        async fn dispatch(
            &self,
            _ctx: &VdfsContext,
            _path: &str,
            _req: VdfsRequest,
        ) -> VdfsResult<VdfsResponse> {
            Err(VdfsError::NotImplemented)
        }
    }

    let ctx = VdfsContext::empty();
    let sink: VdfsChangeSink = Arc::new(|_| {});
    let reqs = [
        VdfsRequest::List {
            limit: None,
            before: None,
        },
        VdfsRequest::Stat,
        VdfsRequest::Read,
        VdfsRequest::Write {
            content: VdfsContent::text("a", "x"),
        },
        VdfsRequest::Delete { recursive: true },
        VdfsRequest::Mkdir,
        VdfsRequest::Move { to: "b".into() },
        VdfsRequest::Action {
            action: "test".into(),
            payload: None,
        },
        VdfsRequest::Watch { sink: sink.clone() },
        VdfsRequest::Unwatch,
    ];
    for req in reqs {
        assert!(
            P.dispatch(&ctx, "a", req)
                .await
                .unwrap_err()
                .is_not_implemented(),
            "未支持的变体应报 NotImplemented"
        );
    }
}
