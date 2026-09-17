use crate::symbio_core::providers::EmbeddingError;
use crate::symbio_core::providers::EmbeddingService;
use crate::symbio_core::{InvokeRequest, EMBEDDING_LOCAL, EMBEDDING_NOOP};
use async_trait::async_trait;
use std::io::Cursor;
use std::sync::{Arc, LazyLock};
use tract_onnx::prelude::*;

const MODEL_BYTES: &[u8] = include_bytes!("model.onnx");
const TOKENIZER_BYTES: &[u8] = include_bytes!("tokenizer.json");

/// 编译产物：`into_runnable()` 的返回（不可变计划，`&self` 并发调用安全）
type CompiledModel = Arc<RunnableModel<TypedFact, Box<dyn TypedOp>>>;

/// 编译好的推理计划 + 分词器（`Arc` 共享，`embed` 只克隆引用计数）
struct Loaded {
    plan: CompiledModel,
    /// 模型声明的输入名次序（ONNX 图里 input_ids / attention_mask / token_type_ids
    /// 的声明顺序不保证，喂张量必须按这次序）
    input_names: Vec<String>,
    tokenizer: tokenizers::Tokenizer,
}

/// 全局单例：懒加载 + 一次性编译（`into_optimized` 含 declutter 与算子融合，秒级开销）
static GLOBAL: LazyLock<Result<Arc<LocalEmbeddingService>, EmbeddingError>> =
    LazyLock::new(LocalEmbeddingService::init);

pub(crate) struct LocalEmbeddingService {
    loaded: Arc<Loaded>,
}

impl LocalEmbeddingService {
    fn get_instance() -> Result<Arc<dyn EmbeddingService>, EmbeddingError> {
        match &*GLOBAL {
            Ok(service) => Ok(service.clone() as Arc<dyn EmbeddingService>),
            Err(e) => Err(EmbeddingError::Init(format!(
                "Local embedding service failed to init: {e}"
            ))),
        }
    }

    fn init() -> Result<Arc<Self>, EmbeddingError> {
        tracing::info!("Initializing local embedding service (tract, pure Rust ONNX)...");

        let tokenizer = tokenizers::Tokenizer::from_bytes(TOKENIZER_BYTES)
            .map_err(|e| EmbeddingError::Init(format!("tokenizer 加载失败: {e}")))?;

        let mut model = tract_onnx::onnx()
            .model_for_read(&mut Cursor::new(MODEL_BYTES))
            .map_err(|e| EmbeddingError::Init(format!("ONNX 模型加载失败: {e}")))?;

        // 符号化 seq 维（S）：batch 恒为 1、token 数可变 ⇒ 一次编译、任意长度复用
        let scope = SymbolScope::default();
        let seq: TDim = scope.sym("S").into();
        let fact = InferenceFact::dt_shape(DatumType::I64, vec![TDim::Val(1), seq]);
        for i in 0..model.inputs.len() {
            model
                .set_input_fact(i, fact.clone())
                .map_err(|e| EmbeddingError::Init(format!("输入维度声明失败: {e}")))?;
        }
        let input_names: Vec<String> = model
            .inputs
            .iter()
            .map(|&o| model.outlet_label(o).unwrap_or_default().to_string())
            .collect();

        let plan = model
            .into_typed()
            .and_then(|m| m.into_optimized())
            .and_then(|m| m.into_runnable())
            .map_err(|e| EmbeddingError::Init(format!("模型编译失败: {e}")))?;

        tracing::info!("Local embedding service ready (pure Rust, no C toolchain).");
        Ok(Arc::new(Self {
            loaded: Arc::new(Loaded {
                plan,
                input_names,
                tokenizer,
            }),
        }))
    }
}

/// 同步推理体：tokenize → 三输入张量 → ONNX(int8) → CLS 池化 → L2 归一化
///
/// 行为与 fastembed 时代逐项对齐：
/// - `encode(text, true)`（= `encode_batch(.., true)`），tokenizer.json 无截断/padding；
/// - CLS 池化（取 last_hidden_state 第 0 个 token）；
/// - 输出做 L2 归一化（spike 实测 fastembed 范数恒为 1.0 ⇒ 它做了这一步）；
/// - 任何一步失败返回 `None`，上层 codebase_search 退化为精确匹配。
fn embed_sync(loaded: &Loaded, text: &str) -> Option<Vec<f32>> {
    let enc = match loaded.tokenizer.encode(text, true) {
        Ok(enc) => enc,
        Err(e) => {
            tracing::warn!("embedding 分词失败: {e}");
            return None;
        }
    };
    let ids: Vec<i64> = enc.get_ids().iter().map(|&x| x as i64).collect();
    let mask: Vec<i64> = enc.get_attention_mask().iter().map(|&x| x as i64).collect();
    let types: Vec<i64> = enc.get_type_ids().iter().map(|&x| x as i64).collect();
    let n = ids.len();
    if n == 0 {
        tracing::warn!("embedding 输入分词后为空");
        return None;
    }

    let mut inputs: TVec<TValue> = TVec::with_capacity(loaded.input_names.len());
    for name in &loaded.input_names {
        let v: &[i64] = match name.as_str() {
            "input_ids" => &ids,
            "attention_mask" => &mask,
            "token_type_ids" => &types,
            other => {
                tracing::warn!("embedding 模型出现未知输入名: {other}");
                return None;
            }
        };
        let arr = match tract_ndarray::Array2::from_shape_vec((1, n), v.to_vec()) {
            Ok(arr) => arr,
            Err(e) => {
                tracing::warn!("embedding 输入张量构造失败: {e}");
                return None;
            }
        };
        inputs.push(arr.into_tvalue());
    }

    let outputs = match loaded.plan.run(inputs) {
        Ok(outputs) => outputs,
        Err(e) => {
            tracing::warn!("embedding 推理失败: {e}");
            return None;
        }
    };
    let first = outputs.into_iter().next()?;
    let tensor = first.into_tensor();
    let hidden = match tensor.shape().last() {
        Some(&h) => h,
        None => {
            tracing::warn!("embedding 输出形状为空");
            return None;
        }
    };
    let data = match tensor.try_as_plain().and_then(|v| v.as_slice::<f32>()) {
        Ok(data) => data,
        Err(e) => {
            tracing::warn!("embedding 输出读取失败: {e}");
            return None;
        }
    };
    if data.len() != n * hidden {
        tracing::warn!(
            "embedding 输出形状异常: len={} 期望={}x{hidden}",
            data.len(),
            n
        );
        return None;
    }

    // CLS 池化 + L2 归一化
    let cls = &data[..hidden];
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
        // 纯 CPU 计算，放阻塞线程池；plan 不可变，`&self` 并发调用安全（无需 Mutex）
        let loaded = self.loaded.clone();
        let text = text.to_string();
        tokio::task::spawn_blocking(move || embed_sync(&loaded, &text))
            .await
            .ok()
            .flatten()
    }
}

pub(crate) struct NoopEmbeddingService;

#[async_trait]
impl EmbeddingService for NoopEmbeddingService {
    async fn embed(&self, _text: &str) -> Option<Vec<f32>> {
        None
    }
}

// === 注册到通用对象创建机制 ===

fn build_local(_ctx: Arc<dyn InvokeRequest>) -> Arc<dyn EmbeddingService> {
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

fn build_noop(_ctx: Arc<dyn InvokeRequest>) -> Arc<dyn EmbeddingService> {
    Arc::new(NoopEmbeddingService)
}

crate::submit_object_creator!(EMBEDDING_LOCAL, build_local, dyn EmbeddingService);
crate::submit_object_creator!(EMBEDDING_NOOP, build_noop, dyn EmbeddingService);

#[cfg(test)]
#[path = "local.test.rs"]
mod tests;
