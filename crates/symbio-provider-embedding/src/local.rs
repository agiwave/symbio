//! 本地嵌入服务：`ort`（ONNX Runtime）跑 int8 量化的 bge-small-zh-v1.5。
//!
//! ## 为什么不是 `tract`（纯 Rust ONNX）
//!
//! 2026-09-18 实测（本机，临时探针，跑完即删）：
//!
//! | 输入 | token | debug | release |
//! |---|---|---|---|
//! | 305 字符 | 127 | 2.88 s | — |
//! | 610 字符 | 252 | 5.46 s | — |
//! | 1250 字符 | 502 | 11.34 s | — |
//! | 2530 字符（= `CHUNK_LINES` 40 行） | 1002 → 截 512 | 11.77 s | **9.86 s** |
//!
//! 即 **每 token ≈ 22.5 ms、纯线性**，512 token 封顶后恒定 ~10 s/块。而
//! `codebase_search` 要给整个工作区分块嵌入（本仓库 483 文件 / 43,151 行 ≈ 2158 块，
//! 且 `CHUNK_STEP=20` 对 `CHUNK_LINES=40` 是 50% 重叠），**全量建索引 ≈ 6 小时**，
//! 远超工具 600 s 的硬超时——语义检索在实现上等不到结果。
//!
//! `tract` 不是"没优化好"：release 只比 debug 快 16%，是**结构性**慢（无 ORT 级的
//! 算子融合与多线程 GEMM）。D-014 当初写的"离线索引 + 单条查询，秒级可接受"
//! 对**查询**成立（短查询 63 ms），对**建索引**不成立——那个前提已被实测证伪。
//!
//! ## 为什么不是 `fastembed`
//!
//! `fastembed` 硬编码 `tokenizers/onig`（关不掉）⇒ `onig_sys` ⇒ **真正的 C 编译链**，
//! 这正是当初弃用它、转 `tract` 的理由。但 `fastembed` 不可替代的部分其实只有
//! "调 ONNX Runtime"一件事——分词、张量构造、CLS 池化、L2 归一化我们都自己做。
//! 因此**直接依赖 `ort`**：拿到同样的推理速度，而 `onig_sys` 那条链不必回来。
//!
//! ## 零 C/C++ 编译仍然成立——但预编译产物不便宜
//!
//! - `ort` 的默认 features 走 `tls-native`（Windows 为 schannel），**不拉 `ring`**。
//!   ⚠️ 不要开 `tls-rustls`——那会把 `ring` 的 C/汇编链拉回来。
//! - `ort-sys` 的 ONNX Runtime 是 **build 期下载预编译二进制**，不是编译源码；
//!   `cargo tree -i cc / ring / onig_sys` 三者皆空。
//! - **代价要写清楚**：Windows x64 拿到的是 **DirectML flavour**
//!   （`BinariesSource::Pyke`，源码注释："pyke libs always ship compiled with DirectML on
//!   Windows"，无法用 features 关掉）⇒ 构建缓存落 341 MB `onnxruntime.lib` + 18 MB
//!   `DirectML.dll`，且 exe **静态导入 `DirectML.dll`**（已用 PE 导入表核对，不是延迟加载）。
//!   好在 Win10 1903+ / Win11 由系统提供该 DLL——实测删掉随包那份仍能推理。
//!   离线构建会静默退化为「不链接」，到链接期才报 `undefined symbol: OrtGetApiBase`。
//! - 分词继续用 `tokenizers` + `fancy-regex`（纯 Rust 正则后端，无 onig）。
//!
//! 完整的取舍与实测数据见 D-016。
//!
//! ## 数值基线
//!
//! 与 `tract` 时代同一份模型 + 同一套前后处理（`encode(text, true)`、CLS 池化、
//! L2 归一化），余弦相似度见 D-015 的实测值。归一化后余弦对尺度不变，因此
//! 换推理器不影响"谁和谁更相似"这个**排序**结论。
//!
//! ## 与 `tract` 版本的三处实现差异（都是 `ort` 的接口约束，不是选择）
//!
//! 1. **`Session::run` 取 `&mut self`**（`tract` 的计划是 `&self`）⇒ 必须 `Mutex` 包一层。
//!    锁只在 `spawn_blocking` 闭包内持有，**不跨 `.await` 边界**，故用 `std::sync::Mutex`
//!    即可，不会阻塞 tokio worker。
//! 2. **输入名从 session 读**（`session.inputs()`），不硬编码。图里声明的输入名不保证
//!    是 `input_ids` / `attention_mask` / `token_type_ids`，硬编码会在换模型时静默错。
//! 3. **输出按 rank 选**（取第一个 rank == 3 的输出），不按位置。BERT 导出通常有
//!    `last_hidden_state`（rank 3）与 `pooler_output`（rank 2），靠顺序猜会在导出变体上错。
//!    顺带：`tract` 时代为绕开形状推断而写的"符号维 / 固定 seq 双策略"整套兜底
//!    （`fixed_seq` / `pad_or_truncate`）随之删除——ORT 原生支持动态形状。

use async_trait::async_trait;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use symbio_core::EmbeddingError;
use symbio_core::EmbeddingService;
use symbio_core::{PluginInvokeRequest, EMBEDDING_LOCAL, EMBEDDING_NOOP};

const MODEL_BYTES: &[u8] = include_bytes!("model.onnx");
const TOKENIZER_BYTES: &[u8] = include_bytes!("tokenizer.json");

/// bge-small-zh-v1.5 的最大位置数。`tokenizer.json` 不做截断，超长在此兜底
/// （位置嵌入表只有这么多项，超出会让 position gather 越界）。
const MAX_SEQ: usize = 512;

/// 建好的推理会话 + 分词器（`Arc` 共享，`embed` 只克隆引用计数）
struct Loaded {
    /// `run` 取 `&mut self`，故需互斥；锁不跨 `.await`（见模块文档）
    session: Mutex<Session>,
    tokenizer: tokenizers::Tokenizer,
}

pub struct LocalEmbeddingService {
    loaded: Arc<Loaded>,
}

/// 全局单例状态：成功实例 + 失败退避。
///
/// **失败不缓存为永久**：`LazyLock<Result<..>>` 会把一次失败钉死整个进程，
/// 而 ONNX Runtime 的初始化失败可能是环境瞬态（临时缺库 / 内存紧张）——永久降级
/// 会让「修好了也用不上」。改成「失败记下原因 + 退避窗口」，窗口过后下一次取用重试。
static GLOBAL: OnceLock<Mutex<InitState>> = OnceLock::new();

/// 初始化失败后的退避窗口：一次建会话是 0.2s 量级，坏环境下每次检索都重试一次
/// 会把一条慢路径变成一把锤子。
const INIT_RETRY_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Default)]
struct InitState {
    service: Option<Arc<LocalEmbeddingService>>,
    gate: RetryGate,
}

/// 初始化失败的退避状态（纯数据；判据在 `local.test.rs`）。
#[derive(Default)]
struct RetryGate {
    last_error: Option<String>,
    next_retry_at: Option<Instant>,
}

impl RetryGate {
    /// 仍在退避窗口内 ⇒ 返回「为什么还不可用」；`None` = 可以再试一次。
    fn blocked(&self, now: Instant) -> Option<String> {
        let at = self.next_retry_at?;
        if now >= at {
            return None;
        }
        // 向上取整到秒：不足 1s 显示「0s 后重试」会让人以为立刻可用。
        let secs = ((at - now).as_millis().saturating_add(999) / 1000).max(1);
        Some(format!(
            "上次初始化失败（{}），{}s 后重试",
            self.last_error.as_deref().unwrap_or("未知原因"),
            secs
        ))
    }

    fn record_failure(&mut self, now: Instant, error: String) {
        self.last_error = Some(error);
        self.next_retry_at = Some(now + INIT_RETRY_BACKOFF);
    }

    fn record_success(&mut self) {
        self.last_error = None;
        self.next_retry_at = None;
    }
}

fn init_state() -> &'static Mutex<InitState> {
    GLOBAL.get_or_init(|| Mutex::new(InitState::default()))
}

/// 语义检索当前为何不可用（可用 / 尚未尝试 ⇒ `None`）。
///
/// 这是给**工具返回体**用的：`codebase_search` 降级到关键词检索时要把原因说出去，
/// 否则「语义检索不可用」与「语义检索没搜到」在用户眼里同形。
pub fn unavailable_reason() -> Option<String> {
    let state = init_state()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if state.service.is_some() {
        return None;
    }
    state
        .gate
        .last_error
        .as_ref()
        .map(|e| format!("{e}（已启用退避重试）"))
}

impl LocalEmbeddingService {
    fn get_instance() -> Result<Arc<dyn EmbeddingService>, EmbeddingError> {
        let mut state = init_state()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(service) = &state.service {
            return Ok(service.clone() as Arc<dyn EmbeddingService>);
        }
        let now = Instant::now();
        if let Some(reason) = state.gate.blocked(now) {
            return Err(EmbeddingError::Init(format!(
                "Local embedding service failed to init: {reason}"
            )));
        }
        match Self::init() {
            Ok(service) => {
                state.service = Some(service.clone());
                state.gate.record_success();
                Ok(service)
            }
            Err(e) => {
                state.gate.record_failure(now, e.to_string());
                Err(e)
            }
        }
    }

    fn init() -> Result<Arc<Self>, EmbeddingError> {
        tracing::info!("Initializing local embedding service (ONNX Runtime via `ort`)...");

        let tokenizer = tokenizers::Tokenizer::from_bytes(TOKENIZER_BYTES)
            .map_err(|e| EmbeddingError::Init(format!("tokenizer 加载失败: {e}")))?;

        // Level3 = 全部图优化（ORT 的默认值，显式写出以固化意图：冷启动一次性开销，
        // 换来每次推理都更快的融合图）。
        //
        // `ort` 的 builder 是**消费式**的（`with_optimization_level(mut self)`），但出错时
        // 把 builder 装进错误里还回来（`Error<SessionBuilder>`），所以不能用 `and_then`
        // 串起来——那要求两侧的 `E` 相同。只能一步步 `?`，每步各自转成本 crate 的错误。
        let builder = Session::builder()
            .map_err(|e| EmbeddingError::Init(format!("ONNX Runtime 初始化失败: {e}")))?;
        let mut builder = builder
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| EmbeddingError::Init(format!("设置图优化级别失败: {e}")))?;
        let session = builder
            .commit_from_memory(MODEL_BYTES)
            .map_err(|e| EmbeddingError::Init(format!("ONNX Runtime 建会话失败: {e}")))?;

        tracing::info!(
            "Local embedding service ready（输入 {} 个：{:?}）",
            session.inputs().len(),
            session
                .inputs()
                .iter()
                .map(|i| i.name())
                .collect::<Vec<_>>()
        );
        Ok(Arc::new(Self {
            loaded: Arc::new(Loaded {
                session: Mutex::new(session),
                tokenizer,
            }),
        }))
    }
}

/// 同步推理体：tokenize → 三输入张量 → ONNX(int8) → CLS 池化 → L2 归一化
///
/// 行为与 `tract` / `fastembed` 时代逐项对齐：
/// - `encode(text, true)`（`tokenizer.json` 无截断/padding，超长在此截到 `MAX_SEQ`）；
/// - CLS 池化（取 `last_hidden_state` 第 0 个 token）；
/// - 输出做 L2 归一化；
/// - 任何一步失败返回 `None`，上层 `codebase_search` 退化为精确匹配。
fn embed_sync(loaded: &Loaded, text: &str) -> Option<Vec<f32>> {
    let enc = match loaded.tokenizer.encode(text, true) {
        Ok(enc) => enc,
        Err(e) => {
            tracing::warn!("embedding 分词失败: {e}");
            return None;
        }
    };
    let ids_raw: Vec<i64> = enc.get_ids().iter().map(|&x| x as i64).collect();
    let mask_raw: Vec<i64> = enc.get_attention_mask().iter().map(|&x| x as i64).collect();
    let types_raw: Vec<i64> = enc.get_type_ids().iter().map(|&x| x as i64).collect();
    let raw_len = ids_raw.len();
    if raw_len == 0 {
        tracing::warn!("embedding 输入分词后为空");
        return None;
    }
    // 动态形状：截到 MAX_SEQ，用真实分词长度（不再补零——那是 tract 定长计划的兜底）
    let n = raw_len.min(MAX_SEQ);
    let ids = &ids_raw[..n];
    let mask = &mask_raw[..n];
    let types = &types_raw[..n];

    let mut session = match loaded.session.lock() {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("embedding 会话锁中毒: {e}");
            return None;
        }
    };

    // 输入名取自 session（图里声明了什么就用什么），不硬编码
    let names: Vec<String> = session
        .inputs()
        .iter()
        .map(|i| i.name().to_string())
        .collect();
    let mut inputs: Vec<(String, SessionInputValue<'_>)> = Vec::with_capacity(names.len());
    for name in names {
        let values: &[i64] = match name.as_str() {
            "input_ids" => ids,
            "attention_mask" => mask,
            "token_type_ids" => types,
            other => {
                tracing::warn!("embedding 模型出现未知输入名: {other}");
                return None;
            }
        };
        let tensor = match Tensor::from_array(([1_i64, n as i64], values.to_vec())) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("embedding 输入张量构造失败: {e}");
                return None;
            }
        };
        inputs.push((name, tensor.into()));
    }

    let outputs = match session.run(inputs) {
        Ok(outputs) => outputs,
        Err(e) => {
            tracing::warn!("embedding 推理失败: {e}");
            return None;
        }
    };

    // 取第一个 rank == 3 的输出（`last_hidden_state`），而不是"第一个输出"：
    // BERT 导出通常同时有 `pooler_output`（rank 2），靠顺序猜会在导出变体上静默取错。
    //
    // 数据在循环内就 `to_vec()` 拷出来：`ValueRef` 经 `Deref` 到 `Value` 上的
    // `try_extract_tensor` 返回的切片借的是**循环变量本身**（不是 `outputs`），
    // 出了这一轮迭代就悬空。512×512 f32 = 1 MiB 的一次拷贝，相对推理性耗时可忽略。
    let mut hidden: Option<(usize, usize, Vec<f32>)> = None;
    for (name, value) in outputs.iter() {
        match value.try_extract_tensor::<f32>() {
            Ok((shape, data)) if shape.len() == 3 => {
                tracing::debug!("embedding 选用输出 {name} {shape}");
                hidden = Some((shape[0] as usize, shape[2] as usize, data.to_vec()));
                break;
            }
            Ok((shape, _)) => tracing::debug!("embedding 跳过非 rank-3 输出 {name}: {shape}"),
            Err(e) => tracing::warn!("embedding 输出读取失败 {name}: {e}"),
        }
    }
    let Some((batch, hidden_dim, data)) = hidden else {
        tracing::warn!("embedding 没有 rank-3 输出（不是预期的 BERT 导出？）");
        return None;
    };
    if batch != 1 || data.len() != n * hidden_dim {
        tracing::warn!(
            "embedding 输出形状异常: batch={batch} len={} 期望=1x{}x{hidden_dim}",
            data.len(),
            n
        );
        return None;
    }

    // CLS 池化 + L2 归一化
    let cls = &data[..hidden_dim];
    let norm = cls.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        tracing::warn!("embedding 向量范数为 0");
        return None;
    }
    Some(cls.iter().map(|x| x / norm).collect())
}

#[async_trait]
impl EmbeddingService for LocalEmbeddingService {
    async fn embed(&self, text: &str) -> Option<Vec<f32>> {
        // 纯 CPU 计算，放阻塞线程池；锁在闭包内取得并释放（不跨 await）
        let loaded = self.loaded.clone();
        let text = text.to_string();
        tokio::task::spawn_blocking(move || embed_sync(&loaded, &text))
            .await
            .ok()
            .flatten()
    }
}

pub struct NoopEmbeddingService;

#[async_trait]
impl EmbeddingService for NoopEmbeddingService {
    async fn embed(&self, _text: &str) -> Option<Vec<f32>> {
        None
    }
}

// === 注册到通用对象创建机制 ===

fn build_local(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn EmbeddingService> {
    match LocalEmbeddingService::get_instance() {
        Ok(svc) => svc,
        Err(e) => {
            // 使用醒目警告格式，明确说明语义搜索将被禁用
            tracing::warn!(
                "⚠️ LocalEmbeddingService unavailable ({}). Falling back to NoopEmbeddingService. \
                ⚠️ Semantic search will be DISABLED. The agent will only support exact-match queries.",
                e
            );
            Arc::new(NoopEmbeddingService) as Arc<dyn EmbeddingService>
        }
    }
}

fn build_noop(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn EmbeddingService> {
    Arc::new(NoopEmbeddingService)
}

symbio_sdk::submit_object_creator!(EMBEDDING_LOCAL, build_local, dyn EmbeddingService);
symbio_sdk::submit_object_creator!(EMBEDDING_NOOP, build_noop, dyn EmbeddingService);

#[cfg(test)]
#[path = "local.test.rs"]
mod tests;
