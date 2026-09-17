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

/// bge-small-zh-v1.5 的最大位置数。`tokenizer.json` 不做截断，超长在此兜底
const MAX_SEQ: usize = 512;

/// 编译好的推理计划 + 分词器（`Arc` 共享，`embed` 只克隆引用计数）
struct Loaded {
    plan: CompiledModel,
    /// 模型声明的输入名次序（ONNX 图里 input_ids / attention_mask / token_type_ids
    /// 的声明顺序不保证，喂张量必须按这次序）
    input_names: Vec<String>,
    tokenizer: tokenizers::Tokenizer,
    /// `None` ⇒ 沿用模型声明的符号维，任意 token 数复用同一计划（首选）；
    /// `Some(n)` ⇒ 编译期定长 n，推理前必须补零/截断到 n（兜底路径）。
    ///
    /// 见 `build_plan` 的说明：两种形态 tract 都能编译，但只有 `None` 能拿到
    /// 与 ONNX Runtime 等价的精度。
    fixed_seq: Option<usize>,
}

/// 补零（不足）或截断（超出）到指定长度
fn pad_or_truncate(v: &[i64], len: usize, pad: i64) -> Vec<i64> {
    match v.len().cmp(&len) {
        std::cmp::Ordering::Equal => v.to_vec(),
        std::cmp::Ordering::Greater => v[..len].to_vec(),
        std::cmp::Ordering::Less => {
            let mut out = vec![pad; len];
            out[..v.len()].copy_from_slice(v);
            out
        }
    }
}

/// 编译推理计划。
///
/// 两条实测约束，都是踩过才确立的：
///
/// 1. **必须 `with_ignore_value_info(true)`**。该模型是 ONNX Runtime 动态量化导出，
///    图里带 `value_info`，把中间张量声明成 `batch_size`/`sequence_length` 符号。
///    保留这些声明时，tract 会拿输入 fact 的 `1` 去和 `value_info` 的 `batch_size`
///    做 unify，直接报
///    `Impossible to unify Sym(batch_size) with Val(1)`。
/// 2. **动态路径不要自建 SymbolScope 去 `set_input_fact`**。那样 tract 0.23.7 会在
///    `ProofCacheSession` 里触发 `scope_id mismatch` 断言（一个 panic，不是 Err）。
///    不覆盖输入 fact、直接沿用模型自己声明的符号维，才是可用的动态长度。
///
/// - `fixed_seq = None`：保留模型声明的符号维 ⇒ 真正的动态长度（首选，数值与
///   ONNX Runtime 余弦相似度 0.999961）；
/// - `fixed_seq = Some(n)`：seq 固定 n，推理前补零/截断（兜底路径）。补位会改变
///   `DynamicQuantizeLinear` 的量化 scale，精度略降（≈0.9945），但仍可用。
fn build_plan(
    fixed_seq: Option<usize>,
) -> Result<(CompiledModel, Vec<String>), Box<dyn std::error::Error>> {
    let mut model = tract_onnx::onnx()
        .with_ignore_value_info(true)
        .model_for_read(&mut Cursor::new(MODEL_BYTES))?;

    if let Some(n) = fixed_seq {
        let fact = InferenceFact::dt_shape(DatumType::I64, vec![TDim::Val(1), TDim::Val(n as i64)]);
        for i in 0..model.inputs.len() {
            model.set_input_fact(i, fact.clone())?;
        }
    }

    // 输入名必须取源节点名：`outlet_label` 对图输入返回**空串**，
    // 拿它做分派会让三个输入全落到「未知输入名」分支而静默返回 None。
    let input_names: Vec<String> = model
        .inputs
        .iter()
        .map(|&o| model.node(o.node).name.clone())
        .collect();

    // `into_runnable()` 直接返回 `Arc<SimplePlan<..>>`（`RunnableModel = SimplePlan`），
    // 这里不再额外套一层 Arc
    let plan = model.into_typed()?.into_optimized()?.into_runnable()?;
    Ok((plan, input_names))
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

        // 两种编译策略：优先符号化 seq（一次编译、任意长度复用）；
        // 若 tract 无法完成形状推断，自动回退到固定长度 MAX_SEQ（全图静态定形）。
        // 两条路径任一成立即可用，实际选择会打进日志。
        let (plan, input_names, fixed_seq) = match build_plan(None) {
            Ok((plan, names)) => {
                tracing::info!("嵌入模型编译成功：动态长度（保留模型声明的符号维）");
                (plan, names, None)
            }
            Err(e_dyn) => match build_plan(Some(MAX_SEQ)) {
                Ok((plan, names)) => {
                    tracing::warn!(
                        "动态长度编译失败（{}）；已回退固定长度 SEQ={}。\
                         短文本也按满长计算，量化精度略降但语义搜索仍可用",
                        e_dyn,
                        MAX_SEQ
                    );
                    tracing::info!("嵌入模型编译成功：固定长度 SEQ={}", MAX_SEQ);
                    (plan, names, Some(MAX_SEQ))
                }
                Err(e_fixed) => {
                    return Err(EmbeddingError::Init(format!(
                        "模型编译失败（两种策略均失败）: 动态 seq => {e_dyn}; 固定 seq => {e_fixed}"
                    )));
                }
            },
        };

        tracing::info!("Local embedding service ready (pure Rust, no C toolchain).");
        Ok(Arc::new(Self {
            loaded: Arc::new(Loaded {
                plan,
                input_names,
                tokenizer,
                fixed_seq,
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
    let ids_raw: Vec<i64> = enc.get_ids().iter().map(|&x| x as i64).collect();
    let mask_raw: Vec<i64> = enc.get_attention_mask().iter().map(|&x| x as i64).collect();
    let types_raw: Vec<i64> = enc.get_type_ids().iter().map(|&x| x as i64).collect();
    let raw_len = ids_raw.len();
    if raw_len == 0 {
        tracing::warn!("embedding 输入分词后为空");
        return None;
    }
    // 位置嵌入表只有 MAX_SEQ 项，超长一律截断，否则 position gather 越界。
    // 定长计划：补零到 SEQ；动态计划：截断到 MAX_SEQ，用真实分词长度。
    let (ids, mask, types, n) = match loaded.fixed_seq {
        Some(max) => (
            pad_or_truncate(&ids_raw, max, 0),
            pad_or_truncate(&mask_raw, max, 0),
            pad_or_truncate(&types_raw, max, 0),
            max,
        ),
        None => {
            let n = raw_len.min(MAX_SEQ);
            (
                ids_raw[..n].to_vec(),
                mask_raw[..n].to_vec(),
                types_raw[..n].to_vec(),
                n,
            )
        }
    };

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
