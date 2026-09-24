//! `symbio/src/providers/vdfs_service/memory.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

use crate::symbio_core::vdfs_provider::VdfsChange;

#[tokio::test]
async fn ops_mirror_the_disk_shapes() {
    let m = MemoryVdfs::new("model");
    let ctx = VdfsContext::empty();

    assert!(m
        .dispatch(
            &ctx,
            "",
            VdfsRequest::List {
                limit: None,
                before: None
            }
        )
        .await
        .unwrap()
        .is_list());
    let r = m
        .dispatch(
            &ctx,
            "p1.model",
            VdfsRequest::Write {
                content: VdfsContent::text("", "{\"id\":\"p1\"}"),
            },
        )
        .await
        .unwrap();
    let VdfsResponse::Write(r) = r else {
        panic!("应为 Write 响应");
    };
    assert!(r.created);
    assert_eq!(r.path, "p1");
    // 呈现扩展名不是地址的一部分
    let VdfsResponse::Read(c) = m.dispatch(&ctx, "p1", VdfsRequest::Read).await.unwrap() else {
        panic!("应为 Read 响应");
    };
    assert_eq!(c.as_text(), Some("{\"id\":\"p1\"}"));
    let VdfsResponse::Stat(n) = m
        .dispatch(&ctx, "p1.model", VdfsRequest::Stat)
        .await
        .unwrap()
    else {
        panic!("应为 Stat 响应");
    };
    assert_eq!(n.name, "p1");
    m.dispatch(&ctx, "p1", VdfsRequest::Delete { recursive: false })
        .await
        .unwrap();
    assert!(matches!(
        m.dispatch(&ctx, "p1", VdfsRequest::Delete { recursive: false })
            .await
            .unwrap_err(),
        VdfsError::NotFound(_)
    ));
}

/// 克隆共享同一张表：每次 traverse 重新构造 provider 也不会读到旧清单
#[test]
fn clones_share_one_table() {
    let a = MemoryVdfs::new("model");
    let b = a.clone();
    a.set("p1", "x");
    assert_eq!(b.get("p1").as_deref(), Some("x"));
    b.remove("p1");
    assert!(a.get("p1").is_none());
}

/// 镜像刷新（整表替换）不广播，逐条写广播——批量灌入不该打扰订阅方
///
/// 广播频道按 `kind` 全局持有（同一 provider 会被每次 traverse 重新构造），
/// 因此订阅类测试必须用**独占的 kind**，否则与同 binary 内其它测试互相串台。
#[tokio::test]
async fn replace_all_is_silent_set_is_announced() {
    let m = MemoryVdfs::new("watch-model");
    let seen: Arc<RwLock<Vec<VdfsChange>>> = Arc::new(RwLock::new(Vec::new()));
    let sink = {
        let seen = seen.clone();
        Arc::new(move |c: VdfsChange| {
            seen.write().unwrap().push(c);
        })
    };
    m.dispatch(&VdfsContext::empty(), "", VdfsRequest::Watch { sink })
        .await
        .unwrap();

    m.replace_all(vec![("quiet".to_string(), "1".to_string())]);
    m.set("loud", "2");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let paths: Vec<String> = seen
        .read()
        .unwrap()
        .iter()
        .map(|c| c.path.clone())
        .collect();
    assert_eq!(paths, vec!["loud".to_string()]);
    assert!(
        seen.read().unwrap()[0].data.is_none(),
        "资源信号无载荷：信封没有操作枚举（S27），消费端回读收敛"
    );

    // 整表替换同样是静默的（且丢掉旧条目）
    m.replace_all(vec![("p9".to_string(), "3".to_string())]);
    assert!(m.get("quiet").is_none());
    assert!(m.get("p9").is_some());
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert_eq!(seen.read().unwrap().len(), 1);
    m.dispatch(&VdfsContext::empty(), "", VdfsRequest::Unwatch)
        .await
        .unwrap();
    m.set("again", "4");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert_eq!(
        seen.read().unwrap().len(),
        1,
        "unwatch 后不得再收到事件（watch/unwatch 严格配对）"
    );
}

/// 无持久化能力的操作保持 `NotImplemented`
#[tokio::test]
async fn unsupported_ops_stay_not_implemented() {
    let m = MemoryVdfs::new("model");
    let ctx = VdfsContext::empty();
    assert!(m
        .dispatch(&ctx, "d", VdfsRequest::Mkdir)
        .await
        .unwrap_err()
        .is_not_implemented());
    assert!(m
        .dispatch(
            &ctx,
            "a",
            VdfsRequest::Move {
                to: "b".to_string()
            }
        )
        .await
        .unwrap_err()
        .is_not_implemented());
    assert!(m
        .dispatch(
            &ctx,
            "p1",
            VdfsRequest::Action {
                action: "export".to_string(),
                payload: None
            }
        )
        .await
        .unwrap_err()
        .is_not_implemented());
    let err = m
        .dispatch(
            &ctx,
            "p1",
            VdfsRequest::Write {
                content: VdfsContent::binary("", "eA==", 1),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, VdfsError::Invalid(_)));
}
