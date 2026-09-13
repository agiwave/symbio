//! Model 详情页定义（definition-driven detail）
//!
//! 完整表达 Model 表单的全部能力：
//! - 提供商预设联动（选中后按 if_empty 填充 api_base / 首个候选模型，
//!   **总是**校正 api_protocol 为该预设支持的首个协议）
//! - 模型 datalist 动态候选（随预设注入）、协议 select 动态选项
//! - API Key 密码显隐、启用开关、高级设置折叠分区
//! - 条件徽标（默认 / 已停用）与动作（校验连接 / 跳过校验保存 /
//!   保存 / 设为默认 / 删除）
//! - id·name 派生回落链（前端 slug 去重，后端 `validate_manifest` 兜底）

use crate::symbio_core::schemas::entities::{
    DetailAction, DetailBadge, DetailCondition, DetailDefinition, DetailField, DetailOption,
    DetailPreset, DetailPresetSpec, DetailSection,
};

type PresetTuple = (
    &'static str,            // value
    &'static str,            // label
    &'static str,            // api_base
    &'static [&'static str], // models
    &'static [&'static str], // protocols
);

/// 供应商预设表（value, 展示名, 端点, 模型候选, 协议）
const PRESETS: &[PresetTuple] = &[
    (
        "openai",
        "OpenAI",
        "https://api.openai.com/v1",
        &[
            "gpt-4o-mini",
            "gpt-4o",
            "gpt-4-turbo",
            "gpt-3.5-turbo",
            "o1",
            "o1-mini",
            "o3-mini",
        ],
        &["openai_responses", "openai_chat"],
    ),
    (
        "anthropic",
        "Anthropic (Claude)",
        "https://api.anthropic.com/v1",
        &[
            "claude-3-7-sonnet-latest",
            "claude-3-5-sonnet-latest",
            "claude-3-5-haiku-latest",
            "claude-3-opus-latest",
        ],
        &["anthropic_messages"],
    ),
    (
        "gemini",
        "Google Gemini",
        "https://generativelanguage.googleapis.com/v1beta",
        &[
            "gemini-2.5-pro",
            "gemini-2.5-flash",
            "gemini-2.0-pro-exp-02-05",
            "gemini-2.0-flash",
            "gemini-2.0-flash-lite-preview-02-05",
            "gemini-2.0-flash-thinking-exp-01-21",
        ],
        &["gemini_api"],
    ),
    (
        "deepseek",
        "DeepSeek",
        "https://api.deepseek.com/v1",
        &["deepseek-chat", "deepseek-coder", "deepseek-reasoner"],
        &["openai_chat"],
    ),
    (
        "xai",
        "xAI (Grok)",
        "https://api.x.ai/v1",
        &["grok-2-latest", "grok-2-vision-latest"],
        &["openai_chat"],
    ),
    (
        "groq",
        "Groq",
        "https://api.groq.com/openai/v1",
        &[
            "llama-3.3-70b-versatile",
            "llama-3.1-8b-instant",
            "mixtral-8x7b-32768",
        ],
        &["openai_chat"],
    ),
    (
        "siliconflow",
        "硅基流动 (SiliconFlow)",
        "https://api.siliconflow.cn/v1",
        &[
            "deepseek-ai/DeepSeek-R1",
            "deepseek-ai/DeepSeek-V3",
            "Qwen/Qwen2.5-72B-Instruct",
        ],
        &["openai_chat"],
    ),
    (
        "alibaba",
        "阿里云百炼 (Qwen)",
        "https://dashscope.aliyuncs.com/compatible-mode/v1",
        &[
            "qwen-plus",
            "qwen-max",
            "qwen-turbo",
            "qwen2.5-72b-instruct",
        ],
        &["openai_chat"],
    ),
    (
        "tencent",
        "腾讯混元",
        "https://api.hunyuan.cloud.tencent.com/v1",
        &["hunyuan-pro", "hunyuan-standard", "hunyuan-lite"],
        &["openai_chat"],
    ),
    (
        "baidu",
        "百度千帆 (ERNIE)",
        "https://qianfan.baidubce.com/v2",
        &["ernie-4.0-8k-latest", "ernie-3.5-8k", "ernie-speed-128k"],
        &["openai_chat"],
    ),
    (
        "moonshot",
        "月之暗面 (Kimi)",
        "https://api.moonshot.cn/v1",
        &["moonshot-v1-8k", "moonshot-v1-32k", "moonshot-v1-128k"],
        &["openai_chat"],
    ),
    (
        "zhipu",
        "智谱 (GLM)",
        "https://open.bigmodel.cn/api/paas/v4",
        &[
            "glm-4.7-flash",
            "glm-4-plus",
            "glm-4-flash",
            "glm-4",
            "glm-3-turbo",
        ],
        &["openai_chat"],
    ),
    (
        "aiyuanjing",
        "爱媛景 (GLM 兼容)",
        "https://maas-api.ai-yuanjing.com/openapi/compatible-mode/v1",
        &["glm-5", "glm-4-plus", "glm-4"],
        &["openai_chat"],
    ),
    (
        "lmstudio",
        "LM Studio（本地）",
        "http://localhost:1234/v1",
        &[],
        &["anthropic_messages", "openai_chat", "openai_responses"],
    ),
    (
        "local",
        "Ollama（本地）",
        "http://localhost:11434/v1",
        &["llama3", "qwen2", "mistral", "deepseek-coder-v2"],
        &["openai_chat"],
    ),
    (
        "mistral",
        "Mistral",
        "https://api.mistral.ai/v1",
        &[
            "mistral-large-latest",
            "mistral-small-latest",
            "codestral-latest",
            "open-mistral-nemo",
        ],
        &["openai_chat"],
    ),
    (
        "azure",
        "Azure OpenAI",
        "https://<your-resource>.openai.azure.com/openai/v1",
        &[],
        &["openai_chat"],
    ),
    (
        "openrouter",
        "OpenRouter",
        "https://openrouter.ai/api/v1",
        &[
            "openrouter/auto",
            "anthropic/claude-3.5-sonnet",
            "openai/gpt-4o",
            "meta-llama/llama-3.3-70b-instruct",
        ],
        &["openai_chat"],
    ),
    (
        "perplexity",
        "Perplexity",
        "https://api.perplexity.ai",
        &["sonar-pro", "sonar", "sonar-reasoning"],
        &["openai_chat"],
    ),
    (
        "volcengine",
        "火山方舟 (豆包)",
        "https://ark.cn-beijing.volces.com/api/v3",
        &[
            "doubao-seed-1-6-250615",
            "doubao-1-5-pro-32k-250115",
            "doubao-1-5-lite-32k-250115",
        ],
        &["openai_chat"],
    ),
    (
        "spark",
        "讯飞星火",
        "https://spark-api-open.xf-yun.com/v1",
        &["4.0Ultra", "max-32k", "lite"],
        &["openai_chat"],
    ),
    (
        "minimax",
        "MiniMax",
        "https://api.minimax.chat/v1",
        &["MiniMax-Text-01", "abab6.5s-chat"],
        &["openai_chat"],
    ),
    (
        "baichuan",
        "百川智能",
        "https://api.baichuan-ai.com/v1",
        &["Baichuan4", "Baichuan4-Turbo"],
        &["openai_chat"],
    ),
    (
        "step",
        "阶跃星辰 (StepFun)",
        "https://api.stepfun.com/v1",
        &["step-2-16k", "step-1-8k"],
        &["openai_chat"],
    ),
    (
        "cerebras",
        "Cerebras",
        "https://api.cerebras.ai/v1",
        &["llama-3.3-70b", "llama-3.1-8b"],
        &["openai_chat"],
    ),
    (
        "together",
        "Together AI",
        "https://api.together.xyz/v1",
        &[
            "meta-llama/Llama-3.3-70B-Instruct-Turbo",
            "deepseek-ai/DeepSeek-V3",
        ],
        &["openai_chat"],
    ),
    (
        "github",
        "GitHub Models",
        "https://models.github.ai/inference",
        &["gpt-4o", "gpt-4o-mini"],
        &["openai_chat"],
    ),
    (
        "openai_compatible",
        "OpenAI 兼容协议",
        "",
        &[],
        &["openai_chat"],
    ),
    (
        "custom",
        "自定义 (OpenAI 兼容)",
        "",
        &[],
        &[
            "openai_chat",
            "openai_responses",
            "anthropic_messages",
            "gemini_api",
        ],
    ),
];

fn opt(value: &str, label: &str) -> DetailOption {
    DetailOption {
        value: value.into(),
        label: label.into(),
    }
}

fn preset_from(t: &PresetTuple) -> DetailPreset {
    let (value, label, api_base, models, protocols) = *t;
    let mut set = std::collections::BTreeMap::new();
    set.insert("api_base".to_string(), serde_json::json!(api_base));
    if let Some(first_model) = models.first() {
        set.insert("model".to_string(), serde_json::json!(first_model));
    }
    let mut set_always = std::collections::BTreeMap::new();
    if let Some(first_protocol) = protocols.first() {
        set_always.insert(
            "api_protocol".to_string(),
            serde_json::json!(first_protocol),
        );
    }
    let mut options = std::collections::BTreeMap::new();
    options.insert(
        "model".to_string(),
        models.iter().map(|s| s.to_string()).collect(),
    );
    options.insert(
        "api_protocol".to_string(),
        protocols.iter().map(|s| s.to_string()).collect(),
    );
    DetailPreset {
        value: value.into(),
        label: label.into(),
        set,
        set_always,
        options,
    }
}

fn field(key: &str, label: &str, desc: &str, widget: &str) -> DetailField {
    DetailField {
        key: key.into(),
        label: label.into(),
        description: Some(desc.into()),
        widget: widget.into(),
        ..Default::default()
    }
}

/// 新建 Provider 的默认配置字段（VDFS `write { create }` 用）。
///
/// 取**预设首项**——与新建表单「选中第一个预设」的预填**同源**，因此实体机制
/// 与 VDFS 两条链路创建出的初始配置一致。返回 `(供应商, 端点, 模型, 协议)`。
pub fn default_provider_fields() -> (&'static str, &'static str, &'static str, &'static str) {
    let (provider, _label, base, models, protocols) =
        PRESETS
            .first()
            .copied()
            .unwrap_or(("openai", "OpenAI", "", &[], &[]));
    (
        provider,
        base,
        models.first().copied().unwrap_or(""),
        protocols.first().copied().unwrap_or(""),
    )
}

/// Model 详情页定义（每次构建为纯静态结构，开销可忽略）
pub fn model_detail_definition() -> DetailDefinition {
    let provider_options: Vec<DetailOption> = PRESETS.iter().map(|t| opt(t.0, t.1)).collect();

    DetailDefinition {
        binding: "upload".into(),
        load_path: None,
        save_path: None,
        title_from: vec!["name".into(), "model".into()],
        title_fallback: Some("新建 Provider".into()),
        subtitle_from: vec!["provider".into(), "model".into()],
        name_from: vec!["name".into(), "model".into(), "provider".into()],
        id_from: vec!["name".into(), "model".into(), "provider".into()],
        sections: vec![
            DetailSection {
                title: None,
                collapsed: false,
                fields: vec![
                    DetailField {
                        placeholder: Some("例如：OpenAI GPT-4o".into()),
                        ..field(
                            "name",
                            "名称",
                            "未填写时自动使用模型名称（ID 自动生成，无需填写）",
                            "text",
                        )
                    },
                    DetailField {
                        options: provider_options,
                        ..field(
                            "provider",
                            "提供商",
                            "选择一个内置的 Model 提供商预设",
                            "select",
                        )
                    },
                    DetailField {
                        required: true,
                        suggestions_from_preset: true,
                        placeholder: Some("gpt-4o / claude-3-5-sonnet-latest / ...".into()),
                        ..field("model", "模型", "选择或输入目标模型名", "datalist")
                    },
                    DetailField {
                        required: true,
                        placeholder: Some("https://api.openai.com/v1".into()),
                        ..field(
                            "api_base",
                            "API Base URL",
                            "切换提供商时会自动填入对应端点",
                            "text",
                        )
                    },
                    DetailField {
                        placeholder: Some("输入 API Key".into()),
                        ..field("api_key", "API Key", "保存在本机配置文件中", "password")
                    },
                    DetailField {
                        default: Some(serde_json::json!(true)),
                        ..field(
                            "enabled",
                            "启用",
                            "禁用后该 Provider 在 Model 对话选项中不可选",
                            "toggle",
                        )
                    },
                ],
            },
            DetailSection {
                title: Some("高级设置".into()),
                collapsed: true,
                fields: vec![
                    DetailField {
                        options_from_preset: true,
                        ..field(
                            "api_protocol",
                            "API 协议",
                            "与该 Provider 通信的协议风格",
                            "select",
                        )
                    },
                    DetailField {
                        min: Some(0.0),
                        max: Some(2.0),
                        step: Some(0.1),
                        default: Some(serde_json::json!(0.7)),
                        ..field(
                            "temperature",
                            "Temperature",
                            "控制输出随机性（0 - 2）",
                            "number",
                        )
                    },
                    DetailField {
                        min: Some(100.0),
                        max: Some(128000.0),
                        placeholder: Some("4096".into()),
                        default: Some(serde_json::json!(4096)),
                        ..field(
                            "max_tokens",
                            "Max Tokens",
                            "单次回复最大 token 数（留空使用默认）",
                            "number",
                        )
                    },
                    DetailField {
                        min: Some(1024.0),
                        max: Some(10_000_000.0),
                        step: Some(1024.0),
                        placeholder: Some("262144".into()),
                        default: Some(serde_json::json!(262_144)),
                        ..field(
                            "max_context_tokens",
                            "最大上下文 (tokens)",
                            "模型可用的总上下文窗口，默认 256k；运行时会话会与服务上报的上限取较小值",
                            "number",
                        )
                    },
                    DetailField {
                        min: Some(0.0),
                        step: Some(100.0),
                        placeholder: Some("0".into()),
                        default: Some(serde_json::json!(0)),
                        ..field(
                            "rate_limit_ms",
                            "请求频率限制（毫秒）",
                            "两次请求之间的最小间隔；0 表示不限制",
                            "number",
                        )
                    },
                    DetailField {
                        rows: Some(3),
                        full_width: true,
                        placeholder: Some("可选：默认 system prompt".into()),
                        ..field(
                            "system_prompt",
                            "系统提示词",
                            "应用于此 Provider 的全局系统提示词",
                            "textarea",
                        )
                    },
                ],
            },
        ],
        presets: Some(DetailPresetSpec {
            field: "provider".into(),
            fill: "if_empty".into(),
            presets: PRESETS.iter().map(preset_from).collect(),
        }),
        badges: vec![
            DetailBadge {
                when: Some(DetailCondition {
                    key: "is_default".into(),
                    equals: Some(serde_json::json!(true)),
                    ..Default::default()
                }),
                label: "默认".into(),
                style: "default".into(),
            },
            DetailBadge {
                when: Some(DetailCondition {
                    key: "enabled".into(),
                    equals: Some(serde_json::json!(false)),
                    ..Default::default()
                }),
                label: "已停用".into(),
                style: "disabled".into(),
            },
        ],
        actions: vec![
            DetailAction {
                id: "test".into(),
                label: "校验连接".into(),
                style: "secondary".into(),
                when: Some(DetailCondition {
                    key: "cap.test_connection".into(),
                    equals: Some(serde_json::json!(true)),
                    ..Default::default()
                }),
                disabled_when: Some(DetailCondition {
                    key: "is_existing".into(),
                    equals: Some(serde_json::json!(false)),
                    ..Default::default()
                }),
                busy_label: Some("校验中…".into()),
                ..Default::default()
            },
            DetailAction {
                id: "save".into(),
                label: "跳过校验保存".into(),
                style: "secondary".into(),
                // 同为 save 的第二形态：显式指定图标区分（其余动作按 id 默认图标）
                icon: Some("save-skip".into()),
                payload: Some(serde_json::json!({ "skip_validation": true })),
                ..Default::default()
            },
            DetailAction {
                id: "save".into(),
                label: "保存".into(),
                style: "primary".into(),
                busy_label: Some("保存中…".into()),
                ..Default::default()
            },
            DetailAction {
                id: "divider".into(),
                label: String::new(),
                style: "divider".into(),
                ..Default::default()
            },
            DetailAction {
                id: "set-default".into(),
                label: "设为默认 Provider".into(),
                style: "icon".into(),
                when: Some(DetailCondition {
                    all: vec![
                        DetailCondition {
                            key: "is_default".into(),
                            equals: Some(serde_json::json!(false)),
                            ..Default::default()
                        },
                        DetailCondition {
                            key: "is_existing".into(),
                            equals: Some(serde_json::json!(true)),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            },
            DetailAction {
                id: "delete".into(),
                label: "删除 Provider".into(),
                style: "icon danger".into(),
                disabled_when: Some(DetailCondition {
                    key: "is_existing".into(),
                    equals: Some(serde_json::json!(false)),
                    ..Default::default()
                }),
                busy_label: Some("删除中…".into()),
                ..Default::default()
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_covers_model_form_surface() {
        let def = model_detail_definition();
        assert_eq!(def.binding, "upload");
        // 基本分区 6 字段 + 高级折叠分区 6 字段
        assert_eq!(def.sections[0].fields.len(), 6);
        assert_eq!(def.sections[1].title.as_deref(), Some("高级设置"));
        assert!(def.sections[1].collapsed);
        // 高级设置包含最大上下文（max_context_tokens，默认 256k）
        let ctx_field = def.sections[1]
            .fields
            .iter()
            .find(|f| f.key == "max_context_tokens")
            .expect("max_context_tokens field should exist");
        assert_eq!(ctx_field.default, Some(serde_json::json!(262_144)));
        // 预设联动：provider 字段触发，预设含模型候选与协议校正
        let spec = def.presets.as_ref().unwrap();
        assert_eq!(spec.field, "provider");
        assert!(spec.presets.len() >= 20);
        let openai = spec.presets.iter().find(|p| p.value == "openai").unwrap();
        assert_eq!(openai.set["api_base"], "https://api.openai.com/v1");
        assert!(openai.options["api_protocol"].contains(&"openai_responses".to_string()));
        // 动作：test/save/divider/set-default/delete 全齐
        let ids: Vec<&str> = def.actions.iter().map(|a| a.id.as_str()).collect();
        assert!(
            ids.contains(&"test")
                && ids.contains(&"save")
                && ids.contains(&"delete")
                && ids.contains(&"set-default")
        );
    }
}
