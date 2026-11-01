//! persona 插件本体 —— **人格语气一致**（名册 #47）。
//!
//! ## 这条能力在治什么
//!
//! 「同一个人在两处口吻不同」比任何一句措辞不理想都糟。系统里有**两条**面向用户
//! 的输出面：
//!
//! | 输出面 | 谁产出 | 它的提示词从哪来 |
//! |---|---|---|
//! | 轮内正文 / 汇报 | `loop` 的一轮 | `resolve_system_prompt`：请求级段 + 全部注册段 |
//! | 对话面措辞（首响 / 答话） | `compose` 的静默往返 | `build_system_prompt`：全部注册段 + 指令段 |
//!
//! 两条面各自拼自己的系统提示词，但**读的是同一个有序集合**
//! （`capability_prompts`，见 `symbio-sdk/src/capability/prompt.rs`）。因此「一致」
//! 不需要新端口、不需要宿主知道有几种输出面——它只需要人格**只有一个 owner**。
//! 本插件就是那个 owner。
//!
//! ## 为什么不复用 provider 配置里的 `system_prompt`
//!
//! 那是 `model` 插件注册的 `model-persona` 段，它回答的是「**这个模型**的人格」。
//! 两件事被绑在一起的后果是具体的：
//!
//! 1. 换模型 = 换人格。人设属于助手，不属于某个服务端的接入参数；
//! 2. provider 没写 `system_prompt`（出厂形态）⇒ 系统里**根本没有**人格这个概念，
//!    想立一个就得去改接入配置——一件认知上的事被塞进了网络配置里。
//!
//! 两条段可以并存，且不冲突：`model-persona` 排在后面，是对「这个模型该怎么说话」
//! 的补充；人格本体排在最前（装配位 55，见 `PLUGIN.yml`）。
//!
//! ## 平凡值：没填 = 一条段都不交
//!
//! 出厂**不带人格**（那是用户的话）。空正文 ⇒ 不注册段 ⇒ 系统提示词与今天逐字相同
//! ——这条平凡值不是省事，而是「装不装本插件，没配置时行为不变」的可检验形式。
//!
//! ## 本插件不认识任何兄弟插件
//!
//! 它只往能力表里放一行（提示词族），不认识 `loop` / `session` / `compose` 是否
//! 存在，也不给它们任何通知。卸载它 ⇒ 段消失 ⇒ 两条面同时失去人格，而系统照常
//! 运行（T3 形状）。

use super::config::PersonaConfig;
use async_trait::async_trait;
use std::sync::Arc;
use symbio_plugin_api::PLUGIN_ID_PERSONA;
use symbio_plugin_api::{
    capability_announce_configurable, plugin_dir_from_ctx, CapabilityRow, Plugin, PluginConfigFile,
    PluginDir, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, CAPABILITY_VISITOR, PATH, TRAVERSE_AVAILABLE_TOOLS,
};
use symbio_plugin_api::{DetailDefinition, DetailField};
use tokio::sync::RwLock;

/// 提示词族的**段名**（`CapabilityRow::prompt` 的覆盖键）。
///
/// 刻意不叫 `persona`：能力表是一张表，**键是行名、族不参与键**
/// （`Registry::put`，「跨族同名覆盖」那条告警）。本插件同时有一个挂载行
/// `<根>/persona`，行名就是 `persona`——段若与它同键，会把挂载行从表里顶掉，
/// 而顶掉之后**没有任何报错**：设置页少一项、VDFS 少一个挂载点，配置也改不了。
/// `memory` 的 `agent-memory`、`model` 的 `model-persona` 是同一条纪律。
pub const SEGMENT_NAME: &str = "persona-tone";

/// 段标题（渲染为 `【人格】`，与记忆段的 `【智能体记忆】` 同一排版习惯）。
const SEGMENT_TITLE: &str = "人格";

/// 插件的展示标题（元数据 / 配置表单 / 挂载根共用）。
pub const PLUGIN_TITLE: &str = "人格";

/// 配置表单的定义 —— **定义由配置的拥有者产出**。
///
/// 只有一个字段：`persona`。它是 `DetailField::text`（单行）而不是多行输入——
/// 现成的字段种类里没有多行文本框，而**为它加一个新种类**是前端呈现的事，不是
/// 本能力落地的前置（人格正文写成一行、用分号断句，语义完全等价）。
fn config_definition() -> DetailDefinition {
    DetailDefinition::form(
        PLUGIN_TITLE,
        vec![DetailField::text(
            "persona",
            "人格",
            "这个助手是谁、用什么口吻说话。留空 = 不注入（平凡值）。\
             这一段落同时进入轮内正文与对话面措辞的系统提示词。",
        )],
    )
}

/// 人格插件（无状态、无兄弟依赖，唯一的状态是自己那份配置）
pub struct PersonaPlugin {
    config: Arc<RwLock<PersonaConfig>>,
    /// 配置文件的呈现与校验（`<根>/persona/PLUGIN.yml`）——落盘写的是本插件目录里的文件
    config_file: PluginConfigFile,
}

impl PersonaPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_PERSONA);
        let config: PersonaConfig = match dir.load::<PersonaConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => PersonaConfig::default(),
            Err(e) => {
                // 配置读坏了不该让人格静默消失：说清楚，然后按平凡值走
                //（一条读不出来的配置与「没配」在本能力里后果相同，都是不注入）。
                symbio_plugin_api::plugin_warn!("persona", "读取自身配置失败，按未配置处理：{e}");
                PersonaConfig::default()
            }
        };
        Arc::new(PersonaPlugin::new(dir, config)) as Arc<dyn Plugin>
    }

    pub fn new(dir: PluginDir, config: PersonaConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            config_file: PluginConfigFile::new(dir, PLUGIN_TITLE, config_definition()),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_PERSONA, PLUGIN_TITLE)
            .with_description("人格：一处定义「我是谁」，轮内正文与对话措辞读同一段。")
            .with_version("0.1.0")
    }

    /// 当前生效的人格正文（空 ⇒ `None`）。
    ///
    /// 每轮**现读**配置锁，不在构造期快照：面板上改完下一次收集就生效，
    /// 与 `web` 的工具读配置同一条纪律（「保存了但要重启」是最难查的一类失效）。
    async fn persona_text(&self) -> Option<String> {
        self.config.read().await.text().map(str::to_string)
    }

    /// 段正文的**唯一排版处**。
    ///
    /// 标题 + 一句作用域说明 + 正文：说明这一句是必需的，因为同一份系统提示词里
    /// 还躺着记忆、技能、模型人格等若干段，模型需要一个理由知道
    /// 「这一段比其它段更不该被后文改写」。**不写「请保持一致」这类元话术**——
    /// 那是给人看的验收语，不是给模型的指令。
    fn segment_text(persona: &str) -> String {
        format!(
            "【{SEGMENT_TITLE}】你一贯的人格与语气如下。面向用户说的每一句话都按它说，\
             不要中途换口吻。\n\n{persona}"
        )
    }
}

#[async_trait]
impl Plugin for PersonaPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn symbio_plugin_api::VdfsProvider>> {
        Some(self)
    }

    /// 路由入口：**无自有协议**。
    ///
    /// 人格的编辑面是 VDFS（`<根>/persona/PLUGIN.yml`，`ext = form`）——与 `web` /
    /// `memory` 同一取舍：宿主与用户走同一条链路，不为「人格」再造一条协议。
    /// 造一条协议意味着再造一份校验、一份广播、一份前端渲染。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(PATH).unwrap_or_default();
        Err(PluginError::NotFound(format!(
            "persona 无自有协议路由 `{path}`：人格一律经 VDFS 访问（<根>/persona/PLUGIN.yml）"
        )))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let sub_path = ctx.get(PATH).unwrap_or_default();
        if sub_path != TRAVERSE_AVAILABLE_TOOLS {
            return Err(PluginError::NotFound(format!("未知遍历路径: {sub_path}")));
        }

        if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
            // 空正文 ⇒ 不注册：注册一条空段等于往系统提示词里塞一个空标题，
            // 而消费侧只跳「空正文」不跳「有标题没正文」——那会平白多占一行上下文。
            if let Some(text) = self.persona_text().await {
                visitor
                    .put(CapabilityRow::prompt(
                        SEGMENT_NAME,
                        Self::segment_text(&text),
                    ))
                    .await;
            }
        }

        // 顺带声明「本插件有一份配置文档」：设置页据此列出本项并指路到
        // `<根>/persona/PLUGIN.yml`（标签与配置节点共用同一个来源，见 `PluginConfigFile`）
        capability_announce_configurable(&ctx, &self.config_file).await;

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

// ==================== VDFS：配置文档（`<根>/persona/PLUGIN.yml`） ====================
//
// 本插件在 VDFS 上的全部内容 = 一个配置文档，故实现 [`PluginConfigMount`] 即可——
// 四臂 dispatch 由机制侧提供（见 `symbio_plugin_api::PluginConfigMount`），本处只声明差异。

impl symbio_plugin_api::PluginConfigMount for PersonaPlugin {
    type Config = PersonaConfig;
    const TITLE: &'static str = PLUGIN_TITLE;

    fn config_file(&self) -> &PluginConfigFile {
        &self.config_file
    }

    fn config_slot(&self) -> &RwLock<PersonaConfig> {
        &self.config
    }
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_PERSONA, PersonaPlugin::build, dyn Plugin);

symbio_plugin_api::impl_config_mount_vdfs!(PersonaPlugin);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
