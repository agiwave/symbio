use serde::{Deserialize, Serialize};
use symbio::symbio_kernel::plugin::{PluginFrame, PluginMessageWire};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

use super::{RequestId, WorkerError, WorkerId};

/// 单帧上限。事件正文与工具结果都可能很大，但无上限的 stdio 会让坏 worker 无限吃内存。
pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerLine {
    Ready {
        worker: WorkerId,
    },
    Request {
        id: RequestId,
        worker: WorkerId,
        message: PluginMessageWire,
    },
    Response {
        id: RequestId,
        result: Result<serde_json::Value, String>,
    },
    Event {
        frame: PluginFrame,
    },
}

pub async fn read_line<R>(reader: &mut R) -> Result<Option<String>, WorkerError>
where
    R: AsyncBufReadExt + Unpin,
{
    let mut line = String::new();
    let bytes = reader
        .read_line(&mut line)
        .await
        .map_err(|error| WorkerError::Io {
            op: "读取 worker 帧",
            path: std::path::PathBuf::from("<stdio>"),
            kind: error.kind(),
            message: error.to_string(),
        })?;
    if bytes == 0 {
        return Ok(None);
    }
    if line.trim_end_matches(['\r', '\n']).len() > MAX_LINE_BYTES {
        return Err(WorkerError::Invalid("worker 帧超过长度上限".to_string()));
    }
    Ok(Some(line))
}

pub async fn write_line<W, T: Serialize>(writer: &mut W, value: &T) -> Result<(), WorkerError>
where
    W: AsyncWriteExt + Unpin,
{
    let line =
        serde_json::to_string(value).map_err(|error| WorkerError::Invalid(error.to_string()))?;
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(stdio_error)?;
    writer.write_all(b"\n").await.map_err(stdio_error)?;
    writer.flush().await.map_err(stdio_error)
}

fn stdio_error(error: std::io::Error) -> WorkerError {
    WorkerError::Io {
        op: "写入 worker 帧",
        path: std::path::PathBuf::from("<stdio>"),
        kind: error.kind(),
        message: error.to_string(),
    }
}
