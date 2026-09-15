//! 存储层原语（**后端内部实现细节**）—— VDFS 挂载点的写盘 / 删除 / 导入 / 导出
//!
//! ## 现状：实体机制已废除
//!
//! 不再有 `EntityProvider` trait、不再有 `provider_registry()` 注册表、也不再有
//! `EntityVdfsAdapter` 适配器。**每个插件直接实现
//! [`VdfsProvider`](crate::symbio_core::vdfs_provider::VdfsProvider)**——列 / 读 /
//! 写 / 删 / 动作的语义由插件自己表达，VDFS 是唯一协议、唯一地址空间。
//!
//! 本模块因此只提供**跨插件共享的存储原语**（自由函数，不带任何 trait 约束）：
//!
//! - [`storage_service`]：解析当前请求的 `StorageService`（`~/.symbio/plugins/` 基座）；
//! - [`write_entity_manifest`] / [`delete_entity_dir`]：`EntityStore` 型资源的写盘与
//!   删除——**唯一实现**，避免每个插件各写一份（含「实体 id 由路径承载」的补齐）；
//! - [`read_manifest`]：读单个资源的主文件（摘要的输入）；
//! - [`import_zip_to_entity`] / [`export_entity_zip`]：整包导入 / 导出；
//! - zip / base64 工具与 [`EntityError`]。
//!
//! **差异化部分不在这里**：清单从哪来、摘要怎么算、manifest 怎么校验、上传后怎么
//! 同步内存，都是各插件的固有方法——这正是「废除实体机制」的要点：不再用一层
//! trait 集中差异、再由适配器翻译成 VDFS，而是让插件直接讲 VDFS。

pub use crate::symbio_core::schemas::entities::*;

use crate::symbio_core::providers::{EntityStore, EntityStoreError, StorageService};
use crate::symbio_core::{create_object, InvokeRequest, PluginError};
use std::io::{Cursor, Read};
use std::sync::Arc;

/// 解析当前请求的存储服务（`~/.symbio/plugins/` 基座）
pub fn storage_service(
    ctx: &Arc<dyn InvokeRequest>,
) -> Result<Arc<dyn StorageService>, PluginError> {
    create_object::<dyn StorageService>("storage_service", ctx.clone())
        .ok_or_else(|| PluginError::InternalError("storage_service 不可用".to_string()))
}

// ==================== 写盘 / 删除 / 导入 / 导出 ====================

/// 实体 id 由路径承载：manifest 缺 `id`（或为空串）时以路径段补全，已有 id 原样保留。
///
/// 这是写入路径的不变量：编辑链路前端只回纯字段值（`DetailForm` option 绑定语义，
/// id 不在表单字段里）。插件若在写盘前把 manifest 反序列化到 `id` 必填的结构体
/// （model / mcp），必须先调用本函数，否则会报「missing field `id`」。
pub fn ensure_manifest_id(manifest: &serde_json::Value, id: &str) -> serde_json::Value {
    let mut m = manifest.clone();
    let missing = m
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .is_empty();
    if missing {
        if let serde_json::Value::Object(ref mut map) = m {
            map.insert("id".to_string(), serde_json::json!(id));
        }
    }
    m
}

/// 列出某分类下的全部资源 id（`EntityStore` 型）
///
/// 列表实现的起点：插件拿到 id 列表后自行读取主文件并产出摘要（摘要口径是
/// 各插件的差异部分，不在此处）。
pub async fn list_entity_ids(
    ctx: &Arc<dyn InvokeRequest>,
    category: &str,
) -> Result<Vec<String>, PluginError> {
    let store = storage_service(ctx)?;
    store
        .entity_store()
        .list_entities(category)
        .await
        .map_err(|e| PluginError::InternalError(format!("列出实体失败: {e}")))
}

/// 读单个资源的主文件内容（`EntityStore` 型）。
///
/// 列表摘要、详情读取都以它为输入；文件不存在即 `NotFound`。
pub async fn read_manifest(
    ctx: &Arc<dyn InvokeRequest>,
    category: &str,
    manifest_file: &str,
    id: &str,
) -> Result<String, PluginError> {
    let store = storage_service(ctx)?;
    store
        .entity_store()
        .read_entity(category, id, manifest_file)
        .await
        .map_err(|e| PluginError::NotFound(format!("未找到实体「{id}」：{e}")))
}

/// 写入（创建或覆盖）一个 `EntityStore` 型资源的 **JSON** manifest —— 唯一实现。
///
/// 主文件不是 JSON 的资源（skill 的 `SKILL.md` 是 Markdown）改用
/// [`write_entity_text`]——本函数会做 `to_string_pretty`，把它用在纯文本上
/// 会写出带引号与转义的文件（详见该函数的说明）。
///
/// 职责链：补全 id → 写盘 → 发布实体生命周期事件。**校验与内存同步不在本函数内**：
/// manifest 的规范化由调用方（插件）先完成，内存同步（原 `on_uploaded`）由调用方在
/// 本函数返回后自行完成——那是各插件的差异部分。
pub async fn write_entity_manifest(
    ctx: &Arc<dyn InvokeRequest>,
    kind: &str,
    category: &str,
    manifest_file: &str,
    id: &str,
    manifest: &serde_json::Value,
) -> Result<EntityUploadResponse, PluginError> {
    let store = storage_service(ctx)?;
    let es = store.entity_store();
    let existed = es
        .entity_exists(category, id)
        .await
        .map_err(|e| PluginError::InternalError(format!("查询实体失败: {e}")))?;

    let manifest = ensure_manifest_id(manifest, id);
    let content = serde_json::to_string_pretty(&manifest)?;
    es.write_entity(category, id, manifest_file, &content)
        .await
        .map_err(|e| PluginError::InternalError(format!("写入实体失败: {e}")))?;

    // 实体生命周期变更通知：前端据此即时同步清单（created 乐观插入 / updated 重拉）。
    // 事件总线不可用不应影响写入本身的结果。写入实体均为顶层（parent_id = None）。
    crate::symbio_core::event_bus::EventBus::publish_entity_changed(
        kind,
        id,
        if existed { "updated" } else { "created" },
        None,
        None,
    )
    .await;
    // VDFS 侧变更：订阅了该挂载点的消费者（前端当前目录）据此即时刷新
    crate::symbio_core::vdfs::host::notify_change(
        kind,
        id,
        if existed { "updated" } else { "created" },
    );

    Ok(EntityUploadResponse {
        kind: kind.to_string(),
        id: id.to_string(),
        created: !existed,
    })
}

/// 写入（创建或覆盖）一个 `EntityStore` 型资源的**纯文本**主文件。
///
/// 与 [`write_entity_manifest`] 唯一的区别是形态：本函数把 `text` **原样**落盘，
/// 不做 JSON 序列化。主文件不是 JSON 的资源（skill 的 `SKILL.md` 是 Markdown）
/// 必须走这里——若把它当 JSON 值写入，`to_string_pretty` 会加上引号并把换行
/// 转义成字面 `\n`，读回来 `strip_prefix("---\n")` 必然失败：文件被静默写坏，
/// 而写盘本身报成功。
///
/// 生命周期事件与 VDFS 变更广播与 [`write_entity_manifest`] 完全一致
/// （两条写路径对消费方无差别）。
pub async fn write_entity_text(
    ctx: &Arc<dyn InvokeRequest>,
    kind: &str,
    category: &str,
    manifest_file: &str,
    id: &str,
    text: &str,
) -> Result<EntityUploadResponse, PluginError> {
    let store = storage_service(ctx)?;
    let es = store.entity_store();
    let existed = es
        .entity_exists(category, id)
        .await
        .map_err(|e| PluginError::InternalError(format!("查询实体失败: {e}")))?;

    es.write_entity(category, id, manifest_file, text)
        .await
        .map_err(|e| PluginError::InternalError(format!("写入实体失败: {e}")))?;

    crate::symbio_core::event_bus::EventBus::publish_entity_changed(
        kind,
        id,
        if existed { "updated" } else { "created" },
        None,
        None,
    )
    .await;
    crate::symbio_core::vdfs::host::notify_change(
        kind,
        id,
        if existed { "updated" } else { "created" },
    );

    Ok(EntityUploadResponse {
        kind: kind.to_string(),
        id: id.to_string(),
        created: !existed,
    })
}

/// 删除一个 `EntityStore` 型资源目录 —— **唯一实现**。
///
/// 磁盘已无目录时幂等告警（不报错），随后发布生命周期事件。
pub async fn delete_entity_dir(
    ctx: &Arc<dyn InvokeRequest>,
    kind: &str,
    category: &str,
    id: &str,
) -> Result<(), PluginError> {
    let store = storage_service(ctx)?;
    let es = store.entity_store();
    match es.delete_entity(category, id).await {
        Ok(()) => {}
        Err(EntityStoreError::NotFound { .. }) => {
            crate::plugin_warn!(kind, "磁盘上已无实体 {} 目录，仅清理内存", id);
        }
        Err(e) => {
            return Err(PluginError::InternalError(format!("删除实体失败: {e}")));
        }
    }
    crate::symbio_core::event_bus::EventBus::publish_entity_changed(
        kind, id, "deleted", None, None,
    )
    .await;
    crate::symbio_core::vdfs::host::notify_change(kind, id, "deleted");
    Ok(())
}

/// 导入整包（zip）到 `EntityStore` 型资源 —— **唯一实现**。
///
/// 与 [`write_entity_manifest`] 同构：整目录落盘 → 发布生命周期事件。区别只在
/// 内容来源与粒度：内容是一个**完整目录**（zip），同一 id 再次导入即**整目录覆盖**
/// （导入即替换，无合并语义）。
///
/// `name` 是建议名（来自新建地址）；目录自管的资源（agent bundle）不调用本函数。
pub async fn import_zip_to_entity(
    ctx: &Arc<dyn InvokeRequest>,
    kind: &str,
    category: &str,
    name: &str,
    zip: &[u8],
) -> Result<EntityUploadResponse, PluginError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(PluginError::ValidationError(
            "导入名称不能为空（zip 文件名即实体目录名）".to_string(),
        ));
    }
    let store = storage_service(ctx)?;
    let es = store.entity_store();
    let existed = es
        .entity_exists(category, name)
        .await
        .map_err(|e| PluginError::InternalError(format!("查询实体失败: {e}")))?;

    extract_zip_to_entity(es, category, name, zip).await?;

    crate::symbio_core::event_bus::EventBus::publish_entity_changed(
        kind,
        name,
        if existed { "updated" } else { "created" },
        None,
        None,
    )
    .await;
    crate::symbio_core::vdfs::host::notify_change(
        kind,
        name,
        if existed { "updated" } else { "created" },
    );

    Ok(EntityUploadResponse {
        kind: kind.to_string(),
        id: name.to_string(),
        created: !existed,
    })
}

/// 导出整包 —— **唯一实现**（与导入互为逆向）。
///
/// 把 `EntityStore` 的 `<category>/<id>/` 整个目录打包成 zip（包内顶层目录名为 id，
/// 与导入端的 `strip_common_root` 恰好配对，可原样导回）。
pub async fn export_entity_zip(
    ctx: &Arc<dyn InvokeRequest>,
    category: &str,
    id: &str,
) -> Result<EntityExport, PluginError> {
    let id = id.trim();
    if id.is_empty() {
        return Err(PluginError::ValidationError("导出目标不能为空".to_string()));
    }
    let store = storage_service(ctx)?;
    let dir = store.entity_store().entity_dir(category, id);
    if !dir.exists() {
        return Err(PluginError::NotFound(format!("未找到实体「{id}」")));
    }
    let bytes = zip_dir(&dir, id)?;
    Ok(EntityExport {
        id: id.to_string(),
        filename: format!("{id}.zip"),
        b64: encode_b64(&bytes),
    })
}

// ==================== zip 整包解包（导入的内部实现） ====================

/// 整包处理错误（zip 解码 / 解包 / 落盘；转为 `PluginError` 抛出）
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct EntityError(pub String);

/// base64 解码（VDFS 二进制通道 `VdfsContent.b64` → 字节）
pub fn decode_b64(s: &str) -> Result<Vec<u8>, EntityError> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    STANDARD
        .decode(s.trim())
        .map_err(|e| EntityError(format!("base64 解码失败: {e}")))
}

/// base64 编码（字节 → VDFS 二进制通道的载荷）
pub fn encode_b64(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    STANDARD.encode(bytes)
}

/// 把目录递归打包成 zip 字节（包内顶层目录名 = `root`）。
///
/// 与 [`extract_zip_to_entity`] 的 `strip_common_root` 配对：导出的包可直接导回。
pub fn zip_dir(dir: &std::path::Path, root: &str) -> Result<Vec<u8>, EntityError> {
    use zip::write::SimpleFileOptions;
    let mut buf = Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts = SimpleFileOptions::default();
        add_dir_to_zip(&mut w, dir, root, opts)?;
        w.finish()
            .map_err(|e| EntityError(format!("zip 生成失败: {e}")))?;
    }
    Ok(buf.into_inner())
}

/// 递归写目录（`arcname` 前缀形成单顶层目录布局）
fn add_dir_to_zip<W: std::io::Write + std::io::Seek>(
    w: &mut zip::ZipWriter<W>,
    dir: &std::path::Path,
    prefix: &str,
    opts: zip::write::SimpleFileOptions,
) -> Result<(), EntityError> {
    use std::io::Write as _;
    let entries = std::fs::read_dir(dir)
        .map_err(|e| EntityError(format!("读取目录失败: {e}")))?
        .flatten()
        .collect::<Vec<_>>();
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        // 跳过隐藏文件（与导入端同一口径）
        if name.starts_with('.') {
            continue;
        }
        let arc = format!("{prefix}/{name}");
        if path.is_dir() {
            w.add_directory(&arc, opts)
                .map_err(|e| EntityError(format!("zip 写目录失败: {e}")))?;
            add_dir_to_zip(w, &path, &arc, opts)?;
        } else {
            w.start_file(&arc, opts)
                .map_err(|e| EntityError(format!("zip 写文件失败: {e}")))?;
            let bytes =
                std::fs::read(&path).map_err(|e| EntityError(format!("读取文件失败: {e}")))?;
            w.write_all(&bytes)
                .map_err(|e| EntityError(format!("zip 写内容失败: {e}")))?;
        }
    }
    Ok(())
}

/// 解析 zip 字节为 `(相对路径, 内容)` 列表。
///
/// - 跳过目录条目、`__MACOSX` 元数据、隐藏文件
/// - 强行去掉条目前导的 `./` / `/`
pub fn parse_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, EntityError> {
    let cursor = Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| EntityError(format!("非法 zip: {e}")))?;

    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| EntityError(format!("读取 zip 条目失败: {e}")))?;

        let raw = file.name().replace('\\', "/");
        if file.is_dir() {
            continue;
        }
        // 先规范化再判隐藏：否则 `./a/b.txt` 的首段 `.` 会被当成隐藏文件整条丢弃
        let rel = normalize_zip_path(&raw);
        if rel.is_empty() {
            continue;
        }
        // 跳过 macOS 元数据 / 隐藏文件
        if rel.contains("__MACOSX")
            || rel
                .split('/')
                .any(|seg| seg.starts_with('.') && !seg.is_empty())
        {
            continue;
        }
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)
            .map_err(|e| EntityError(format!("读取 zip 条目内容失败: {e}")))?;
        out.push((rel, buf));
    }
    Ok(out)
}

/// 若 zip 内所有条目共享一个顶层根目录（常见打包方式），剥离该层，
/// 使内容平铺到目标实体目录下。
pub fn strip_common_root(entries: &mut [(String, Vec<u8>)]) {
    if entries.is_empty() {
        return;
    }
    let prefix = entries
        .iter()
        .filter_map(|(p, _)| p.split('/').next())
        .filter(|seg| !seg.is_empty())
        .min()
        .map(|root| format!("{root}/"));
    // 仅当每个条目都以此根目录开头时才剥离
    if let Some(prefix) = prefix {
        if entries.iter().all(|(p, _)| p.starts_with(&prefix)) {
            for (p, _) in entries.iter_mut() {
                if let Some(rest) = p.strip_prefix(&prefix) {
                    *p = rest.to_string();
                }
            }
        }
    }
}

/// 把已解析的 zip 内容解压写入 `EntityStore` 的 `<category>/<id>/` 目录。
///
/// - 若目录已存在则整体删除重建（导入即覆盖整包）
/// - 返回写入的文件数量
pub async fn extract_zip_to_entity(
    es: &dyn EntityStore,
    category: &str,
    id: &str,
    bytes: &[u8],
) -> Result<usize, EntityError> {
    let mut entries = parse_zip(bytes)?;
    strip_common_root(&mut entries);
    if entries.is_empty() {
        return Err(EntityError("zip 中没有任何可用的实体文件".to_string()));
    }

    let dir = es.entity_dir(category, id);
    if dir.exists() {
        tokio::fs::remove_dir_all(&dir)
            .await
            .map_err(|e| EntityError(format!("清理旧实体目录失败: {e}")))?;
    }

    for (rel, content) in &entries {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| EntityError(format!("创建目录失败: {e}")))?;
        }
        tokio::fs::write(&path, content)
            .await
            .map_err(|e| EntityError(format!("写入实体文件失败: {e}")))?;
    }
    Ok(entries.len())
}

/// 规范化 zip 内部相对路径文本（去掉前导 `./` 与 `/`）
fn normalize_zip_path(p: &str) -> String {
    p.trim_start_matches("./")
        .trim_start_matches('/')
        .to_string()
}

impl From<EntityError> for PluginError {
    fn from(e: EntityError) -> Self {
        PluginError::InternalError(e.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VDFS 编辑链路下发的 manifest 只含纯字段值（无 id）——写入前必须能以
    /// 路径 id 补全，否则 model / mcp 的校验反序列化直接报「missing field `id`」
    /// （编辑已有 Provider 保存失败即为该症状）。
    #[test]
    fn ensure_manifest_id_fills_missing_or_empty() {
        let no_id = serde_json::json!({ "name": "x", "provider": "openai" });
        assert_eq!(
            ensure_manifest_id(&no_id, "p1").get("id"),
            Some(&serde_json::json!("p1"))
        );
        // 其余字段原样保留
        assert_eq!(
            ensure_manifest_id(&no_id, "p1").get("name"),
            Some(&serde_json::json!("x"))
        );

        let empty_id = serde_json::json!({ "id": "", "name": "x" });
        assert_eq!(
            ensure_manifest_id(&empty_id, "p1").get("id"),
            Some(&serde_json::json!("p1"))
        );
    }

    #[test]
    fn ensure_manifest_id_keeps_existing() {
        let has_id = serde_json::json!({ "id": "orig", "name": "x" });
        assert_eq!(
            ensure_manifest_id(&has_id, "p1").get("id"),
            Some(&serde_json::json!("orig"))
        );
    }

    // ==================== zip 整包解包 ====================

    /// 构造内存 zip（按给定顺序写入条目；`dirs` 只建目录条目）
    fn make_zip(entries: &[(&str, Option<&[u8]>)]) -> Vec<u8> {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            let opts = SimpleFileOptions::default();
            for (name, content) in entries {
                match content {
                    Some(bytes) => {
                        w.start_file(*name, opts).unwrap();
                        w.write_all(bytes).unwrap();
                    }
                    None => {
                        w.add_directory(*name, opts).unwrap();
                    }
                }
            }
            w.finish().unwrap();
        }
        buf
    }

    /// 解包：跳过目录 / 隐藏文件 / `__MACOSX`，并规范化前导 `./` 与 `/`
    #[test]
    fn parse_zip_skips_dirs_and_metadata() {
        let bytes = make_zip(&[
            ("SKILL.md", Some(b"# demo")),
            ("scripts/run.sh", Some(b"echo hi")),
            ("scripts/", None),
            ("__MACOSX/._SKILL.md", Some(b"junk")),
            (".hidden", Some(b"junk")),
            ("./nested/ok.txt", Some(b"ok")),
        ]);
        let out = parse_zip(&bytes).unwrap();
        let names: Vec<&str> = out.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            names,
            vec!["SKILL.md", "scripts/run.sh", "nested/ok.txt"],
            "目录条目与元数据不落盘：`{names:?}`"
        );
        assert_eq!(out[0].1, b"# demo".to_vec());
    }

    /// 单顶层目录的打包习惯：剥离该层，内容平铺到实体目录
    #[test]
    fn strip_common_root_flattens_single_root() {
        let mut entries: Vec<(String, Vec<u8>)> = vec![
            ("pkg/SKILL.md".into(), b"a".to_vec()),
            ("pkg/scripts/s.sh".into(), b"b".to_vec()),
        ];
        strip_common_root(&mut entries);
        let names: Vec<&str> = entries.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, vec!["SKILL.md", "scripts/s.sh"]);

        // 不共享顶层目录 ⇒ 原样保留（避免误伤多根包）
        let mut mixed: Vec<(String, Vec<u8>)> =
            vec![("a/x.md".into(), Vec::new()), ("b/y.md".into(), Vec::new())];
        strip_common_root(&mut mixed);
        let names: Vec<&str> = mixed.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(names, vec!["a/x.md", "b/y.md"]);
    }

    /// 非法输入：非 base64 / 非 zip，都转为可读错误（不 panic）
    #[test]
    fn zip_errors_are_reported() {
        assert!(decode_b64("!!!not-base64!!!").is_err());
        assert!(parse_zip(b"not a zip at all").is_err());
        // 空包（只有目录条目）⇒ 解包报「没有任何可用文件」由 `extract_zip_to_entity` 负责
        let empty = make_zip(&[("only-dir/", None)]);
        assert!(parse_zip(&empty).unwrap().is_empty());
    }

    /// 打包：单顶层目录 = 实体 id，跳过隐藏文件——与导入端的
    /// `parse_zip` + `strip_common_root` 恰好配对（导出包可原样导回）
    #[test]
    fn zip_dir_roundtrips_with_import() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("demo");
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(dir.join("SKILL.md"), b"# demo").unwrap();
        std::fs::write(dir.join("scripts").join("run.sh"), b"echo hi").unwrap();
        // 隐藏文件不进包（与导入端同一口径）
        std::fs::write(dir.join(".DS_Store"), b"junk").unwrap();

        let bytes = zip_dir(&dir, "demo").unwrap();
        let mut entries = parse_zip(&bytes).unwrap();
        strip_common_root(&mut entries);
        let mut names: Vec<&str> = entries.iter().map(|(p, _)| p.as_str()).collect();
        names.sort_unstable();
        assert_eq!(
            names,
            vec!["SKILL.md", "scripts/run.sh"],
            "导出 → 导入可原样往返：`{names:?}`"
        );
        let body = entries
            .iter()
            .find(|(p, _)| p == "SKILL.md")
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        assert_eq!(body, b"# demo".to_vec());
    }

    /// 导出：空 id 直接校验失败（不产出空包）
    #[tokio::test]
    async fn export_entity_zip_rejects_empty_id() {
        let ctx: Arc<dyn InvokeRequest> =
            Arc::new(crate::symbio_core::SimpleRequest::new(None, None));
        let err = export_entity_zip(&ctx, "skill", "  ").await.unwrap_err();
        assert!(
            matches!(err, PluginError::ValidationError(_)),
            "空导出目标应报校验错误：{err:?}"
        );
    }
}
