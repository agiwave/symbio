//! 插件目录与插件配置规范
//!
//! ## 布局：一个插件 = 一个目录
//!
//! ```text
//! <homedir>/plugins/<插件>/
//!   PLUGIN.yml     ← 插件配置（插件自持读写）
//!   …              ← 该插件自己的数据 / 资源
//! ```
//!
//! 配置与数据**同处一个目录**，因此整个目录可以直接拷贝移植：搬走目录 =
//! 搬走插件（连同它的配置与数据）。
//!
//! ## 系统级插件：目录就是系统根
//!
//! `home` 与它构造的容器 `composite`（home 的动态内置替身）不住在 `plugins/` 下，
//! 它们的目录就是**系统根** `<homedir>` 本身：
//!
//! ```text
//! <homedir>/
//!   PLUGIN.yml     ← home 的配置（系统级状态）
//!   plugins/       ← 容器管辖的插件根：一层目录 = 一个插件
//! ```
//!
//! 这同时消掉一个自举环：若 `home` 住在 `plugins/home`，容器扫描插件根时会把它
//! 当成普通插件再构造一次，而那个 `home` 又会构造容器……系统级插件不参与扫描。
//!
//! ## 配置规范（`PLUGIN.yml`）
//!
//! 一个 YAML 映射：
//!
//! ```yaml
//! plugin_provider: web      # 身份字段：工厂 id（装配方据此构造）
//! plugin_name: web          # 实例名；缺省 = 目录名
//! web_enabled: true         # 以下都是该插件自己的配置
//! web_timeout: 30
//! ```
//!
//! - **加载判据**（由装配方 `composite` 执行）：文件存在、可解析、且
//!   `plugin_provider` 指向一个已注册的工厂（[`has_creator`](crate::symbio_core::has_creator)）。
//! - **身份字段是保留键**：`plugin_provider` / `plugin_name` 不参与插件配置的
//!   反序列化，插件配置也不得占用同名键（本模块在读写时自动剥离 / 补回）。
//!
//! ## 职责划分（本方案的全部要点）
//!
//! | 谁 | 负责什么 |
//! |---|---|
//! | 容器（`composite`） | 发现插件目录、构造插件、把目录告知插件 |
//! | 插件自己 | 读写自己的 `PLUGIN.yml` |
//! | 父插件 | **什么都不负责**——不再持有 / 合并 / 转发子插件配置 |
//!
//! 过去配置集中在 `home` 的 `config.yaml`（`symbio.plugins.<名>`），父插件必须
//! 认识「合并规则」、容器必须认识「分发规则」，而插件自己的配置却不在自己的目录里。
//! 现在配置回到**拥有者**手上：插件目录里的一个文件，谁写谁读。
//!
//! ## 与 VDFS 的关系
//!
//! [`ConfigFile`] 只做两件事：把配置文件**当作一个文件**呈现（`ext = form` +
//! `schema` = 定义），并在写入前按定义校验。它不持有生命周期、不认识父插件、
//! 不向上推任何东西——「写完之后还要做什么」（重启监听 / 重建缓存）留在插件
//! 自己的 `write` 里。

use crate::symbio_core::homedir::HomedirRegistry;
use crate::symbio_core::schemas::detail::DetailDefinition;
use crate::symbio_core::vdfs::host::notify_change;
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsContent, VdfsError, VdfsNode, VdfsResult, VdfsWriteResponse,
    VFDS_CHANGE_UPDATED, VFDS_EXT_FORM,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

/// 插件配置文件名（插件目录下）
pub const PLUGIN_FILE: &str = "PLUGIN.yml";

/// 插件根目录名（系统根下的这一层：一层目录 = 一个插件）
pub const PLUGINS_DIR: &str = "plugins";

/// 身份字段：工厂 id（构造插件用）
pub const KEY_PROVIDER: &str = "plugin_provider";
/// 身份字段：实例名（缺省 = 目录名）
pub const KEY_NAME: &str = "plugin_name";

/// 插件根目录：`<homedir>/plugins`
///
/// 每次现取（不缓存）——`home/reload` 切换 homedir 后必须立刻生效。
pub fn plugins_root() -> PathBuf {
    HomedirRegistry::get().join(PLUGINS_DIR)
}

/// 某插件的目录：`<homedir>/plugins/<插件>`
pub fn dir_of(plugin: &str) -> PathBuf {
    plugins_root().join(plugin)
}

/// 某插件的配置文件：`<homedir>/plugins/<插件>/PLUGIN.yml`
pub fn config_file_of(plugin: &str) -> PathBuf {
    dir_of(plugin).join(PLUGIN_FILE)
}

/// 从插件上下文读自己的目录（装配方经 [`PLUGIN_DIR`](crate::symbio_core::PLUGIN_DIR) 告知）
///
/// 插件构造时的标准入口：`PLUGIN_DIR` 缺省（测试 / 未装配）退回常规落位
/// `<插件根>/<插件>`——与装配态一致，插件自己不必知道两种情形。
pub fn dir_from_ctx(ctx: &dyn crate::symbio_core::InvokeRequest, plugin: &str) -> PluginDir {
    use crate::symbio_core::{InvokeRequestExt, PLUGIN_DIR};
    ctx.get(PLUGIN_DIR).unwrap_or_else(|| PluginDir::of(plugin))
}

// ==================== 插件目录 ====================

/// 一个插件的目录
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDir {
    /// 实例名 = 目录名 = 挂载名
    name: String,
    /// 工厂 id（写回 `PLUGIN.yml` 的身份字段）
    provider: String,
    /// 目录绝对路径
    dir: PathBuf,
}

impl PluginDir {
    /// 常规情形：实例名 = 工厂 id = 目录名
    pub fn of(plugin: impl Into<String>) -> Self {
        let plugin = plugin.into();
        Self {
            dir: dir_of(&plugin),
            name: plugin.clone(),
            provider: plugin,
        }
    }

    /// 显式指定目录（非常规落位 / 测试用）
    pub fn at(dir: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            dir: dir.into(),
            provider: name.clone(),
            name,
        }
    }

    /// **系统级插件**的目录 = 系统根 `<homedir>` 本身
    ///
    /// `home` 与它构造的容器 `composite` 走这一条：系统根下的 `PLUGIN.yml` 是
    /// `home` 的配置，容器只把系统根当**锚点**（它管辖的插件根是
    /// [`plugins_root`](Self::plugins_root)），容器自身没有配置、不写 manifest。
    pub fn system(plugin: impl Into<String>) -> Self {
        Self::at(HomedirRegistry::get(), plugin)
    }

    /// 以本目录为**系统根**，插件根在其下：`<本目录>/plugins`
    ///
    /// 容器的视角——它拿到系统根，据此定位自己管辖的插件。
    pub fn plugins_root(&self) -> PathBuf {
        self.dir.join(PLUGINS_DIR)
    }

    /// 实例名与工厂 id 不同（装配时实例改名）
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = provider.into();
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 配置文件全路径
    pub fn config_path(&self) -> PathBuf {
        self.dir.join(PLUGIN_FILE)
    }

    // ==================== 读 ====================

    /// 读原始 YAML 映射；文件不存在 → `None`，解析失败 → `Err`
    ///
    /// 装配方用它做「配置是否符合要求」的判据（取 `plugin_provider`）。
    pub fn read_manifest(&self) -> Result<Option<Map<String, Value>>, String> {
        let path = self.config_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("读取 {} 失败：{e}", path.display())),
        };
        let value: Value = serde_yaml_ng::from_str(&text)
            .map_err(|e| format!("解析 {} 失败：{e}", path.display()))?;
        match value {
            Value::Object(map) => Ok(Some(map)),
            // 空文件（YAML 解析为 Null）按「没有配置」处理，不算错误
            Value::Null => Ok(None),
            _ => Err(format!("{} 的顶层必须是映射", path.display())),
        }
    }

    /// 读插件配置；文件不存在 → `None`
    ///
    /// 身份字段被剥离后再反序列化——它们属于「这是哪个插件」，不属于配置。
    pub fn load<C: DeserializeOwned>(&self) -> Result<Option<C>, String> {
        let Some(mut map) = self.read_manifest()? else {
            return Ok(None);
        };
        map.remove(KEY_PROVIDER);
        map.remove(KEY_NAME);
        serde_json::from_value(Value::Object(map))
            .map(Some)
            .map_err(|e| {
                format!(
                    "{} 与当前版本的配置结构不符：{e}",
                    self.config_path().display()
                )
            })
    }

    // ==================== 写 ====================

    /// 原子写插件配置（身份字段自动补回）
    ///
    /// 同步实现：配置文件只有几百字节，且 `composite::build` 本身在同步上下文里
    /// 补默认配置——一处实现能同时服务装配期与运行期，不值得为此分两份。
    pub fn save<C: Serialize>(&self, value: &C) -> Result<(), String> {
        let mut map =
            match serde_json::to_value(value).map_err(|e| format!("配置序列化失败：{e}"))? {
                Value::Object(m) => m,
                other => {
                    return Err(format!("配置必须是映射，实得 {other}"));
                }
            };
        map.insert(
            KEY_PROVIDER.to_string(),
            Value::String(self.provider.clone()),
        );
        map.insert(KEY_NAME.to_string(), Value::String(self.name.clone()));
        self.write_map(&map)
    }

    /// 确保目录与配置文件存在（缺失则只写**身份字段**）
    ///
    /// 装配期用：必需插件即便从未配置过，也必须有一个可编辑的 `PLUGIN.yml`。
    /// 容器不知道各插件的配置类型（那是插件自己的事），因此只补身份字段——
    /// 缺省的配置字段由插件在构造时用自己的 `Default` 兜底。
    /// 已存在则**不动**——装配不该覆盖用户配置。
    pub fn ensure_manifest(&self) -> Result<(), String> {
        if self.config_path().exists() {
            return Ok(());
        }
        let mut map = Map::new();
        map.insert(
            KEY_PROVIDER.to_string(),
            Value::String(self.provider.clone()),
        );
        map.insert(KEY_NAME.to_string(), Value::String(self.name.clone()));
        self.write_map(&map)
    }

    /// 从配置文件里**摘掉**若干键（遗留字段清理）
    ///
    /// 一次性迁移用：旧形态把资源明细混在配置里（如 `model` 的 `providers`、
    /// `mcp` 的 `servers`）。插件把它们迁成资源之后，这些键就不该再留在配置
    /// 文件里——否则同一个事实有两份来源。键不存在则不动文件。
    pub fn remove_keys(&self, keys: &[&str]) -> Result<(), String> {
        let Some(mut map) = self.read_manifest()? else {
            return Ok(());
        };
        let before = map.len();
        for key in keys {
            map.remove(*key);
        }
        if map.len() == before {
            return Ok(());
        }
        // 身份字段恒在（手写的配置文件可能漏了它们）
        map.insert(
            KEY_PROVIDER.to_string(),
            Value::String(self.provider.clone()),
        );
        map.insert(KEY_NAME.to_string(), Value::String(self.name.clone()));
        self.write_map(&map)
    }

    fn write_map(&self, map: &Map<String, Value>) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("创建插件目录 {} 失败：{e}", self.dir.display()))?;
        let text = serde_yaml_ng::to_string(map).map_err(|e| format!("配置序列化失败：{e}"))?;

        // 原子写：先写临时文件再 rename 覆盖，避免并发写交错留下半截 YAML
        let path = self.config_path();
        let tmp = path.with_extension("yml.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("写入 {} 失败：{e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("落盘 {} 失败：{e}", path.display()))
    }
}

// ==================== 配置文件的 VDFS 呈现 ====================

/// 插件配置文件的呈现与校验
///
/// 组合 [`PluginDir`]（文件在哪）+ [`DetailDefinition`]（呈现与校验的唯一定义）。
/// 使用方持有它，与自己的配置槽一起接进 `VdfsProvider`：
///
/// ```ignore
/// // 查询侧（list / stat / read）
/// if path == plugin_dir::PLUGIN_FILE { return self.config_file.read(&self.config).await; }
/// // 写入侧
/// if path == plugin_dir::PLUGIN_FILE {
///     let resp = self.config_file.apply(&self.config, content).await?;
///     self.apply_side_effects().await;   // 仅需要副作用的插件（如重启监听）
///     return Ok(resp);
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ConfigFile {
    /// 配置文件所在的插件目录
    dir: PluginDir,
    /// 展示标题（如「网络工具设置」）
    label: String,
    /// 字段定义（呈现与校验同源）
    definition: DetailDefinition,
}

impl ConfigFile {
    pub fn new(dir: PluginDir, label: impl Into<String>, definition: DetailDefinition) -> Self {
        Self {
            dir,
            label: label.into(),
            definition,
        }
    }

    pub fn dir(&self) -> &PluginDir {
        &self.dir
    }

    /// 展示标签（如「网络工具」）——节点标题与「可配置声明」共用这一份
    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn definition(&self) -> &DetailDefinition {
        &self.definition
    }

    /// 节点：`ext = form`（前端据此选通用表单渲染器）、`rw`、`schema` = 定义
    ///
    /// 节点名就是**真实文件名** `PLUGIN.yml`——它是目录里的一个普通文件，
    /// 不是某个保留地址段。`ext` 显式声明为 `form`，覆盖由文件名推导出的 `yml`
    /// （这正是 `ext` 的用途：呈现方式由声明决定，不由文件名猜）。
    pub fn node(&self) -> VdfsNode {
        let mut n = VdfsNode::file(PLUGIN_FILE, &self.label, VdfsAccess::READ_WRITE);
        n.kind = self.dir.name().to_string();
        n.ext = Some(VFDS_EXT_FORM.to_string());
        n.schema = serde_json::to_value(&self.definition).ok();
        n
    }

    /// 读：当前配置 → 内容（pretty JSON）
    ///
    /// 返回 JSON 而非磁盘上的 YAML：`ext = form` 的消费者是表单渲染器，
    /// 它提交与校验的都是 JSON；YAML 只是**落盘形态**。
    pub async fn read<C: Serialize>(&self, slot: &RwLock<C>) -> VdfsResult<VdfsContent> {
        let value = serde_json::to_value(&*slot.read().await)
            .map_err(|e| VdfsError::internal(format!("配置序列化失败：{e}")))?;
        encode(&value)
    }

    /// 写：**校验 → 落内存 → 落自己的文件 → 广播**（插件写配置的唯一路径）
    ///
    /// 需要副作用的插件（如网关重启监听）在本方法返回后再做——那件事只有插件
    /// 自己知道，因此不引入任何回调抽象。
    pub async fn apply<C>(
        &self,
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
        self.dir
            .save(&value)
            .map_err(|e| VdfsError::internal(format!("配置落盘失败：{e}")))?;
        self.announce();
        Ok(VdfsWriteResponse {
            path: PLUGIN_FILE.to_string(),
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
        self.definition
            .validate(&value)
            .map_err(VdfsError::Invalid)?;
        Ok(value)
    }

    /// 广播「配置已更新」（前端据此刷新）
    pub fn announce(&self) {
        notify_change(self.dir.name(), PLUGIN_FILE, VFDS_CHANGE_UPDATED);
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

    fn definition() -> DetailDefinition {
        let mut port = DetailField {
            key: "port".into(),
            label: "端口".into(),
            widget: "number".into(),
            min: Some(1.0),
            max: Some(65535.0),
            ..Default::default()
        };
        port.required = true;
        DetailDefinition {
            sections: vec![DetailSection {
                title: None,
                collapsed: false,
                fields: vec![port],
            }],
            ..Default::default()
        }
    }

    fn dir_at(tmp: &Path) -> PluginDir {
        PluginDir::at(tmp, "demo")
    }

    /// 布局：插件目录 + `PLUGIN.yml`，配置与数据同处一处
    #[test]
    fn plugin_dir_is_the_plugin_home() {
        let tmp = tempfile::TempDir::new().unwrap();
        let d = dir_at(tmp.path());

        assert_eq!(d.dir(), tmp.path());
        assert_eq!(d.config_path(), tmp.path().join(PLUGIN_FILE));
        assert_eq!(d.name(), "demo");
        assert_eq!(d.provider(), "demo");
    }

    /// 身份字段与配置字段同处一个文件，但**只有配置**参与反序列化
    #[test]
    fn identity_fields_are_stripped_and_restored() {
        let tmp = tempfile::TempDir::new().unwrap();
        let d = dir_at(tmp.path()).with_provider("demo_provider");

        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct C {
            port: u32,
        }

        d.save(&C { port: 8080 }).unwrap();

        // 落盘形态：身份字段在前，配置字段平级
        let text = std::fs::read_to_string(d.config_path()).unwrap();
        assert!(text.contains("plugin_provider: demo_provider"), "{text}");
        assert!(text.contains("plugin_name: demo"), "{text}");
        assert!(text.contains("port: 8080"), "{text}");

        // 读回来只有配置——身份字段不会撞进配置结构
        assert_eq!(d.load::<C>().unwrap(), Some(C { port: 8080 }));
    }

    /// 文件不存在 → `None`；空文件按「没有配置」处理，不算错误
    #[test]
    fn missing_and_empty_config_are_both_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let d = dir_at(tmp.path());

        assert_eq!(d.read_manifest().unwrap(), None);
        assert_eq!(d.load::<Map<String, Value>>().unwrap(), None);

        std::fs::write(d.config_path(), "").unwrap();
        assert_eq!(d.read_manifest().unwrap(), None);
    }

    /// `ensure_manifest` 只补身份字段，**不覆盖**已有配置
    #[test]
    fn ensure_never_overwrites_existing_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        let d = dir_at(tmp.path());

        #[derive(serde::Serialize, serde::Deserialize, Default, PartialEq, Debug)]
        struct C {
            #[serde(default)]
            port: u32,
        }

        // 目录都不存在时也能补出来；只有身份字段 → 缺省字段由插件自己的 Default 兜底
        d.ensure_manifest().unwrap();
        let manifest = d.read_manifest().unwrap().unwrap();
        assert_eq!(manifest[KEY_PROVIDER], json!("demo"));
        assert_eq!(manifest[KEY_NAME], json!("demo"));
        assert_eq!(d.load::<C>().unwrap(), Some(C::default()));

        d.save(&C { port: 8080 }).unwrap();
        d.ensure_manifest().unwrap();
        assert_eq!(d.load::<C>().unwrap(), Some(C { port: 8080 }));
    }

    /// `remove_keys`：遗留字段清理后，身份字段仍在；键不存在则不动文件
    #[test]
    fn remove_keys_drops_legacy_fields_but_keeps_identity() {
        let tmp = tempfile::TempDir::new().unwrap();
        let d = dir_at(tmp.path());

        // 旧形态：资源明细混在配置里
        std::fs::write(
            d.config_path(),
            "plugin_provider: demo\nplugin_name: demo\nservers:\n  a: 1\n_storage: plugins/x\n",
        )
        .unwrap();

        d.remove_keys(&["servers"]).unwrap();
        let text = std::fs::read_to_string(d.config_path()).unwrap();
        assert!(!text.contains("servers"), "{text}");
        assert!(
            text.contains("_storage: plugins/x"),
            "未点名的键不动：{text}"
        );
        assert!(text.contains("plugin_provider: demo"), "{text}");

        // 再摘一个不存在的键：文件不动
        let before = std::fs::read_to_string(d.config_path()).unwrap();
        d.remove_keys(&["nope"]).unwrap();
        assert_eq!(std::fs::read_to_string(d.config_path()).unwrap(), before);
    }

    /// 节点：真实文件名 + `ext = form` + `rw` + `schema` 即定义
    #[test]
    fn config_node_is_a_form_document_named_by_the_real_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let f = ConfigFile::new(dir_at(tmp.path()), "演示设置", definition());
        let n = f.node();

        assert_eq!(n.name, PLUGIN_FILE, "地址就是真实文件名，不是保留段");
        assert_eq!(n.title, "演示设置");
        assert_eq!(n.ext.as_deref(), Some(VFDS_EXT_FORM));
        assert_eq!(n.access.flags(), "rw");
        assert!(!n.is_dir(), "配置是文件而非目录");
        assert_eq!(
            n.schema.as_ref().unwrap()["sections"][0]["fields"][0]["key"],
            "port"
        );
    }

    /// 解码即校验：越界 / 缺必填 / 非 JSON 都拦在这里，且是**字段级**错误
    #[test]
    fn decode_validates_through_the_definition() {
        let tmp = tempfile::TempDir::new().unwrap();
        let f = ConfigFile::new(dir_at(tmp.path()), "演示", definition());

        let field_of = |e: VdfsError| match e {
            VdfsError::Invalid(v) => v.fields[0].field.clone(),
            other => panic!("应为字段级校验错误，实得 {other:?}"),
        };

        let bad = VdfsContent::text("", r#"{"port": 70000}"#);
        assert_eq!(field_of(f.decode(&bad).unwrap_err()), "port");

        let missing = VdfsContent::text("", "{}");
        assert_eq!(field_of(f.decode(&missing).unwrap_err()), "port");

        let broken = VdfsContent::text("", "{oops");
        assert!(f.decode(&broken).is_err());

        let ok = VdfsContent::text("", r#"{"port": 8080}"#);
        assert_eq!(f.decode(&ok).unwrap(), json!({ "port": 8080 }));
    }

    /// 读：配置槽 → JSON 文本内容
    #[tokio::test]
    async fn read_serializes_the_slot() {
        let tmp = tempfile::TempDir::new().unwrap();
        let f = ConfigFile::new(dir_at(tmp.path()), "演示", definition());
        let slot = RwLock::new(json!({ "port": 8080 }));

        let content = f.read(&slot).await.unwrap();
        let text = content.text.unwrap();
        assert!(text.contains("\"port\": 8080"), "应为 pretty JSON：{text}");
        assert_eq!(content.mime.as_deref(), Some("application/json"));
    }

    /// 写的一条链：**校验 → 落内存 → 落自己的文件**。
    ///
    /// 锁定本方案的要点：落盘**不再经过父插件**——写的就是自己目录里的
    /// `PLUGIN.yml`，校验未过则内存与磁盘都不动。
    #[tokio::test]
    async fn apply_writes_the_plugins_own_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let f = ConfigFile::new(dir_at(tmp.path()), "演示", definition());
        let slot = RwLock::new(json!({ "port": 1 }));

        // 校验失败：内存与磁盘都不动
        let bad = VdfsContent::text("", r#"{"port": 70000}"#);
        assert!(f.apply(&slot, &bad).await.is_err());
        assert_eq!(*slot.read().await, json!({ "port": 1 }));
        assert!(!f.dir().config_path().exists(), "校验未过不该落盘");

        // 成功：内存生效 + 文件落在自己的目录里
        let resp = f
            .apply(&slot, &VdfsContent::text("", r#"{"port": 8080}"#))
            .await
            .unwrap();
        assert_eq!(resp.path, PLUGIN_FILE);
        assert_eq!(*slot.read().await, json!({ "port": 8080 }));

        let text = std::fs::read_to_string(f.dir().config_path()).unwrap();
        assert!(text.contains("port: 8080"), "{text}");
        assert!(text.contains("plugin_provider: demo"), "{text}");
    }
}
