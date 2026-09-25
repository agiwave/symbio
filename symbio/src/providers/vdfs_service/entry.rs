//! 条目寻址与落盘原语（三种集中实现**共用**的同一套磁盘布局）
//!
//! ## 磁盘布局（与旧的 `FileEntityStore` **完全一致**，不做数据迁移）
//!
//! ```text
//! <类别根>/<id>/<manifest>
//! ```
//!
//! 三个实现的区别**不在布局**，而在**访问拓扑**：单文件型只暴露主文件、
//! 目录型还能下钻条目内部、内存型不落盘。因此「换拓扑」不动磁盘，
//! 「换类别」不碰协议。
//!
//! ## 为什么是自由函数而不是又一个 trait
//!
//! 这些操作没有第二种实现需要切换（磁盘就是磁盘），把它们抽成 trait 只会
//! 重新制造「`EntityStore` 抽象 + 一个实现」那层空转。类型化入口都收在
//! [`DirVdfs`](super::dir::DirVdfs) / [`SingleFileVdfs`](super::single_file::SingleFileVdfs)
//! 自己身上。

use crate::symbio_core::{VdfsAccess, VdfsError, VdfsNode, VdfsResult};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 类别根目录：`<category>` 段对应的目录
///
/// ⚠️ **生产代码不应调用它** —— 插件一律用父插件经 `PLUGIN_DIR` 告知的目录，
/// 不按插件名反推自己落在哪。当前仅剩两处合法使用：
/// ① 读旧版历史落位的数据迁移（model 的旧分类 `ai`）；② 测试构造。
///
/// **与插件目录是同一个目录**——一个插件 = 一个目录，配置（`PLUGIN.yml`）与资源
/// 同处一处。因此这里直接委托 [`plugin_dir::dir_of`]，不另写一份路径规则。
///
/// 基址每次现取（不缓存）——`home/reload` 切换 homedir 后必须立刻生效。
pub fn category_dir(category: &str) -> PathBuf {
    crate::symbio_core::dir_of(category)
}

/// 把条目 id 转成安全的磁盘段名
///
/// - `/` `\` `:` `*` `?` `"` `<` `>` `|` 替换为 `_`
/// - 去除前后空白；空串归一为 `_empty_`
/// - 禁止 `.` 与 `..`（否则条目目录会逃逸出类别根）
pub fn safe_segment(id: &str) -> String {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return "_empty_".to_string();
    }
    if trimmed == "." || trimmed == ".." {
        return format!("_{}_", trimmed);
    }
    let mut s = trimmed.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    s = s
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .collect();
    s
}

/// 条目目录：`<base>/<safe(id)>`
pub fn entry_dir(base: &Path, id: &str) -> PathBuf {
    base.join(safe_segment(id))
}

/// 相对路径 → `(条目 id 段, 条目内剩余路径)`
///
/// provider 只见子树内相对路径（规范 §2.4），首段即条目；剩余段用于目录型的
/// 下钻。`""` → `None`（挂载根，不是任何条目）。
pub fn split_rel(path: &str) -> Option<(&str, &str)> {
    let p = path.trim_matches('/');
    if p.is_empty() {
        return None;
    }
    Some(match p.split_once('/') {
        Some((head, rest)) => (head, rest.trim_matches('/')),
        None => (p, ""),
    })
}

/// 路径末段 → 条目 id（去掉 `.<kind>` 呈现扩展名）
///
/// `<根>/skill/demo.skill` 与裸 `demo` 同解——呈现扩展名是**前端选渲染器**的
/// 键，不是地址的一部分，因此寻址必须先剥掉。历史上这段代码在每个资源插件里
/// 各有一份，收敛到这里。
pub fn id_of(path: &str, kind: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.strip_suffix(&format!(".{kind}"))
        .unwrap_or(base)
        .to_string()
}

/// 整包导入的**建议名**：末段再去掉 `.zip`（新建地址是 `<name>.zip`）
pub fn pack_name_of(path: &str, kind: &str) -> String {
    let base = id_of(path, kind);
    match base.strip_suffix(".zip") {
        Some(stem) if !stem.is_empty() => stem.to_string(),
        _ => base,
    }
}

/// **无名字新建**时的自动条目 id —— 使用方只说「建在这个目录」，名字由插件定。
///
/// 形状 = `<kind>-<8 位十六进制>`：类别前缀让人读得懂磁盘目录名（目录名即 id），
/// 随机段保证唯一——同一个目录里并发新建两项不会撞名。
///
/// 它与 [`id_of`] 是同一件事的两半：**有名字**时 id 来自地址（使用方给），
/// **没名字**时 id 由这里生成（provider 给）。两者都只产出「条目 id」，
/// 谁生成的对外不可见——地址里的 id 永远由 provider 决定。
pub fn auto_id(kind: &str) -> String {
    let mut u = uuid::Uuid::new_v4().simple().to_string();
    u.truncate(8);
    format!("{kind}-{u}")
}

/// 条目摘要（落盘原语向上传递的**唯一**素材形状）
///
/// 「标题 / 状态 / 呈现扩展名 / schema」是各资源的差异，本层给不出，也不该猜；
/// 本层只保证把**寻址、原文、时间戳、字节数**这四样取准，其余由调用方合成节点。
#[derive(Debug, Clone)]
pub struct Entry {
    /// 条目 id（= 磁盘目录名，已安全化）
    pub id: String,
    /// 主文件原文；`None` = 主文件缺失或读失败（条目**存在**但内容不可用）
    pub raw: Option<String>,
    /// 主文件更新时间（Unix 秒）
    pub updated_at: Option<i64>,
    /// 主文件字节数
    pub size: Option<u64>,
}

impl Entry {
    /// 缺省节点：以 id 为名字与标题，`ext` 由主文件名推导
    ///
    /// 无差异的挂载点（不需要自定义标题 / 状态 / schema）直接用它即可，
    /// 这就是「单文件型 / 目录型可以原样注册为挂载点」的落点。
    pub fn node(&self, manifest_file: &str) -> VdfsNode {
        let mut n = VdfsNode::file(self.id.clone(), self.id.clone(), VdfsAccess::READ_WRITE);
        n.ext = manifest_file
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase());
        n.updated_at = self.updated_at;
        n.size = self.size;
        n
    }
}

/// 条目清单：类别根下的**一层目录**，按名升序
///
/// 目录不存在 = 清单为空（首次使用某类别是正常状态，不是错误）。
pub async fn list_entry_ids(base: &Path) -> VdfsResult<Vec<String>> {
    let mut rd = match tokio::fs::read_dir(base).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(VdfsError::internal(format!("读取条目清单失败：{e}"))),
    };
    let mut ids = Vec::new();
    while let Some(entry) = rd
        .next_entry()
        .await
        .map_err(|e| VdfsError::internal(format!("遍历条目清单失败：{e}")))?
    {
        let file_type = entry
            .file_type()
            .await
            .map_err(|e| VdfsError::internal(format!("读取条目类型失败：{e}")))?;
        if !file_type.is_dir() {
            continue;
        }
        // 目录名即 id：非 UTF-8 的名称无法在地址空间里表达，跳过
        if let Some(name) = entry.file_name().to_str() {
            ids.push(name.to_string());
        }
    }
    ids.sort();
    Ok(ids)
}

/// 读条目主文件原文（不存在 → [`VdfsError::NotFound`]）
pub async fn read_entry(base: &Path, id: &str, manifest_file: &str) -> VdfsResult<Entry> {
    let dir = entry_dir(base, id);
    let path = dir.join(manifest_file);
    let meta = match tokio::fs::metadata(&path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(VdfsError::NotFound(format!("未找到条目「{id}」：{path:?}")))
        }
        Err(e) => return Err(VdfsError::internal(format!("读取条目失败：{e}"))),
    };
    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| VdfsError::internal(format!("读取条目内容失败（非文本？）：{e}")))?;
    Ok(Entry {
        id: dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(id)
            .to_string(),
        updated_at: meta.modified().ok().and_then(to_unix),
        size: Some(meta.len()),
        raw: Some(text),
    })
}

/// 读条目主文件原文，**读失败降级为 `None`**（列表用的宽松版）
///
/// 列表不得被单个坏条目挡住：损坏条目降级呈现，而不是让整次 `list` 失败。
pub async fn read_entry_tolerant(base: &Path, id: &str, manifest_file: &str) -> Entry {
    match read_entry(base, id, manifest_file).await {
        Ok(e) => e,
        Err(_) => Entry {
            id: safe_segment(id),
            raw: None,
            updated_at: stat_mtime(&entry_dir(base, id)).await,
            size: None,
        },
    }
}

/// 写条目主文件（原子：`<name>.tmp` → rename），返回**是否为新建**
pub async fn write_entry(
    base: &Path,
    id: &str,
    manifest_file: &str,
    text: &str,
) -> VdfsResult<bool> {
    let dir = entry_dir(base, id);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| VdfsError::internal(format!("创建条目目录失败：{e}")))?;

    let final_path = dir.join(manifest_file);
    let tmp_path = {
        let mut name = final_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("manifest")
            .to_string();
        name.push_str(".tmp");
        final_path.with_file_name(name)
    };

    let created = !final_path.exists();
    tokio::fs::write(&tmp_path, text)
        .await
        .map_err(|e| VdfsError::internal(format!("写入临时文件失败：{e}")))?;
    if let Err(e) = tokio::fs::rename(&tmp_path, &final_path).await {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(VdfsError::internal(format!("落盘失败：{e}")));
    }
    Ok(created)
}

/// 删除条目目录（不存在 → [`VdfsError::NotFound`]，交由调用方决定是否幂等）
pub async fn remove_entry(base: &Path, id: &str) -> VdfsResult<()> {
    let dir = entry_dir(base, id);
    if !dir.is_dir() {
        return Err(VdfsError::NotFound(format!("磁盘上已无条目目录「{id}」")));
    }
    tokio::fs::remove_dir_all(&dir)
        .await
        .map_err(|e| VdfsError::internal(format!("删除条目失败：{e}")))
}

/// 条目目录是否存在
pub fn entry_exists(base: &Path, id: &str) -> bool {
    entry_dir(base, id).is_dir()
}

/// 目录 mtime（拿不到就 `None`——时间戳是展示信息，不值得为此失败）
async fn stat_mtime(path: &Path) -> Option<i64> {
    tokio::fs::metadata(path)
        .await
        .ok()?
        .modified()
        .ok()
        .and_then(to_unix)
}

/// `SystemTime` → Unix 秒
fn to_unix(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// 文件 / 目录元数据 → 节点（目录型下钻时呈现真实条目内部）
pub async fn tree_node(path: &Path, name: &str) -> VdfsResult<VdfsNode> {
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| VdfsError::not_found(format!("无法读取元数据 {path:?}：{e}")))?;
    let mut n = if meta.is_dir() {
        VdfsNode::dir(name, name, VdfsAccess::dir(true, true))
    } else {
        VdfsNode::file(name, name, VdfsAccess::file(true))
    };
    n.updated_at = meta.modified().ok().and_then(to_unix);
    if !meta.is_dir() {
        n.size = Some(meta.len());
    }
    Ok(n)
}

#[cfg(test)]
#[path = "entry.test.rs"]
mod tests;
