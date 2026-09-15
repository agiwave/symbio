//! 配置文档型 —— 「一个插件的配置 = 一个可寻址文档」
//!
//! 取代 `CONFIG_GET` / `CONFIG_SET`（第二套资源协议）与 `setting` 插件的分区代理。
//! 配置不再经一条私有路由读写，而是一个普通的 VDFS 节点：`ext = form`、
//! `access = rw`、`schema` = 该插件自己的详情定义。于是「读配置」「改配置」用的
//! 就是 `vdfs/read` / `vdfs/write`，与其它任何资源同一条链路、同一套寻址。
//!
//! ## 地址
//!
//! `<挂载根>/配置`（[`SEG_CONFIG`]）。挂载根恒为目录（前端导航只列目录），
//! 因此配置文档是根下的一个**保留子段**：
//!
//! - 只有配置、没有资源树的插件（本地工具 / 网络工具 / 开放接口）——
//!   `impl VdfsProvider` 的查询方法直接转发给本模块；
//! - 已有资源树的插件（会话 / 模型 / MCP）——在自身 `list` / `stat` /
//!   `read` / `write` 里把 `配置` 段交给本模块。
//!
//! ## 本模块不是一层抽象
//!
//! [`ConfigDoc`] 是**值 + 一组函数**：节点形状、编解码、落盘通知各写一遍。
//! 没有 trait、没有注册表、没有回调——「写配置之后还要做什么」（重启监听 /
//! 重建缓存）留在插件自己的 `write` 里，因此不需要钩子抽象。
//!
//! ## 校验归定义，落盘靠推送
//!
//! - **校验归定义**：提交值交给 [`DetailDefinition::validate`]，失败即字段级
//!   错误（`vdfs/write` 的失败载荷），前端据此逐字段高亮——定义与校验同源，
//!   插件不再各写一套字段规则；
//! - **落盘靠推送**：写配置者把自己的切片推给宿主（`save_config` 载荷
//!   `{plugin, config}`），宿主只负责合并落盘，**不再反向拉取**。

use crate::symbio_core::schemas::common::ConfigSlice;
use crate::symbio_core::schemas::detail::DetailDefinition;
use crate::symbio_core::vdfs::host::{host_ctx, notify_change};
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsContent, VdfsContext, VdfsError, VdfsNode, VdfsResult, VdfsWriteResponse,
    VFDS_CHANGE_UPDATED, VFDS_EXT_FORM,
};
use crate::symbio_core::{InvokeRequestExt, PATH, SAVE_CONFIG};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::RwLock;

/// 配置文档的地址段：`<挂载根>/配置`
///
/// **保留段**：插件自身资源不得占用同名 id——判定顺序上配置优先。
/// 会话 id 是 UUID、模型 / MCP 的 id 由用户命名派生，理论上可撞名，
/// 因此「保留段优先」是地址规则的一部分（见 `docs/design/vdfs.md`）。
pub const SEG_CONFIG: &str = "配置";

/// 路径是否指向配置文档（`配置` 段，且不再下钻）
pub fn is_config_path(path: &str) -> bool {
    path == SEG_CONFIG
}

/// 一个插件的配置文档
///
/// 使用方持有它（与自己的配置槽一起），把它接进自己的 `VdfsProvider`：
///
/// ```ignore
/// // 查询侧（list / stat / read）
/// if config::is_config_path(path) { return self.config_doc.read(&self.config).await; }
/// // 写入侧
/// if config::is_config_path(path) {
///     let resp = self.config_doc.apply(ctx, &self.config, content).await?;
///     self.apply_side_effects().await;   // 仅需要副作用的插件（如重启监听）
///     return Ok(resp);
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ConfigDoc {
    /// 广播频道键 / 落盘切片的归属 = 插件名
    kind: String,
    /// 文档标题（如「会话设置」）
    label: String,
    /// 字段定义（呈现与校验同源）
    definition: DetailDefinition,
}

impl ConfigDoc {
    pub fn new(
        kind: impl Into<String>,
        label: impl Into<String>,
        definition: DetailDefinition,
    ) -> Self {
        Self {
            kind: kind.into(),
            label: label.into(),
            definition,
        }
    }

    /// 文档节点：`ext = form`（前端据此选通用表单渲染器）、`rw`、`schema` = 定义
    pub fn node(&self) -> VdfsNode {
        let mut n = VdfsNode::file(SEG_CONFIG, &self.label, VdfsAccess::READ_WRITE);
        n.kind = self.kind.clone();
        n.ext = Some(VFDS_EXT_FORM.to_string());
        n.schema = serde_json::to_value(&self.definition).ok();
        n
    }

    /// 读：当前配置 → 内容（pretty JSON）
    pub async fn read<C: Serialize>(&self, slot: &RwLock<C>) -> VdfsResult<VdfsContent> {
        let value = serde_json::to_value(&*slot.read().await)
            .map_err(|e| VdfsError::internal(format!("配置序列化失败：{e}")))?;
        encode(&value)
    }

    /// 写：**校验 → 落内存 → 落盘 → 广播**（插件写配置的唯一路径）
    ///
    /// 需要副作用的插件（如网关重启监听）在本方法返回后再做——那件事只有插件
    /// 自己知道，因此不引入任何回调抽象。
    pub async fn apply<C>(
        &self,
        ctx: &VdfsContext,
        slot: &RwLock<C>,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse>
    where
        C: Serialize + DeserializeOwned,
    {
        let value = self.decode(content)?;
        let next: C = serde_json::from_value(value.clone())
            .map_err(|e| VdfsError::invalid(format!("配置结构与当前版本不符：{e}")))?;
        *slot.write().await = next;
        self.persist(ctx, &value).await?;
        self.announce();
        Ok(VdfsWriteResponse {
            path: SEG_CONFIG.to_string(),
            created: false,
            etag: None,
        })
    }

    /// 提交内容 → 校验后的值（定义校验失败即字段级错误）
    pub fn decode(&self, content: &VdfsContent) -> VdfsResult<Value> {
        let text = content
            .text
            .as_deref()
            .ok_or_else(|| VdfsError::invalid("配置写入需要文本（JSON）内容"))?;
        let value: Value = serde_json::from_str(text)
            .map_err(|e| VdfsError::invalid(format!("配置不是合法 JSON：{e}")))?;
        self.definition.validate(&value).map_err(VdfsError::Invalid)?;
        Ok(value)
    }

    /// 把本插件的配置切片推给宿主落盘（`save_config`）
    pub async fn persist(&self, ctx: &VdfsContext, value: &Value) -> VdfsResult<()> {
        let host = host_ctx(ctx)?;
        let parent = host
            .parent()
            .and_then(|w| w.upgrade())
            .ok_or_else(|| VdfsError::internal("插件未挂载到容器，配置无法落盘"))?;
        let save = host.fork();
        save.set(PATH, SAVE_CONFIG.to_string());
        save.set_payload(ConfigSlice::new(self.kind.clone(), value.clone()))
            .map_err(|e| VdfsError::internal(e.to_string()))?;
        parent
            .route(save)
            .await
            .map_err(|e| VdfsError::internal(e.to_string()))?;
        Ok(())
    }

    /// 广播「配置已更新」（前端据此刷新）
    pub fn announce(&self) {
        notify_change(&self.kind, SEG_CONFIG, VFDS_CHANGE_UPDATED);
    }
}

/// 值 → 内容（pretty JSON，`mime = application/json`）
fn encode(value: &Value) -> VdfsResult<VdfsContent> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| VdfsError::internal(format!("配置序列化失败：{e}")))?;
    Ok(VdfsContent::text("", text).with_mime("application/json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::schemas::detail::{DetailField, DetailSection};
    use serde_json::json;

    fn doc() -> ConfigDoc {
        let mut port = DetailField {
            key: "port".into(),
            label: "端口".into(),
            widget: "number".into(),
            min: Some(1.0),
            max: Some(65535.0),
            ..Default::default()
        };
        port.required = true;
        ConfigDoc::new(
            "demo",
            "演示设置",
            DetailDefinition {
                sections: vec![DetailSection {
                    title: None,
                    collapsed: false,
                    fields: vec![port],
                }],
                ..Default::default()
            },
        )
    }

    /// 节点形状固定：`ext = form` + `rw` + `schema` 即定义
    #[test]
    fn node_is_a_writable_form_document() {
        let n = doc().node();
        assert_eq!(n.name, SEG_CONFIG);
        assert_eq!(n.title, "演示设置");
        assert_eq!(n.ext.as_deref(), Some(VFDS_EXT_FORM));
        assert_eq!(n.access.flags(), "rw");
        assert!(!n.is_dir(), "配置是文档而非目录");
        assert_eq!(n.schema.as_ref().unwrap()["sections"][0]["fields"][0]["key"], "port");
    }

    /// 解码即校验：越界 / 缺必填 / 非 JSON 都拦在这里，且是**字段级**错误
    #[test]
    fn decode_validates_through_the_definition() {
        let d = doc();

        let field_of = |e: VdfsError| match e {
            VdfsError::Invalid(v) => v.fields[0].field.clone(),
            other => panic!("应为字段级校验错误，实得 {other:?}"),
        };

        let bad = VdfsContent::text("", r#"{"port": 70000}"#);
        assert_eq!(field_of(d.decode(&bad).unwrap_err()), "port");

        let missing = VdfsContent::text("", "{}");
        assert_eq!(field_of(d.decode(&missing).unwrap_err()), "port");

        let broken = VdfsContent::text("", "{oops");
        assert!(d.decode(&broken).is_err());

        let ok = VdfsContent::text("", r#"{"port": 8080}"#);
        assert_eq!(d.decode(&ok).unwrap(), json!({ "port": 8080 }));
    }

    /// 读：配置槽 → JSON 文本内容
    #[tokio::test]
    async fn read_serializes_the_slot() {
        let slot = RwLock::new(json!({ "port": 8080 }));
        let content = doc().read(&slot).await.unwrap();
        let text = content.text.unwrap();
        assert!(text.contains("\"port\": 8080"), "应为 pretty JSON：{text}");
        assert_eq!(content.mime.as_deref(), Some("application/json"));
    }

    #[test]
    fn config_path_matching_is_exact() {
        assert!(is_config_path(SEG_CONFIG));
        assert!(!is_config_path(""));
        assert!(!is_config_path("配置/子项"));
        assert!(!is_config_path("session"));
    }

    /// 写的一条链：**校验 → 落内存 → 落盘 → 广播**。
    ///
    /// 同时锁定落盘契约——路由仍是 `save_config`（插件让父容器帮自己存，
    /// 这一层不变），只是载荷从「父容器反向拉取」改成「写配置者推自己的切片」。
    #[tokio::test]
    async fn apply_stores_and_pushes_the_slice_to_the_parent() {
        use crate::symbio_core::{
            InvokeRequest, InvokeResponse, Plugin, PluginMeta, PluginPayload, SimpleRequest, PATH,
        };
        use std::sync::{Arc, Mutex};

        /// 记录「父容器收到了什么」的极简父插件
        struct Recorder(Mutex<Vec<(String, Option<ConfigSlice>)>>);

        #[async_trait::async_trait]
        impl Plugin for Recorder {
            fn meta(&self) -> PluginMeta {
                PluginMeta::new("recorder", "记录器")
            }
            async fn route(
                self: Arc<Self>,
                ctx: Arc<dyn InvokeRequest>,
            ) -> InvokeResponse<PluginPayload> {
                self.0.lock().unwrap().push((
                    ctx.get(PATH).unwrap_or_default(),
                    ctx.payload::<ConfigSlice>().ok(),
                ));
                Ok(PluginPayload::new(&json!({ "ok": true })))
            }
            async fn traverse(
                self: Arc<Self>,
                _path: String,
                _ctx: Arc<dyn InvokeRequest>,
            ) -> InvokeResponse<PluginPayload> {
                Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
            }
        }

        let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
        let parent: Arc<dyn Plugin> = recorder.clone();
        let req: Arc<dyn InvokeRequest> =
            Arc::new(SimpleRequest::new(Some(Arc::downgrade(&parent)), None));
        let ctx = crate::symbio_core::vdfs::vdfs_context(&req);

        let d = doc();
        let slot = RwLock::new(json!({ "port": 1 }));

        // 校验失败：既不落内存，也不触达父容器
        let bad = VdfsContent::text("", r#"{"port": 70000}"#);
        assert!(d.apply(&ctx, &slot, &bad).await.is_err());
        assert_eq!(*slot.read().await, json!({ "port": 1 }));
        assert!(recorder.0.lock().unwrap().is_empty(), "校验未过不该落盘");

        // 成功：内存生效 + 切片推给父容器
        let resp = d
            .apply(&ctx, &slot, &VdfsContent::text("", r#"{"port": 8080}"#))
            .await
            .unwrap();
        assert_eq!(resp.path, SEG_CONFIG);
        assert_eq!(*slot.read().await, json!({ "port": 8080 }));

        let seen = recorder.0.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, SAVE_CONFIG);
        let slice = seen[0].1.as_ref().expect("落盘载荷应是 ConfigSlice");
        assert_eq!(slice.plugin, "demo");
        assert_eq!(slice.config["port"], 8080);
    }
}
