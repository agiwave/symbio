//! VDFS 工具的表驱动骨架 —— 把「同形工具」的样板收成一处的唯一实现。
//!
//! ## 为什么有本模块
//!
//! VDFS 的工具里，有六个的形状**逐字相同**：一个持 `Arc<ToolVdfs>` 的结构体、
//! 一个 `meta()`（名字 / 描述 / schema / 示例 / 保留策略）、一个 `execute()`
//! （`ensure_required` → 取参数 → 调 provider 的某个方法 → 拼一个回执 JSON）。
//! 它们真正不同的只有四样东西：
//!
//! 1. **工具名与描述**（喂给 LLM 的文案）；
//! 2. **入参 schema 与示例**；
//! 3. **上下文保留策略**（`LastOnly` / 无）；
//! 4. **调用哪个 provider 方法、怎么把结果拼成回执**——这是唯一的**真逻辑**。
//!
//! 此前这四样散在独立文件里，而第 1~3 样之外的部分（结构体声明、`new`、`impl`、
//! 导入行）是纯复制。新增一个工具 = 新建一个文件、抄一遍骨架、填四个洞。
//!
//! 本模块把「骨架」抽成 [`VdfsTool`]（**唯一**的 `PortExecutor` 实现），把「四个洞」
//! 抽成 [`ToolSpec`]（静态表 + 一个异步闭包）。于是每个工具退化成**一条表项**。
//!
//! ## 边界：为什么不是八个全收
//!
//! `vdfs_read`（行号 + 分页）与 `vdfs_list`（ignore glob 过滤 + 目录优先排序）带
//! **可读的呈现加工**，不是「转发 + 包一层回执」；把它们塞进表会逼出一个
//! 「按工具名分支」的大 `execute`——那等于把独立文件的样板换成一份带六个分支的
//! 函数，**没有消除任何东西**。故这两者保留为独立文件。
//!
//! 判据是「有没有表装不下的呈现逻辑」，不是「行数够不够少」。

use super::{request_of, tool, ToolVdfs};
use async_trait::async_trait;
use futures::future::BoxFuture;
use serde_json::Value;
use std::sync::Arc;
use symbio_plugin_api::{
    missing_caller, port_result, CapabilityMeta, CapabilityRiskLevel, ExecEnv, PluginError,
    PluginInvokeRequest, PortExecContext, PortExecutor, PortInput, PortOutcome,
};

/// 工具的真逻辑签名：`(provider, 已拆好的参数, ctx) -> 回执 JSON`。
///
/// 具名 trait object（而非 `async fn` 指针）是因为 `async` 闭包不能直接存进
/// 静态表；调用方 [`VdfsTool::execute`] 只负责把三个参数透传过来。
pub type ToolCall = fn(
    Arc<ToolVdfs>,
    Value,
    Arc<dyn PluginInvokeRequest>,
) -> BoxFuture<'static, Result<Value, PluginError>>;

/// 一个同形工具的全部差异。
pub struct ToolSpec {
    /// LLM 可见的工具短名（`vdfs_` 打头，注册时由 `traverse` 拼上挂载点前缀）
    pub name: &'static str,
    /// 工具描述（喂给 LLM 的行为说明）
    pub description: &'static str,
    /// 入参 JSON Schema
    pub schema: fn() -> Value,
    /// 调用示例（协议适配层会追加到描述之后）
    pub examples: &'static [&'static str],
    /// 风险等级：这张表里的工具同形但风险不同（`vdfs_read` 只读、`vdfs_write`
    /// 改状态），故风险是**逐工具的差异**之一，不靠默认值隐式决定。
    pub risk: CapabilityRiskLevel,
    /// 自声明的行为名（DE-025 ）。写族四个声明 `file_change`（产物格因此带上
    /// `file_op`，工作目录变更账按声明折，不按工具名猜）；读 / 统计族声明空集。
    pub behaviors: &'static [&'static str],
    /// 真逻辑：取参数、调 provider、拼回执
    pub call: ToolCall,
}

/// 同形工具的**唯一**端口执行体——所有 [`ToolSpec`] 共用它。
pub struct VdfsTool {
    provider: Arc<ToolVdfs>,
    spec: &'static ToolSpec,
}

impl VdfsTool {
    pub fn new(provider: Arc<ToolVdfs>, spec: &'static ToolSpec) -> Self {
        Self { provider, spec }
    }

    pub fn meta(&self) -> CapabilityMeta {
        tool(
            self.spec.name,
            self.spec.description,
            (self.spec.schema)(),
            self.spec.examples.to_vec(),
            self.spec.risk,
            self.spec.behaviors,
        )
    }
}

#[async_trait]
impl PortExecutor for VdfsTool {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let Some(ctx) = exec.ctx else {
            return missing_caller();
        };
        let env = ExecEnv::from_request(&*ctx);
        port_result(self.run(input.payload, &env, ctx).await)
    }
}

impl VdfsTool {
    async fn run(
        &self,
        args: Value,
        _env: &ExecEnv,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        (self.spec.call)(self.provider.clone(), args, ctx).await
    }
}

// ==================== 六个同形工具的规格表 ====================
//
// 每条 `call` 是一个**普通 `fn` 指针**，指向一个返回 `BoxFuture` 的 async 块。
// 这样表可以在 `static` 里直接写（async 闭包不能），且每个工具的「真逻辑」集中
// 在它自己的那条表项里——想改 `vdfs_write` 的回执形状，就看 `vdfs_write` 那一项。

/// 全部同形工具的表——**新增一个「转发 + 回执」型工具只需加一条表项**。
pub static SPECS: &[ToolSpec] = &[
    ToolSpec {
        name: "vdfs_stat",
        description: "读取单个节点的元数据（名称/访问位/大小/更新时间/类型），不返回内容。",
        schema: || {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": super::PATH_SHORT }
                },
                "required": ["path"]
            })
        },
        examples: &["{\"path\":\"README.md\"}"],
        risk: CapabilityRiskLevel::Low,
        behaviors: &[],
        call: |p, args, ctx| {
            Box::pin(async move {
                super::ensure_required(&args, "path")?;
                let req: super::super::protocol::VdfsPathRequest = request_of(&args);
                let node = p.stat(&ctx, &req.path).await?;
                Ok(serde_json::to_value(&node)?)
            })
        },
    },
    ToolSpec {
        name: "vdfs_delete",
        description: "删除文件或目录。删除目录需 recursive=true。不可逆，删除前务必确认路径。",
        schema: || {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": super::PATH_SHORT },
                    "recursive": { "type": "boolean", "description": "删除目录时是否递归" }
                },
                "required": ["path"]
            })
        },
        examples: &[
            "{\"path\":\"tmp/old.txt\"}",
            "{\"path\":\"tmp\",\"recursive\":true}",
        ],
        risk: CapabilityRiskLevel::High,
        behaviors: &[crate::behavior::FILE_CHANGE],
        call: |p, args, ctx| {
            Box::pin(async move {
                super::ensure_required(&args, "path")?;
                let req: super::super::protocol::VdfsPathRequest = request_of(&args);
                p.delete(&ctx, &req.path, req.recursive).await?;
                Ok(serde_json::json!({
                    "success": true,
                    "path": req.path,
                    "message": format!("已删除 {}", req.path),
                }))
            })
        },
    },
    ToolSpec {
        name: "vdfs_mkdir",
        description: "新建目录（父目录不存在时自动创建）。",
        schema: || {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": super::PATH_SHORT }
                },
                "required": ["path"]
            })
        },
        examples: &["{\"path\":\"docs/notes\"}"],
        risk: CapabilityRiskLevel::Medium,
        behaviors: &[crate::behavior::FILE_CHANGE],
        call: |p, args, ctx| {
            Box::pin(async move {
                super::ensure_required(&args, "path")?;
                let req: super::super::protocol::VdfsPathRequest = request_of(&args);
                p.mkdir(&ctx, &req.path).await?;
                Ok(serde_json::json!({
                    "success": true,
                    "path": req.path,
                    "message": format!("已创建目录 {}", req.path),
                }))
            })
        },
    },
    ToolSpec {
        name: "vdfs_write",
        description: "写入（创建或覆盖）文件内容。父目录不存在时自动创建。写前应先用 vdfs_read 或 vdfs_stat 确认目标存在性与访问位；失败时返回的错误可能含字段级校验信息，请据其修正后重试。",
        schema: || {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": super::PATH_SHORT },
                    "text": { "type": "string", "description": "文本内容（与 b64 二选一）" },
                    "b64": { "type": "string", "description": "base64 内容（与 text 二选一）" }
                },
                "required": ["path"]
            })
        },
        examples: &["{\"path\":\"notes.md\",\"text\":\"# 标题\\n\"}"],
        risk: CapabilityRiskLevel::Medium,
        behaviors: &[crate::behavior::FILE_CHANGE],
        call: |p, args, ctx| {
            Box::pin(async move {
                super::ensure_required(&args, "path")?;
                let req: super::super::protocol::VdfsWriteRequest = request_of(&args);
                let content = req.to_content();
                let data = p.write(&ctx, &req.path, &content).await?;
                let message = if data.created {
                    format!("已创建文件 {}", req.path)
                } else {
                    format!("已覆盖文件 {}", req.path)
                };
                Ok(serde_json::json!({
                    "success": true,
                    "path": req.path,
                    "created": data.created,
                    "message": message,
                }))
            })
        },
    },
    ToolSpec {
        name: "vdfs_edit",
        description: "编辑文件（精确字符串替换）。old_string 必须在文件中精确匹配一次；自动保持原换行符风格（CRLF/LF）；拒绝编辑符号链接。匹配 0 次或多次均报错。",
        schema: || {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": super::PATH_SHORT },
                    "old_string": { "type": "string", "description": "要查找并替换的文本（必须精确匹配一次）" },
                    "new_string": { "type": "string", "description": "替换为的文本（默认空字符串）" }
                },
                "required": ["path", "old_string"]
            })
        },
        examples: &["{\"path\":\"src/main.rs\",\"old_string\":\"fn old()\",\"new_string\":\"fn new()\"}"],
        risk: CapabilityRiskLevel::Medium,
        behaviors: &[crate::behavior::FILE_CHANGE],
        call: |p, args, ctx| {
            Box::pin(async move {
                super::ensure_required(&args, "path")?;
                let req: super::super::protocol::VdfsEditRequest = request_of(&args);
                let data = p.edit(&ctx, &req.path, &req.old_string, &req.new_string).await?;
                // provider 已给出可读说明（如「内容已为最新」）时优先采用；否则据 replaced 生成
                let message = match data.message {
                    Some(m) if !m.is_empty() => m,
                    _ if data.replaced == 0 => {
                        format!("已编辑 {}: 内容已为最新，无需修改", req.path)
                    }
                    _ => format!("已编辑 {}: 替换了 {} 处", req.path, data.replaced),
                };
                Ok(serde_json::json!({
                    "success": true,
                    "path": req.path,
                    // 无操作（`old_string == new_string`）在结果里是结构化的：
                    // 读侧据此跳过，不必匹配 `message` 的中文措辞。
                    "replaced": data.replaced,
                    "message": message,
                }))
            })
        },
    },
    ToolSpec {
        name: "vdfs_search",
        description: "文件名模式搜索（Glob）。pattern 为文件名 Glob（如 '**/*.rs'、'src/**/*.toml'）；path 为可选搜索基目录（相对工作目录，缺省工作目录根）。仅返回普通文件，目录不返回。",
        schema: || {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "可选搜索基目录（相对工作目录），缺省为工作目录根" },
                    "pattern": { "type": "string", "description": "Glob 模式，如 '**/*.rs'" }
                },
                "required": ["pattern"]
            })
        },
        examples: &[
            "{\"pattern\":\"**/*.rs\"}",
            "{\"pattern\":\"src/**/*.toml\"}",
        ],
        risk: CapabilityRiskLevel::Low,
        behaviors: &[],
        call: |p, args, ctx| {
            Box::pin(async move {
                super::ensure_required(&args, "pattern")?;
                let req: super::super::protocol::VdfsSearchRequest = request_of(&args);
                let data = p.search(&ctx, &req.path, &req.pattern).await?;
                Ok(search_receipt(data))
            })
        },
    },
];

/// `vdfs_search` 的回执封装（结果列表 + 总计 + 截断提示）。
const SEARCH_MAX_RESULTS: usize = 1000;

fn search_receipt(mut data: super::super::protocol::VdfsSearchResult) -> Value {
    let mut results = std::mem::take(&mut data.results);
    let mut truncated = data.truncated;
    if results.len() > SEARCH_MAX_RESULTS {
        truncated = true;
        results.truncate(SEARCH_MAX_RESULTS);
    }
    results.sort();

    let message = if results.is_empty() {
        "未找到匹配的文件。".to_string()
    } else {
        let mut msg = results.join("\n");
        if truncated {
            msg.push_str(&format!(
                "\n\n[结果已截断：显示前 {SEARCH_MAX_RESULTS} 个匹配]"
            ));
        }
        msg.push_str(&format!("\n\n总计: {} 个文件", results.len()));
        msg
    };

    serde_json::json!({
        "results": results,
        "truncated": truncated,
        "message": message,
    })
}
