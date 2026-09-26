//! 节点内容：文本 / 二进制互斥载荷与「新建」写意图位。
//!
use serde::{Deserialize, Serialize};

use super::node::is_false;

// ==================== 内容 ====================

/// 节点内容（文本或二进制，二者互斥）。
///
/// **不带地址**：内容总是「某个节点的」内容，而那个节点由本次调用的 `path` 参数
/// 指认（[`VdfsProvider::dispatch`](super::VdfsProvider::dispatch)）。读回来的内容里再写一遍请求地址，等于把
/// 调用方已经知道的东西回传——消费者要的是正文。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VdfsContent {
    /// 文本内容（`binary == false`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// base64 内容（`binary == true`）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b64: Option<String>,
    /// 是否二进制
    #[serde(default)]
    pub binary: bool,
    /// 字节数
    #[serde(default)]
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    /// 内容版本（乐观并发令牌；provider 可选实现）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    /// **写意图**：允许创建缺失的节点（`vdfs/write` 的 `create` 位）。
    ///
    /// provider 据此区分「新建」与「覆盖」——规范 §5.3 的「新建」正是
    /// 「对目标地址的一次 `vdfs/write`（`create: true`）」，创建语义（生成
    /// 标识、校验归属……）由 provider 自持。读取结果恒为 `false`。
    ///
    /// ⚠️ 它只回答「目标不存在时怎么办」，**不改变内容的处理方式**：内容一律取自
    /// 本次写入（见 [`VdfsRequest::Write`](super::VdfsRequest::Write) 的 `create` 位一节）。唯一例外是内容为空
    /// ——那是「先建一个，随后再填」，由 provider 落最小合法内容。
    ///
    /// 具名目标 + 不存在：**写入型资源应就地创建**（「给了名字就写得进去」），
    /// 只有「更新既有对象的字段」型语义才报 [`VdfsError::NotFound`](super::VdfsError::NotFound)。
    ///
    /// 写**目录自身**（无名字，见 [`VdfsRequest::Write`](super::VdfsRequest::Write)）时本字段是唯一判据：
    /// 那种写没有任何「已存在的目标」可覆盖，`false` 只能报错。
    #[serde(default, skip_serializing_if = "is_false")]
    pub create: bool,
}

impl VdfsContent {
    /// 文本内容
    pub fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            size: text.len() as u64,
            text: Some(text),
            ..Default::default()
        }
    }

    /// 二进制内容（base64）
    pub fn binary(b64: impl Into<String>, size: u64) -> Self {
        Self {
            b64: Some(b64.into()),
            binary: true,
            size,
            ..Default::default()
        }
    }

    pub fn with_mime(mut self, mime: impl Into<String>) -> Self {
        self.mime = Some(mime.into());
        self
    }

    pub fn with_etag(mut self, etag: impl Into<String>) -> Self {
        self.etag = Some(etag.into());
        self
    }

    /// 标记为「新建」写意图（允许创建缺失节点）
    pub fn with_create(mut self) -> Self {
        self.create = true;
        self
    }

    /// 取文本视图（二进制返回 `None`）
    pub fn as_text(&self) -> Option<&str> {
        self.text.as_deref()
    }
}
