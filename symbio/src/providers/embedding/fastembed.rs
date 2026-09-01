use crate::symbio_core::providers::EmbeddingError;
use crate::symbio_core::providers::EmbeddingService;
use crate::symbio_core::{InvokeRequest, EMBEDDING_FASTEMBED, EMBEDDING_NOOP};
use async_trait::async_trait;
use fastembed::{InitOptions, TextEmbedding};
use std::sync::{Arc, LazyLock, Mutex};

const MODEL_BYTES: &[u8] = include_bytes!("model.onnx");
const CONFIG_BYTES: &[u8] = include_bytes!("config.json");
const TOKENIZER_BYTES: &[u8] = include_bytes!("tokenizer.json");
const TOKENIZER_CONFIG_BYTES: &[u8] = include_bytes!("tokenizer_config.json");
const SPECIAL_TOKENS_BYTES: &[u8] = include_bytes!("special_tokens_map.json");

/// 全局单例的 FastEmbed 服务
///
/// 改进点：用 `tracing::info!` 替换之前的 `println!`，接入统一日志通道
static GLOBAL_FAST_EMBED: LazyLock<Result<Arc<FastEmbedService>, EmbeddingError>> =
    LazyLock::new(FastEmbedService::init_internal);

pub(crate) struct FastEmbedService {
    /// 使用 std::sync::Mutex 以便在 spawn_blocking 中安全使用
    /// (S-002 审计通过): TextEmbedding::embed 是纯 CPU 计算，在 `spawn_blocking` 派生的
    /// 阻塞线程池线程中执行；持锁范围仅在 spawn_blocking 闭包内，不跨 .await 边界，
    /// 因此 std::sync::Mutex 不会阻塞 tokio worker。改用 tokio::sync::Mutex 反而会
    /// 引入不必要的 async 桥接开销。
    model: Arc<Mutex<TextEmbedding>>,
}

impl FastEmbedService {
    fn get_instance() -> Result<Arc<dyn EmbeddingService>, EmbeddingError> {
        match &*GLOBAL_FAST_EMBED {
            Ok(service) => Ok(service.clone() as Arc<dyn EmbeddingService>),
            Err(e) => Err(EmbeddingError::Init(format!(
                "Global Embedding Service failed to init: {}",
                e
            ))),
        }
    }

    fn init_internal() -> Result<Arc<Self>, EmbeddingError> {
        tracing::info!("[Mindscape] Initializing Embedding service in offline (in-memory) mode...");

        let tokenizer_files = fastembed::TokenizerFiles {
            tokenizer_file: TOKENIZER_BYTES.to_vec(),
            config_file: CONFIG_BYTES.to_vec(),
            special_tokens_map_file: SPECIAL_TOKENS_BYTES.to_vec(),
            tokenizer_config_file: TOKENIZER_CONFIG_BYTES.to_vec(),
        };

        let user_model =
            fastembed::UserDefinedEmbeddingModel::new(MODEL_BYTES.to_vec(), tokenizer_files);

        let model =
            TextEmbedding::try_new_from_user_defined(user_model, InitOptions::default().into())
                .map_err(|e| {
                    EmbeddingError::Init(format!(
                        "Failed to initialize memory-based fastembed: {e}"
                    ))
                })?;

        tracing::info!("[Mindscape] Local Embedding service ready (Global Singleton).");

        Ok(Arc::new(Self {
            model: Arc::new(Mutex::new(model)),
        }))
    }
}

#[async_trait]
impl EmbeddingService for FastEmbedService {
    async fn embed(&self, text: &str) -> Option<Vec<f32>> {
        let model = self.model.clone();
        let text = text.to_string();

        tokio::task::spawn_blocking(move || {
            let mut model = model.lock().expect("Embedding model mutex poisoned");
            match model.embed(vec![text], None) {
                Ok(mut embeddings) => embeddings.pop(),
                Err(e) => {
                    tracing::warn!("Failed to generate embedding: {}", e);
                    None
                }
            }
        })
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

fn build_fastembed(_ctx: Arc<dyn InvokeRequest>) -> Arc<dyn EmbeddingService> {
    match FastEmbedService::get_instance() {
        Ok(svc) => svc,
        Err(e) => {
            // 使用醒目警告格式，明确说明语义搜索将被禁用
            tracing::warn!(
                "⚠️ FastEmbedService unavailable ({}). Falling back to NoopEmbeddingService. \
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

crate::submit_object_creator!(EMBEDDING_FASTEMBED, build_fastembed, dyn EmbeddingService);
crate::submit_object_creator!(EMBEDDING_NOOP, build_noop, dyn EmbeddingService);

#[cfg(test)]
mod tests;
