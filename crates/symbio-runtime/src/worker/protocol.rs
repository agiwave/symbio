use super::{RequestId, WorkerError, WorkerId};
use serde::{Deserialize, Serialize};
use symbio_platform::kernel::plugin::{PluginFrame, PluginMessageWire};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

/// 单帧上限。事件正文与工具结果都可能很大，但无上限的 stdio 会让坏 worker 无限吃内存。
pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
pub const CONTROL_CHANNEL_CAPACITY: usize = 32;
pub const EVENT_CHANNEL_CAPACITY: usize = 256;

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
    /// worker → 控制面：本进程事件总线上产生的帧。
    Event {
        frame: PluginFrame,
    },
    /// 控制面 → worker：把别的 worker 的事件投进本进程总线（已打回灌标记）。
    DeliverEvent {
        frame: PluginFrame,
    },
    StartSubagent {
        id: RequestId,
        child_session_id: String,
        message: PluginMessageWire,
    },
    SubagentStarted {
        id: RequestId,
        result: Result<(), String>,
    },
    CancelSubagent {
        id: RequestId,
        worker: WorkerId,
    },
    Shutdown {
        worker: WorkerId,
    },
    BrokerOpen {
        worker: WorkerId,
        stream: u64,
        target: String,
        method: String,
        path: String,
        headers: Vec<(String, String)>,
        body: String,
    },
    BrokerHead {
        worker: WorkerId,
        stream: u64,
        status: u16,
        headers: Vec<(String, String)>,
    },
    BrokerData {
        worker: WorkerId,
        stream: u64,
        offset: u64,
        data: String,
    },
    BrokerEnd {
        worker: WorkerId,
        stream: u64,
    },
    BrokerError {
        worker: WorkerId,
        stream: u64,
        message: String,
    },
    BrokerCancel {
        worker: WorkerId,
        stream: u64,
    },
}

pub async fn read_line<R>(reader: &mut R) -> Result<Option<String>, WorkerError>
where
    R: AsyncBufReadExt + Unpin,
{
    let mut bytes = Vec::new();
    loop {
        let filled = reader.fill_buf().await.map_err(read_error)?;
        if filled.is_empty() {
            break;
        }
        let newline = filled.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(filled.len(), |index| index + 1);
        let content_end = newline.map_or(take, |index| {
            if index > 0 && filled[index - 1] == b'\r' {
                index - 1
            } else {
                index
            }
        });
        if bytes.len().saturating_add(content_end) > MAX_LINE_BYTES {
            return Err(WorkerError::Invalid("worker 帧超过长度上限".to_string()));
        }
        bytes.extend_from_slice(&filled[..take]);
        reader.consume(take);
        if newline.is_some() {
            break;
        }
    }
    if bytes.is_empty() {
        return Ok(None);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| WorkerError::Invalid(format!("worker 帧不是有效 UTF-8: {error}")))
}

pub async fn write_line<W, T: Serialize>(writer: &mut W, value: &T) -> Result<(), WorkerError>
where
    W: AsyncWriteExt + Unpin,
{
    let line =
        serde_json::to_string(value).map_err(|error| WorkerError::Invalid(error.to_string()))?;
    if line.len() > MAX_LINE_BYTES {
        return Err(WorkerError::Invalid("worker 帧超过长度上限".to_string()));
    }
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(stdio_error)?;
    writer.write_all(b"\n").await.map_err(stdio_error)?;
    writer.flush().await.map_err(stdio_error)
}

fn read_error(error: std::io::Error) -> WorkerError {
    WorkerError::Io {
        op: "读取 worker 帧",
        path: std::path::PathBuf::from("<stdio>"),
        kind: error.kind(),
        message: error.to_string(),
    }
}

fn stdio_error(error: std::io::Error) -> WorkerError {
    WorkerError::Io {
        op: "写入 worker 帧",
        path: std::path::PathBuf::from("<stdio>"),
        kind: error.kind(),
        message: error.to_string(),
    }
}
