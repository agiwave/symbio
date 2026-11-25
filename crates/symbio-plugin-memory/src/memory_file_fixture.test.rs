//! `memory_file` 的测试夹具：把临时目录当成一个最小 VDFS provider。
//!
//! 生产侧只有 `MemoryFile::via(provider, rel, …)` 一种构造；测试若想验证落位，
//! 就用这里的文件 provider 参与同一条读写链路，不在生产类型上开测试专用后门。

use crate::memory_file::MemoryFile;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use symbio_plugin_api::{
    DynVdfsProvider, VdfsAccess, VdfsContent, VdfsContext, VdfsError, VdfsNode, VdfsProvider,
    VdfsRequest, VdfsResponse, VdfsResult, VdfsWriteResponse,
};

/// 一个落在实际文件上的记忆作用域；`None` = 作用域不存在。
pub(crate) fn at(
    path: Option<PathBuf>,
    write_max_bytes: usize,
    inject_max_bytes: usize,
) -> MemoryFile {
    let Some(path) = path else {
        return MemoryFile::via(None, "", write_max_bytes, inject_max_bytes);
    };
    let rel = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("MEMORY.md")
        .to_string();
    let root = path.parent().unwrap_or_else(|| Path::new("")).to_path_buf();
    let provider: DynVdfsProvider = Arc::new(FileProvider { root });
    MemoryFile::via(Some(provider), rel, write_max_bytes, inject_max_bytes)
}

pub(crate) fn absent(write_max_bytes: usize, inject_max_bytes: usize) -> MemoryFile {
    MemoryFile::via(None, "", write_max_bytes, inject_max_bytes)
}

struct FileProvider {
    root: PathBuf,
}

impl FileProvider {
    fn path(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }
}

#[async_trait]
impl VdfsProvider for FileProvider {
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        let full = self.path(path);
        match req {
            VdfsRequest::Read => match std::fs::read_to_string(&full) {
                Ok(text) => Ok(VdfsResponse::Read(VdfsContent::text(text))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    Err(VdfsError::not_found("记忆文件不存在"))
                }
                Err(e) => Err(VdfsError::internal(format!("读取失败：{e}"))),
            },
            VdfsRequest::Stat => match std::fs::metadata(&full) {
                Ok(meta) => {
                    let name = full
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("MEMORY.md")
                        .to_string();
                    let mut node = VdfsNode::file(name, "", VdfsAccess::READ_WRITE);
                    node.size = Some(meta.len());
                    node.updated_at = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64);
                    Ok(VdfsResponse::Stat(node))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    Err(VdfsError::not_found("记忆文件不存在"))
                }
                Err(e) => Err(VdfsError::internal(format!("读取元数据失败：{e}"))),
            },
            VdfsRequest::Write { content } => {
                if let Some(parent) = full.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| VdfsError::internal(format!("创建记忆目录失败：{e}")))?;
                }
                std::fs::write(&full, content.text.unwrap_or_default())
                    .map_err(|e| VdfsError::internal(format!("写入失败：{e}")))?;
                Ok(VdfsResponse::Write(VdfsWriteResponse {
                    name: None,
                    created: true,
                }))
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }
}
