//! 会话工作目录的 tree 场景实现（统一实体协议 tree 机制下的一个 provider 场景）
//!
//! tree 机制（`ContainerKindInfo.view = "tree"`）只定义「层级 + 懒加载 +
//! 选择」：节点是统一 `EntitySummary`（`id` = 容器内相对路径、`parent` =
//! 父路径、`expandable` = 可展开提示），经 `entities/list` 的 `parent`
//! 请求参数逐层下发。本模块是该机制的一个场景：把**会话工作目录**的
//! 文件系统层级表达为 tree 节点——机制层不感知文件语义。
//!
//! 路径安全：节点 id 一律为工作目录内的相对路径（`/` 分隔）；拒绝 `..`
//! 与绝对路径，并在 join 后做前缀校验（双重闸门，与 agent bundle 的
//! 路径白名单同风格）。

use crate::symbio_core::entities::EntitySummary;
use crate::symbio_core::{InvokeRequest, PluginError};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::types::Session;

/// 子类别的统一 kind（provider 场景自定；机制层仅透传）
pub const TREE_KIND: &str = "dir";

/// 从会话元数据取工作目录（缺失/为空 = 该会话无 tree 数据）
pub fn workdir_of(session: &Session) -> Option<String> {
    session
        .metadata
        .get("workdir")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 校验并拼接工作目录内相对路径（拒绝穿越/绝对路径，返回 (绝对路径, 规范相对路径)）
fn resolve_under(workdir: &str, rel: &str) -> Result<(PathBuf, String), PluginError> {
    let normalized = rel.trim_matches('/');
    if normalized.contains('\\') || normalized.split('/').any(|seg| seg == "..") {
        return Err(PluginError::ValidationError(format!(
            "非法路径（拒绝路径穿越）: {rel}"
        )));
    }
    let abs = Path::new(workdir).join(normalized);
    // 前缀校验（路径穿越之外的第二道安全网；两侧同为规范化形式）
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    if !canon(&abs).starts_with(canon(Path::new(workdir))) {
        return Err(PluginError::ValidationError(format!(
            "非法路径（越出工作目录）: {rel}"
        )));
    }
    Ok((abs, normalized.to_string()))
}

/// 列出 `parent`（相对路径，`None` = 根层）的下一层节点。
///
/// 节点：`kind = "dir"`（场景子类别）、`id` = 相对路径、`parent` = 父路径、
/// `expandable` = 是否目录；隐藏项（`.` 开头）不下发。排序：目录优先、
/// 名称字典序（与前端树展示约定一致）。
pub async fn list_children(
    _ctx: &Arc<dyn InvokeRequest>,
    workdir: &str,
    parent: Option<&str>,
) -> Result<Vec<EntitySummary>, PluginError> {
    let parent_rel = parent.unwrap_or("").trim_matches('/').to_string();
    let (dir_abs, _) = resolve_under(workdir, &parent_rel)?;
    if !dir_abs.is_dir() {
        return Err(PluginError::NotFound(format!(
            "目录不存在: {parent_rel}"
        )));
    }

    let mut entries = tokio::fs::read_dir(&dir_abs)
        .await
        .map_err(|e| PluginError::InternalError(format!("读取目录失败: {e}")))?;

    let mut nodes: Vec<EntitySummary> = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| PluginError::InternalError(e.to_string()))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue; // 隐藏项不下发
        }
        let child_rel = if parent_rel.is_empty() {
            name.clone()
        } else {
            format!("{parent_rel}/{name}")
        };
        let meta = entry
            .metadata()
            .await
            .map_err(|e| PluginError::InternalError(e.to_string()))?;
        let is_dir = meta.is_dir();

        let mut it = EntitySummary::new(TREE_KIND, child_rel.clone(), name);
        if !parent_rel.is_empty() {
            it.parent = Some(parent_rel.clone());
        }
        it.expandable = Some(is_dir);
        it.description = None;
        if let serde_json::Value::Object(ref mut m) = it.extra {
            let _ = m.insert("is_dir".to_string(), json!(is_dir));
            if !is_dir {
                let _ = m.insert("size".to_string(), json!(meta.len()));
            }
        }
        nodes.push(it);
    }

    // 目录优先，其余按名称字典序
    nodes.sort_by(|a, b| {
        let da = a.extra.get("is_dir").and_then(|v| v.as_bool()) != Some(true);
        let db = b.extra.get("is_dir").and_then(|v| v.as_bool()) != Some(true);
        da.cmp(&db).then_with(|| a.name.cmp(&b.name))
    });
    Ok(nodes)
}

/// 读取单个树节点：目录 → 无内容概要；文件 → 内容置于 `extra.content`
/// （只读浏览，能力声明 `TREE_READONLY`，机制不提供写回）。
pub async fn read_node(workdir: &str, rel: &str) -> Result<EntitySummary, PluginError> {
    let (abs, rel_norm) = resolve_under(workdir, rel)?;
    let meta = tokio::fs::metadata(&abs)
        .await
        .map_err(|_| PluginError::NotFound(format!("路径不存在: {rel}")))?;
    let name = abs
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| rel_norm.to_string());
    let parent_rel = match rel_norm.rsplit_once('/') {
        Some((p, _)) => p.to_string(),
        None => String::new(),
    };

    let mut it = EntitySummary::new(TREE_KIND, rel_norm.clone(), name);
    if !parent_rel.is_empty() {
        it.parent = Some(parent_rel);
    }
    let is_dir = meta.is_dir();
    it.expandable = Some(is_dir);
    if let serde_json::Value::Object(ref mut m) = it.extra {
        let _ = m.insert("is_dir".to_string(), json!(is_dir));
        if !is_dir {
            let _ = m.insert("size".to_string(), json!(meta.len()));
            if meta.len() <= MAX_INLINE_READ_BYTES {
                match tokio::fs::read_to_string(&abs).await {
                    Ok(content) => {
                        let _ = m.insert("content".to_string(), json!(content));
                    }
                    Err(_) => {
                        // 二进制/不可解码文件：不下发内容，详情回落只读概要
                    }
                }
            }
        }
    }
    Ok(it)
}

/// 内容内联下发上限（64 KiB）；超限文件只给概要，避免大文件撑爆列表协议
const MAX_INLINE_READ_BYTES: u64 = 64 * 1024;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::SimpleRequest;
    use serde_json::json;
    use tempfile::TempDir;

    fn ctx() -> Arc<dyn InvokeRequest> {
        Arc::new(SimpleRequest::new(None, None))
    }

    async fn seed(dir: &Path) {
        tokio::fs::create_dir_all(dir.join("src")).await.unwrap();
        tokio::fs::create_dir_all(dir.join(".hidden")).await.unwrap();
        tokio::fs::write(dir.join("src/lib.rs"), "fn main() {}").await.unwrap();
        tokio::fs::write(dir.join("README.md"), "# demo").await.unwrap();
        tokio::fs::write(dir.join(".env"), "SECRET=1").await.unwrap();
    }

    #[tokio::test]
    async fn root_children_skip_hidden_and_sort_dirs_first() {
        let tmp = TempDir::new().unwrap();
        seed(tmp.path()).await;
        let nodes = list_children(&ctx(), tmp.path().to_str().unwrap(), None)
            .await
            .unwrap();
        let ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(ids, vec!["src", "README.md"]);
        assert_eq!(nodes[0].expandable, Some(true));
        assert_eq!(nodes[1].expandable, Some(false));
        assert_eq!(nodes[0].parent, None);
    }

    #[tokio::test]
    async fn nested_level_carries_parent_pointer() {
        let tmp = TempDir::new().unwrap();
        seed(tmp.path()).await;
        let nodes = list_children(&ctx(), tmp.path().to_str().unwrap(), Some("src"))
            .await
            .unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, "src/lib.rs");
        assert_eq!(nodes[0].parent.as_deref(), Some("src"));
    }

    #[tokio::test]
    async fn traversal_paths_are_rejected() {
        let tmp = TempDir::new().unwrap();
        seed(tmp.path()).await;
        let wd = tmp.path().to_str().unwrap();
        assert!(list_children(&ctx(), wd, Some("../..")).await.is_err());
        assert!(read_node(wd, "../secrets").await.is_err());
    }

    #[tokio::test]
    async fn file_node_inlines_content() {
        let tmp = TempDir::new().unwrap();
        seed(tmp.path()).await;
        let wd = tmp.path().to_str().unwrap();
        let node = read_node(wd, "README.md").await.unwrap();
        assert_eq!(node.extra.get("content").and_then(|c| c.as_str()), Some("# demo"));
        let dir_node = read_node(wd, "src").await.unwrap();
        assert!(dir_node.extra.get("content").is_none());
        assert_eq!(dir_node.expandable, Some(true));
    }

    #[tokio::test]
    async fn workdir_metadata_extracts() {
        let mut s = Session::new("s1");
        assert!(workdir_of(&s).is_none());
        s.metadata["workdir"] = json!("D:/work");
        assert_eq!(workdir_of(&s).as_deref(), Some("D:/work"));
    }
}
