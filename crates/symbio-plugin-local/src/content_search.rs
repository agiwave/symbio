//! 内容搜索工具 —— 工具端口执行体（`PortExecutor`），使用正则表达式搜索文件内容
//!
//! 直接调用 ripgrep 库（grep / grep-searcher / grep-regex / ignore）完成跨平台搜索，
//! 不依赖 rg / PowerShell / grep 等外部可执行文件，各操作系统行为一致，并遵循 .gitignore。

use super::policy::SecurityPolicy;
use super::system::validate_params;
use async_trait::async_trait;
use bstr::ByteSlice;
use grep::matcher::Matcher;
use grep::regex::RegexMatcherBuilder;
use grep::searcher::sinks::UTF8 as UTF8Sink;
use grep::searcher::SearcherBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use symbio_plugin_api::{
    missing_caller, port_result, CapabilityMeta, CapabilityRiskLevel, DynVdfsProvider, ExecEnv,
    PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PortExecContext, PortExecutor, PortInput, PortOutcome, VdfsContent, VdfsContext, VdfsRequest,
    VdfsResponse,
};

const MAX_RESULTS: usize = 1000;
const MAX_OUTPUT_BYTES: usize = 1_048_576;

/// 递归列举作用域内的文件，返回相对地址（名字序、深度优先）。
///
/// 替代原来的 `WalkBuilder`：插件不直接读盘，而是经 `WORKSPACE_VDFS` 的
/// `List` 逐层展开（`ScopeDirVdfs` 只接受相对地址并做逃逸守卫）。复刻了
/// `standard_filters(true)` 的两个关键语义——隐藏文件（`.` 开头）与
/// `.gitignore`（每层可叠加，子目录优先）。排序固定为名字序，与原来的
/// `sort_by_file_name` 一致，保证 `offset` / `head_limit` 窗口跨平台稳定。
fn collect_files<'a>(
    provider: &'a DynVdfsProvider,
    vctx: &'a VdfsContext,
    dir: &'a str,
    ignores: &'a mut Vec<Gitignore>,
    out: &'a mut Vec<String>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>> {
    Box::pin(async move {
        let resp = provider
            .dispatch(vctx, dir, VdfsRequest::List {})
            .await
            .map_err(|e| format!("列举目录失败: {e}"))?;
        let VdfsResponse::List(nodes) = resp else {
            return Ok(());
        };
        let mut nodes: Vec<_> = nodes.into_iter().collect();
        nodes.sort_by(|a, b| a.node.name.cmp(&b.node.name));
        for node in nodes {
            let is_dir = node.node.is_dir();
            let name = node.node.name;
            if name.starts_with('.') {
                continue;
            }
            let rel = if dir.is_empty() {
                name.clone()
            } else {
                format!("{dir}/{name}")
            };
            if ignores
                .iter()
                .any(|ig| ig.matched(Path::new(&name), is_dir).is_ignore())
            {
                continue;
            }
            if is_dir {
                let pushed = load_ignore(provider, vctx, &rel).await;
                let has_pushed = pushed.is_some();
                if let Some(ig) = pushed {
                    ignores.push(ig);
                }
                collect_files(provider, vctx, &rel, ignores, out).await?;
                if has_pushed {
                    ignores.pop();
                }
            } else {
                out.push(rel);
            }
        }
        Ok(())
    })
}

/// 读某层的 `.gitignore` 并编译为 matcher；没有文件就是「不忽略」。
async fn load_ignore(
    provider: &DynVdfsProvider,
    vctx: &VdfsContext,
    dir: &str,
) -> Option<Gitignore> {
    let path = if dir.is_empty() {
        ".gitignore".to_string()
    } else {
        format!("{dir}/.gitignore")
    };
    let VdfsResponse::Read(content) = provider
        .dispatch(vctx, &path, VdfsRequest::Read)
        .await
        .ok()?
    else {
        return None;
    };
    let text = content.text?;
    let mut builder = GitignoreBuilder::new("");
    for line in text.lines() {
        builder.add_line(None, line).ok()?;
    }
    builder.build().ok()
}

/// 从 VDFS 读到的正文取出字节：文本直接用，二进制按 base64 解码。
fn content_bytes(content: VdfsContent) -> Option<Vec<u8>> {
    if let Some(text) = content.text {
        return Some(text.into_bytes());
    }
    let encoded = content.b64?;
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()
}

/// 内容搜索工具
///
/// 持有 `SecurityPolicy` 做**用户配置的路径策略**判定（禁止路径等）——provider
/// 只保证「不逃逸作用域」，不实现用户自定义的 `forbidden_paths` 等规则，这一层
/// 不能少。但路径本身是 VDFS 相对地址（不是物理路径），逃逸守卫在 provider 的
/// `resolve` / `guard_read` 里做。
#[derive(Clone)]
pub struct ContentSearchTool {
    security: Arc<SecurityPolicy>,
}

impl ContentSearchTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    async fn execute_inner(
        &self,
        args: &Value,
        provider: Option<&DynVdfsProvider>,
    ) -> PluginInvokeResponse<Value> {
        validate_params(args, &["pattern"]).map_err(PluginError::ValidationError)?;

        let pattern = match args.get("pattern").and_then(|v| v.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => {
                return Err(PluginError::ValidationError(
                    "Missing or empty 'pattern' argument".into(),
                ))
            }
        };

        if pattern.is_empty() {
            return Err(PluginError::ValidationError(
                "不允许使用空模式。".to_string(),
            ));
        }

        let search_path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let output_mode = args
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("content");
        let include = args.get("include").and_then(|v| v.as_str());
        let case_sensitive = args
            .get("case_sensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let context_before = args
            .get("context_before")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let context_after = args
            .get("context_after")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let type_filter = args.get("type").and_then(|v| v.as_str());
        let multiline = args
            .get("multiline")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let head_limit = args.get("head_limit").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;

        if !matches!(output_mode, "content" | "files_with_matches" | "count") {
            return Err(PluginError::ValidationError(format!(
                "无效的 output_mode '{output_mode}'。允许值: content, files_with_matches, count。"
            )));
        }

        let provider = provider.ok_or_else(|| {
            PluginError::ValidationError("当前上下文没有工作区作用域，无法搜索文件内容".into())
        })?;
        // 作用域只认相对地址（拒绝绝对路径与 `..` 父段，见 ScopeDirVdfs::resolve），
        // 由 provider 统一做逃逸守卫；插件不再自行 canonicalize / 拼物理路径。
        let rel_root = search_path.trim_start_matches("./").trim_end_matches('/');
        // 用户配置的路径策略（forbidden_paths / workspace_only 等）仍需生效：provider
        // 只管「不逃逸作用域」，不管「用户禁止读哪一段」。这里按相对地址判定。
        if !self
            .security
            .is_path_allowed_for_read(rel_root, std::path::Path::new(""))
            .await
        {
            return Err(PluginError::ValidationError(
                "搜索路径被安全策略拒绝。".into(),
            ));
        }
        let vctx = VdfsContext::empty();

        match self
            .search(
                pattern,
                provider,
                &vctx,
                rel_root,
                output_mode,
                include,
                type_filter,
                case_sensitive,
                context_before,
                context_after,
                multiline,
                offset,
                head_limit,
            )
            .await
        {
            Ok(result) => {
                let truncated = result.len() > MAX_OUTPUT_BYTES;
                let result = if truncated {
                    // 安全截断：搜索结果正文常含中文，裸 `&[..N]` 会 panic 在字符边界内
                    format!(
                        "{}...\n\n[输出已截断：超过 {} 字节]",
                        symbio_plugin_api::text_truncate_bytes(&result, MAX_OUTPUT_BYTES, ""),
                        MAX_OUTPUT_BYTES
                    )
                } else {
                    result
                };

                Ok(json!({
                    "output": result,
                    "truncated": truncated,
                    "backend": "ripgrep-lib"
                }))
            }
            Err(e) => Err(PluginError::InternalError(e)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn search(
        &self,
        pattern: &str,
        provider: &DynVdfsProvider,
        vctx: &VdfsContext,
        rel_root: &str,
        output_mode: &str,
        include: Option<&str>,
        file_type: Option<&str>,
        case_sensitive: bool,
        context_before: usize,
        context_after: usize,
        multiline: bool,
        offset: usize,
        head_limit: usize,
    ) -> Result<String, String> {
        // 编译正则：多行模式开启 dot-matches-newline（对齐 rg -U）
        let mut rb = RegexMatcherBuilder::new();
        rb.case_smart(false)
            .case_insensitive(!case_sensitive)
            .multi_line(true)
            .dot_matches_new_line(multiline)
            .octal(false);
        let matcher = rb
            .build(pattern)
            .map_err(|e| format!("无效的正则表达式: {e}"))?;

        // 文件名过滤：显式 include(glob) + type(ripgrep 类型名) 经 globset 合并
        let mut globs: Vec<String> = Vec::new();
        if let Some(g) = include {
            globs.push(g.to_string());
        }
        if let Some(ty) = file_type {
            for g in type_to_globs(ty) {
                globs.push((*g).to_string());
            }
        }
        // glob 过滤：编译一次，遍历时对 VDFS 相对地址匹配（原来的 filter_entry
        // 拿的是物理路径，这里换成相对地址——语义等价，且不再泄漏物理路径）。
        let glob_set = if globs.is_empty() {
            None
        } else {
            let mut builder = globset::GlobSetBuilder::new();
            for g in &globs {
                builder.add(
                    globset::GlobBuilder::new(g)
                        .literal_separator(false)
                        .build()
                        .map_err(|e| format!("无效的 include/glob '{g}': {e}"))?,
                );
            }
            Some(
                builder
                    .build()
                    .map_err(|e| format!("构建 glob 失败: {e}"))?,
            )
        };

        // 经作用域 provider 递归列举文件：不读物理盘，逃逸由 provider 守卫。
        // 名字序在 collect_files 里钉死，保证 offset / head_limit 窗口跨平台稳定。
        let mut files = Vec::new();
        let mut ignores = Vec::new();
        if let Some(ig) = load_ignore(provider, vctx, "").await {
            ignores.push(ig);
        }
        collect_files(provider, vctx, rel_root, &mut ignores, &mut files).await?;

        let mut searcher = SearcherBuilder::new().line_number(true).build();

        // 收集窗口：`offset` 跳过前 N 行输出，`head_limit`（缺省 `MAX_RESULTS`）决定一段给多少行。
        // 窗口只约束**收集**、不约束**扫描**：走完全程才拿得到真实总数（`match_count` /
        // `line_index`）。截断处只说「限制了」不说「一共多少」，聚合型问题（「有多少处
        // `unwrap()`」）就必然逼出一次原样重发。
        let window = if head_limit > 0 {
            head_limit
        } else {
            MAX_RESULTS
        };
        let mut out: Vec<String> = Vec::new();
        let mut line_index: usize = 0;
        let mut match_count: u64 = 0;

        macro_rules! emit_line {
            ($text:expr) => {{
                let at = line_index;
                line_index += 1;
                if at >= offset && out.len() < window {
                    out.push($text);
                }
            }};
        }

        for rel in files {
            if let Some(set) = &glob_set {
                if !set.is_match(Path::new(&rel)) {
                    continue;
                }
            }
            // 展示路径用作用域相对地址（VDFS 视图内唯一），不再有 Windows 扩展前缀。
            let disp = if rel_root.is_empty() {
                rel.clone()
            } else {
                format!("{rel_root}/{rel}")
            };

            // 经 provider 读取正文；二进制以 b64 返回，解码后同样可搜。
            let Some(VdfsResponse::Read(read)) =
                provider.dispatch(vctx, &rel, VdfsRequest::Read).await.ok()
            else {
                continue;
            };
            let Some(content) = content_bytes(read) else {
                continue;
            };

            if multiline {
                // 跨行：整文件读入后用 Matcher 手动迭代匹配（'.' 已开启匹配换行）
                let haystack = bstr::BString::from(content);
                let mut at = 0usize;
                let mut file_matches: Vec<String> = Vec::new();
                loop {
                    let m = matcher
                        .find_at(&haystack, at)
                        .map_err(|e| format!("匹配失败: {e}"))?;
                    let m = match m {
                        Some(m) => m,
                        None => break,
                    };
                    file_matches.push(haystack[m.start()..m.end()].to_str_lossy().into_owned());
                    at = m.end();
                    if m.start() == m.end() {
                        at += 1;
                    }
                    if at >= haystack.len() {
                        break;
                    }
                }
                if !file_matches.is_empty() {
                    if output_mode == "files_with_matches" {
                        emit_line!(disp.to_string());
                    } else if output_mode == "count" {
                        emit_line!(format!("{}:{}", disp, file_matches.len()));
                    } else {
                        emit_line!(format!("{}", disp));
                        for m in &file_matches {
                            emit_line!(m.clone());
                        }
                    }
                    match_count += file_matches.len() as u64;
                }
            } else {
                // 逐行搜索：收集匹配行号与文本
                let mut file_matches: Vec<(u64, String)> = Vec::new();
                let sink = UTF8Sink(|ln: u64, text: &str| {
                    // `UTF8Sink` 交给闭包的 `text` **含行终止符**（`SinkMatch` 语义）：
                    // 原样入列会让 `join("\n")` 后每个匹配行间多一个空行，且
                    // `apply_line_pagination` 按**输出行**切分页 ⇒ 与匹配行错位
                    // （`head_limit=2` 只覆盖 1 个匹配）。在入口剥掉终止符。
                    file_matches.push((ln, text.trim_end_matches(['\n', '\r']).to_string()));
                    Ok(true)
                });
                if searcher.search_slice(&matcher, &content, sink).is_err() {
                    continue;
                }
                if file_matches.is_empty() {
                    continue;
                }
                match_count += file_matches.len() as u64;

                if output_mode == "files_with_matches" {
                    emit_line!(disp.to_string());
                } else if output_mode == "count" {
                    emit_line!(format!("{}:{}", disp, file_matches.len()));
                } else if context_before == 0 && context_after == 0 {
                    for (ln, text) in &file_matches {
                        emit_line!(format!("{}:{}:{}", disp, ln, text));
                    }
                } else {
                    // 手动上下文窗口：用已读入的正文，合并匹配 ± 上下文行号
                    let owned = bstr::BString::from(content);
                    let line_vec: Vec<&[u8]> = owned.lines().collect();
                    let n = line_vec.len();
                    let mut show: BTreeSet<usize> = BTreeSet::new();
                    for (ln, _) in &file_matches {
                        let l = *ln as usize;
                        let start = l.saturating_sub(context_before);
                        let end = (l + context_after).min(n);
                        for i in start..=end {
                            show.insert(i);
                        }
                    }
                    for i in show {
                        if i >= n {
                            continue;
                        }
                        let text = line_vec[i].to_str_lossy().into_owned();
                        emit_line!(format!("{}:{}:{}", disp, i + 1, text));
                    }
                }
            }
        }

        // 尾部读数：说清「一共多少、给了哪一段、剩下的怎么拿」。
        let given = out.len();
        let next_offset = offset + given;
        let withheld = line_index.saturating_sub(next_offset);
        if given > 0 && withheld > 0 {
            out.push(format!(
                "[结果已分页：共 {match_count} 处命中、{line_index} 行输出；本次给出第 {}–{next_offset} 行（{given} 行），其后还有 {withheld} 行。取后续用 offset={next_offset}（head_limit 控每页行数）；只问总数用 output_mode=\"count\"；或按 path/include/type 收窄后重查。整份结果超预算时由系统存档，取回见结果中的存档指引。]",
                offset + 1
            ));
        } else if given == 0 && line_index > 0 {
            out.push(format!(
                "[窗口越过末尾：共 {match_count} 处命中、{line_index} 行输出，offset={offset} 已超出最后一行（第 {line_index} 行）；从 offset=0 起取。]"
            ));
        }

        if out.is_empty() {
            Ok("未找到匹配。".to_string())
        } else {
            Ok(out.join("\n"))
        }
    }
}

/// 将 ripgrep 风格的文件类型名映射为文件名 glob（用于按类型过滤）。
/// 覆盖常见类型；未知类型返回空切片（不过滤）。
fn type_to_globs(ty: &str) -> &'static [&'static str] {
    match ty.to_ascii_lowercase().as_str() {
        "rust" => &["*.rs"],
        "python" | "py" => &["*.py"],
        "js" | "javascript" => &["*.js", "*.jsx", "*.mjs", "*.cjs"],
        "ts" | "typescript" => &["*.ts", "*.tsx"],
        "toml" => &["*.toml"],
        "json" => &["*.json"],
        "markdown" | "md" => &["*.md", "*.markdown"],
        "html" => &["*.html", "*.htm"],
        "css" => &["*.css"],
        "scss" => &["*.scss"],
        "java" => &["*.java"],
        "go" => &["*.go"],
        "c" => &["*.c", "*.h"],
        "cpp" | "c++" => &["*.cpp", "*.cc", "*.cxx", "*.hpp", "*.hxx"],
        "ruby" | "rb" => &["*.rb"],
        "php" => &["*.php"],
        "sh" | "bash" | "shell" => &["*.sh"],
        "yaml" | "yml" => &["*.yml", "*.yaml"],
        "xml" => &["*.xml"],
        "sql" => &["*.sql"],
        "swift" => &["*.swift"],
        "kotlin" | "kt" => &["*.kt", "*.kts"],
        "r" => &["*.r"],
        "lua" => &["*.lua"],
        "make" => &["Makefile", "*.mk"],
        "log" => &["*.log"],
        "txt" => &["*.txt"],
        "csv" => &["*.csv"],
        _ => &[],
    }
}

impl ContentSearchTool {
    pub fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: "content_search".to_string(),
            description:
                "文件内容搜索（正则表达式）。跨平台，直接调用 ripgrep 库，遵循 .gitignore。"
                    .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "正则表达式模式"
                    },
                    "path": {
                        "type": "string",
                        "description": "搜索目录（默认当前目录）"
                    },
                    "output_mode": {
                        "type": "string",
                        "enum": ["content", "files_with_matches", "count"],
                        "description": "输出模式"
                    },
                    "include": {
                        "type": "string",
                        "description": "文件过滤模式（glob，如 '*.rs'）"
                    },
                    "case_sensitive": {
                        "type": "boolean",
                        "description": "是否区分大小写"
                    },
                    "type": {
                        "type": "string",
                        "description": "文件类型过滤（ripgrep 类型名，如 rust / python / js；按文件名 glob 等效过滤）"
                    },
                    "multiline": {
                        "type": "boolean",
                        "description": "是否跨行匹配（'.' 匹配换行）"
                    },
                    "context_before": {
                        "type": "integer",
                        "description": "匹配前显示的上下文行数"
                    },
                    "context_after": {
                        "type": "integer",
                        "description": "匹配后显示的上下文行数"
                    },
                    "head_limit": {
                        "type": "integer",
                        "description": "本次返回的最大行数（0 表示用默认上限 1000 行；对齐 Trae Grep 的 head_limit）"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "跳过前 N 行再返回（翻页取回被截断段落；结果尾部的分页读数会直接给出下一个 offset 与命中总数）"
                    }
                },
                "required": ["pattern"]
            }),
            category: Some(symbio_plugin_api::CapabilityCategory::FileOperation),
            risk: Some(CapabilityRiskLevel::Low),
            examples: Some(vec![
                "pattern='fn main'".to_string(),
                "pattern='TODO', include='*.rs'".to_string(),
            ]),
            ..Default::default()
        }
    }

    async fn run(
        &self,
        args: Value,
        _env: &ExecEnv,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        let provider = ctx.get(symbio_plugin_api::WORKSPACE_VDFS).flatten();
        let data = self.execute_inner(&args, provider.as_ref()).await?;
        Ok(serde_json::to_value(&data)?)
    }
}

#[async_trait]
impl PortExecutor for ContentSearchTool {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let Some(ctx) = exec.ctx else {
            return missing_caller();
        };
        let env = ExecEnv::from_request(&*ctx);
        port_result(self.run(input.payload, &env, ctx).await)
    }
}

#[cfg(test)]
#[path = "content_search.test.rs"]
mod tests;
