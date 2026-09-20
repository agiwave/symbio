//! 跨插件共用的响应壳（success / status 等）
//!
//! ⚠️ 文件头曾写着 `// Corresponding Frontend: tauri/src/schemas/common.ts`，而那个文件
//! **从来没在版本史里出现过**（`git log --diff-filter=A -- 'tauri/src/protocols/*'` 为空）
//! ——同时另有 6 条同类头指向同样不存在的 `tauri/src/protocols/`。约定现在由
//! `protocol-mirror-audit` 的 **E 组**判定：写了就必须指向真实存在的文件。
//! 前端没有镜像本文件的类型，故**不写**这条头（写一条假的比不写更糟）。

use serde::{Deserialize, Serialize};

/// Generic success response - 保持向后兼容
pub type SuccessResponse = String;

/// 通用成功响应（带状态和消息）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl SimpleResponse {
    pub fn success() -> Self {
        Self {
            status: "success".to_string(),
            message: None,
        }
    }

    pub fn success_with_message(message: impl Into<String>) -> Self {
        Self {
            status: "success".to_string(),
            message: Some(message.into()),
        }
    }

    pub fn ok() -> Self {
        Self {
            status: "ok".to_string(),
            message: None,
        }
    }
}
