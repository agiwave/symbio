//! `symbio/src/providers/vdfs_service/entry.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// id 安全化：分隔符与控制字符全部落地为 `_`，点段被禁用
#[test]
fn safe_segment_neutralizes_separators() {
    assert_eq!(safe_segment("demo"), "demo");
    assert_eq!(safe_segment("  demo  "), "demo");
    assert_eq!(safe_segment("a/b\\c:d"), "a_b_c_d");
    assert_eq!(safe_segment("."), "_._");
    assert_eq!(safe_segment(".."), "_.._");
    assert_eq!(safe_segment(""), "_empty_");
    // 首尾空白清掉后仍为空 ⇒ 与空串同解
    assert_eq!(safe_segment("   "), "_empty_");
}

/// 呈现扩展名不是地址的一部分：带不带 `.skill` 必须寻到同一个条目
#[test]
fn id_of_strips_presentation_extension() {
    assert_eq!(id_of("demo", "skill"), "demo");
    assert_eq!(id_of("demo.skill", "skill"), "demo");
    assert_eq!(id_of("a/b/demo.skill", "skill"), "demo");
    // 别人的扩展名不误剥（mcp 条目不会因 `.skill` 结尾被改名）
    assert_eq!(id_of("demo.other", "skill"), "demo.other");
    // 只有 `.skill` 一个段时剥完仍回落到自身（不产生空 id）
    assert_eq!(id_of("demo", "skill"), "demo");
}

#[test]
fn pack_name_strips_zip_then_kind() {
    assert_eq!(pack_name_of("demo.zip", "skill"), "demo");
    assert_eq!(pack_name_of("demo.skill", "skill"), "demo");
    assert_eq!(pack_name_of(".zip", "skill"), ".zip");
}

/// 无名字新建的自动 id：带类别前缀、随机段定长、两次不撞、且本身就是安全段名
#[test]
fn auto_id_is_prefixed_unique_and_safe() {
    let a = auto_id("model");
    let b = auto_id("model");
    assert!(a.starts_with("model-"), "带类别前缀便于人读：{a}");
    assert_eq!(a.len(), "model-".len() + 8, "随机段定长：{a}");
    assert_ne!(a, b, "两次生成必须不同");
    assert_eq!(safe_segment(&a), a, "自动 id 必须本身就是安全段名");
}

#[test]
fn split_rel_separates_entry_and_inner_path() {
    assert_eq!(split_rel(""), None);
    assert_eq!(split_rel("/"), None);
    assert_eq!(split_rel("demo"), Some(("demo", "")));
    assert_eq!(
        split_rel("demo/scripts/run.sh"),
        Some(("demo", "scripts/run.sh"))
    );
    assert_eq!(split_rel("demo/sub/"), Some(("demo", "sub")));
}

/// 缺省节点：`ext` 由主文件名推导，不硬编码资源类型
#[test]
fn entry_default_node_derives_ext_from_manifest_file() {
    let e = Entry {
        id: "demo".into(),
        raw: Some("{}".into()),
        updated_at: Some(100),
        size: Some(2),
    };
    let n = e.node("SKILL.md");
    assert_eq!(n.ext.as_deref(), Some("md"));
    assert_eq!(n.name, "demo");
    assert_eq!(n.updated_at, Some(100));
    assert_eq!(e.node("provider.json").ext.as_deref(), Some("json"));
}

#[tokio::test]
async fn missing_category_dir_is_an_empty_list() {
    let base = std::env::temp_dir().join("symbio-entry-missing-category");
    let _ = std::fs::remove_dir_all(&base);
    assert!(list_entry_ids(&base).await.unwrap().is_empty());
}

#[tokio::test]
async fn write_then_read_then_remove_roundtrips() {
    let base = temp_base("roundtrip");
    let created = write_entry(&base, "p1", "provider.json", "{\"id\":\"p1\"}")
        .await
        .unwrap();
    assert!(created, "首次写入即新建");
    // 覆盖写不再算新建
    assert!(
        !write_entry(&base, "p1", "provider.json", "{\"id\":\"p1\",\"x\":1}")
            .await
            .unwrap()
    );

    let e = read_entry(&base, "p1", "provider.json").await.unwrap();
    assert_eq!(e.raw.as_deref(), Some("{\"id\":\"p1\",\"x\":1}"));
    assert_eq!(e.size, Some(17));
    assert!(e.updated_at.is_some());
    assert_eq!(list_entry_ids(&base).await.unwrap(), vec!["p1".to_string()]);
    assert!(entry_exists(&base, "p1"));

    remove_entry(&base, "p1").await.unwrap();
    assert!(!entry_exists(&base, "p1"));
    // 删除已不存在的条目报 NotFound（幂等与否由调用方决定）
    assert!(matches!(
        remove_entry(&base, "p1").await.unwrap_err(),
        VdfsError::NotFound(_)
    ));
    let _ = std::fs::remove_dir_all(&base);
}

/// 坏条目不得让整次列表失败：主文件缺失时降级为 `raw = None`
#[tokio::test]
async fn tolerant_read_degrades_instead_of_failing() {
    let base = temp_base("tolerant");
    std::fs::create_dir_all(base.join("broken")).unwrap();
    let e = read_entry_tolerant(&base, "broken", "SKILL.md").await;
    assert!(e.raw.is_none());
    assert_eq!(e.id, "broken");
    assert!(read_entry(&base, "broken", "SKILL.md").await.is_err());
    let _ = std::fs::remove_dir_all(&base);
}

fn temp_base(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "symbio-vdfs-entry-{tag}-{}-{:?}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}
