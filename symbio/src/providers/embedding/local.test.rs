//! local.rs 单元测试
//!
//! 对应源文件: `local.rs`
//!
//! 数值对齐验证是迁移 spike 阶段的实测结论（见 `docs/DECISIONS.md` ADR-016）：
//! 参考管线（fastembed / tract）的依赖本体已移除，无法在单测中再跑。

use super::*;

/// `NoopEmbeddingService` 应稳定返回 None
#[tokio::test]
async fn test_noop_returns_none() {
    let svc = NoopEmbeddingService;
    assert!(svc.embed("anything").await.is_none());
}

/// 本地嵌入服务必须真正可用：模型能被 tract 编译，并产出 512 维单位向量。
///
/// 这是语义搜索不被静默降级的回归护栏。此前 tract 在符号化 seq 下报
/// `Failed analyse for node #N "/Unsqueeze" AddDims`，整个服务回退 Noop，
/// 而其余单测全部照常通过——只有真正跑一次推理才暴露得出来。
#[tokio::test]
async fn test_local_embedding_produces_unit_vector() {
    let svc = match LocalEmbeddingService::get_instance() {
        Ok(svc) => svc,
        Err(e) => panic!("LocalEmbeddingService 初始化失败（语义搜索会被禁用）: {e}"),
    };

    let v = match svc.embed("你好，世界").await {
        Some(v) => v,
        None => panic!("嵌入推理失败，语义搜索会退化为精确匹配"),
    };

    assert_eq!(v.len(), 512, "bge-small-zh-v1.5 输出维度应为 512");
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(
        (norm - 1.0).abs() < 1e-3,
        "输出应做 L2 归一化，实际范数 = {norm}"
    );
}

/// 超过模型位置表上限（512）的文本必须被截断而不是让推理崩掉。
///
/// 位置嵌入表只有 512 项，不截断会让 position embedding 的 Gather 越界。
/// （`tract` 时代这一条是"符号维未固定"，ORT 下是实打实的越界。）
#[tokio::test]
async fn test_local_embedding_truncates_overlong_input() {
    let svc = match LocalEmbeddingService::get_instance() {
        Ok(svc) => svc,
        Err(e) => panic!("LocalEmbeddingService 初始化失败: {e}"),
    };

    let long = "嵌入模型需要能处理超长输入。".repeat(300);
    let v = svc
        .embed(&long)
        .await
        .unwrap_or_else(|| panic!("超长输入的嵌入推理失败"));
    assert_eq!(v.len(), 512);
}
