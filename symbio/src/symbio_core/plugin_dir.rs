//! 插件目录与插件配置规范
//!
//! ## 布局：一个插件 = 一个目录
//!
//! ```text
//! <homedir>/<插件>/
//!   PLUGIN.yml     ← 插件配置（插件自持读写）
//!   …              ← 该插件自己的数据 / 资源
//! ```
//!
//! 配置与数据**同处一个目录**，因此整个目录可以直接拷贝移植：搬走目录 =
//! 搬走插件（连同它的配置与数据）。
//!
//! ## 系统级插件：目录就是系统根
//!
//! `home` 与它构造的容器 `composite`（home 的动态内置替身）的目录就是**系统根**
//! `<homedir>` 本身，它们与业务插件**并列**：
//!
//! ```text
//! <homedir>/
//!   PLUGIN.yml     ← home 的配置（系统级状态）
//!   session/       ← 容器管辖的插件根（= 系统根）：一层目录 = 一个插件
//!   model/
//!   …
//! ```
//!
//! 插件根**不额外嵌套一层**（早先是 `<homedir>/plugins/<插件>`）：那一层既不承载
//! 语义、又让每个插件的路径深一段，而系统根下本来就是「一个插件一个目录」的扁平
//! 结构，再加一层纯属重复。
//!
//! 这同时消掉一个自举环：若 `home` 住在 `<homedir>/home`，容器扫描插件根时会把它
//! 当成普通插件再构造一次，而那个 `home` 又会构造容器……系统级插件不参与扫描。
//!
//! ## 插件不认识全局布局
//!
//! 插件**不知道、也不该知道**自己被放在哪。目录一律由容器经 `PLUGIN_DIR` 告知
//! （[`dir_from_ctx`]），插件只持有 [`PluginDir`] 并向下传：
//!
//! - ❌ 不要 `HomedirRegistry::get().join("plugins").join(PLUGIN_X)`——按插件名反推
//!   落位等于把「装配决策」写死进插件，插件挪个位置就全错；
//! - ❌ 不要 [`crate::providers::vdfs_service::entry::category_dir`]——同上，它只留给
//!   读旧版历史落位的数据迁移和测试；
//! - ✅ 实例方法里用 `self.dir`；按 id 派生路径的**自由函数**则让调用方把根当参数
//!   传进来（见 `plugins::session::paths`，`root` 是首个入参）。
//!
//! 注释与文档同理：写「本插件自己的目录」，不写 `<homedir>/…` 具体路径。
//! 插件目录是**父插件给的**，不是插件算出来的。
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
    VdfsAccess, VdfsContent, VdfsError, VdfsNode, VdfsResult, VdfsWriteResponse, VDFS_EXT_FORM,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

/// 插件配置文件名（插件目录下）
pub const PLUGIN_FILE: &str = "PLUGIN.yml";

// 注：早先这里有常量 `PLUGINS_DIR = "plugins"`，插件落位是 `<homedir>/plugins/<插件>`。
// 那一层不承载语义（既非挂载点、也不参与寻址），只是把每个插件的路径都加深一段，
// 现已去掉——插件直接并列在系统根下。**「插件根 = 哪一层」只有本文件这一处定义**：
// 依赖方一律走 [`plugins_root`] / [`dir_of`]，不要自己 `join` 目录段。

/// 身份字段：工厂 id（构造插件用）
pub const KEY_PROVIDER: &str = "plugin_provider";
/// 身份字段：实例名（缺省 = 目录名）
pub const KEY_NAME: &str = "plugin_name";
/// **装配位**：`false` = 停用（缺省 / 键不存在 = 启用）
///
/// 与 [`KEY_PROVIDER`] / [`KEY_NAME`] 同类——它不是插件自己的配置，而是
/// **装配方对这个插件的状态**：停用的插件连同它的配置与数据一起留在插件根里，
/// 只是容器不再构造它（因而它不启动任何后台行为，也不出现在资源树与能力集合里）。
///
/// ## 为什么是 `PLUGIN.yml` 里的一个保留键
///
/// 「一个插件 = 一个目录」，目录可以被整体搬走；装配位若另立一个文件，那个
/// 不变量就变成「一个插件 = 一个目录 + 一个别处的标记」。写进同一份 manifest，
/// 则搬走目录 = 连它的启停状态一起搬走。
///
/// ## 为什么默认是「启用」
///
/// 键**不存在**即启用。于是：新装 / 手写 / 从旧版本迁移来的插件天然可用，
/// 而「停用」是一个**显式**动作。反过来（缺省停用）会让每一个新插件都先隐形。
pub const KEY_ENABLED: &str = "plugin_enabled";

// ==================== 装配态在**配置表单模型**里的投影键 ====================
//
// 插件管理插件的条目表单，其模型就是那个插件的配置（`vdfs/read` 的结果）。但条目上
// 的按钮（启用 / 停用 / 卸载）要按**装配态**显隐，而 `DetailAction.when` 只能对表单
// 模型求值（见 `schemas/detail.rs`）——于是装配态得一并放进那份模型。
//
// 三个键与身份字段同前缀（`plugin_`）：它们在 `PLUGIN.yml` 里同样是**保留键**
// （插件配置不得占用），而表单保存时只回传**定义声明过的字段**（见前端
// `DetailForm.buildValues`），因此注入它们既不会显示成字段，也不会写进配置文件。
//
// 键名与它投影的来源**逐字对应**，改一处就能顺着找到另一处：
// `plugin_version` ← `PluginEntry::version`、`plugin_required` ← `PluginEntry::required`、
// `plugin_can_disable` ← `UNDISABLABLE_PLUGINS` 的补集。

/// 投影键：版本（`PluginEntry::version`；插件未被构造时为空串）
pub const KEY_VERSION: &str = "plugin_version";
/// 投影键：构造者是否声明为必需（`PluginEntry::required`）——必需即**不可删除**
pub const KEY_REQUIRED: &str = "plugin_required";
/// 投影键：是否允许停用（= 不在 [`crate::symbio_core::UNDISABLABLE_PLUGINS`] 里）
pub const KEY_CAN_DISABLE: &str = "plugin_can_disable";

/// 插件根 = **系统根本身**：一层目录 = 一个插件
///
/// 每次现取（不缓存）——`home/reload` 切换 homedir 后必须立刻生效。
pub fn plugins_root() -> PathBuf {
    HomedirRegistry::get()
}

/// 某插件的目录：`<homedir>/<插件>`
pub fn dir_of(plugin: &str) -> PathBuf {
    plugins_root().join(plugin)
}

/// 某插件的配置文件：`<homedir>/<插件>/PLUGIN.yml`
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
    /// `home` 的配置，容器只把系统根当**锚点**（它管辖的插件根就是系统根本身），
    /// 容器自身没有配置、不写 manifest。
    pub fn system(plugin: impl Into<String>) -> Self {
        Self::at(HomedirRegistry::get(), plugin)
    }

    /// 把**本目录当成系统根**：插件根就是它自己（`<本目录>/<插件>`）
    ///
    /// ⚠️ 只有**系统级插件**（`home` / `composite`，其目录就是系统根）可以这样看。
    /// 叶子插件调它拿到的是自己的目录，不是任何「根」——需要自己的目录直接用
    /// [`dir`](Self::dir)。名字里的 `as_` 正是在提示这是一种**视角转换**，不是查询。
    pub fn as_plugins_root(&self) -> PathBuf {
        self.dir.clone()
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
    /// 身份字段与装配位被剥离后再反序列化——它们属于「这是哪个插件、它开没开」，
    /// 不属于配置。
    pub fn load<C: DeserializeOwned>(&self) -> Result<Option<C>, String> {
        let Some(mut map) = self.read_manifest()? else {
            return Ok(None);
        };
        map.remove(KEY_PROVIDER);
        map.remove(KEY_NAME);
        map.remove(KEY_ENABLED);
        serde_json::from_value(Value::Object(map))
            .map(Some)
            .map_err(|e| {
                format!(
                    "{} 与当前版本的配置结构不符：{e}",
                    self.config_path().display()
                )
            })
    }

    // ==================== 装配位 ====================

    /// 本插件是否启用（见 [`KEY_ENABLED`]）。
    ///
    /// 文件缺失 / 不可读 / 不可解析一律按**启用**处理：「读不出来」不该把一个插件
    /// 静默停掉——那是装配期告警的职责，不是本判据的（否则一次磁盘故障会让整棵树
    /// 的插件集体隐形，而日志里只有解析错误）。
    pub fn enabled(&self) -> bool {
        match self.read_manifest() {
            Ok(Some(map)) => !matches!(map.get(KEY_ENABLED), Some(Value::Bool(false))),
            _ => true,
        }
    }

    /// 写装配位：`true` = 启用（摘掉键，保持文件干净）；`false` = 停用。
    ///
    /// 只动这一个键，其余内容原样保留——身份字段恒在（手写的 manifest 可能漏了它们）。
    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let mut map = self.read_manifest()?.unwrap_or_default();
        if enabled {
            map.remove(KEY_ENABLED);
        } else {
            map.insert(KEY_ENABLED.to_string(), Value::Bool(false));
        }
        map.insert(
            KEY_PROVIDER.to_string(),
            Value::String(self.provider.clone()),
        );
        map.insert(KEY_NAME.to_string(), Value::String(self.name.clone()));
        self.write_map(&map)
    }

    // ==================== 写 ====================

    /// 原子写插件配置（身份字段自动补回）
    ///
    /// 同步实现：配置文件只有几百字节，且 `composite::build` 本身在同步上下文里
    /// 补默认配置——一处实现能同时服务装配期与运行期，不值得为此分两份。
    ///
    /// **装配位随写保留**：`plugin_enabled` 是保留键（见 [`KEY_ENABLED`]），
    /// 插件写自己的配置时不得把它冲掉——否则「在设置页里保存一次配置」就等于
    /// 悄悄把停用的插件启用了（或反之）。故写入前先读一次旧 manifest 取回它。
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
        if let Ok(Some(existing)) = self.read_manifest() {
            if let Some(flag) = existing.get(KEY_ENABLED) {
                map.insert(KEY_ENABLED.to_string(), flag.clone());
            }
        }
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

// ==================== 插件注册表条目 ====================

/// 插件注册表里的一条 —— **一个插件目录的观测结果**。
///
/// 由装配方（容器）产出：它扫插件根、读每个目录的 `PLUGIN.yml`、与自己的实例表
/// 对照，得到「这个智能体由哪些插件组成、各自什么状态」。消费方（插件管理插件）
/// 只读它、不自己扫目录——「有哪些插件」的判据（合格性 / 必需 / 启用）只有一份实现。
///
/// ## 为什么标题可能是空的
///
/// `title` / `description` / `version` 来自插件的 [`PluginMeta`]（`Plugin::meta()`），
/// 而那是**构造物**：只有被构造出来的插件才答得上来。停用的插件**刻意不被构造**
/// （见 [`KEY_ENABLED`]——它不该启动任何后台行为），因此这几个字段在那种情形下为空，
/// 消费方按目录名兜底。
///
/// [`PluginMeta`]: crate::symbio_core::PluginMeta
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PluginEntry {
    /// 插件名 = 目录名 = 挂载名（路由前缀）
    pub name: String,
    /// 工厂 id（`PLUGIN.yml` 的 `plugin_provider`）
    pub provider: String,
    /// 展示标题（来自 `PluginMeta`；未构造时为空串，消费方按 `name` 兜底）
    #[serde(default)]
    pub title: String,
    /// 语义描述（同上，可为空）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 版本（同上，可为空）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// 导航排序（来自 `PluginMeta`；未构造时用缺省值，与 `PluginMeta` 同一口径）
    #[serde(default = "default_entry_order")]
    pub order: i32,
    /// 构造者声明为必需（**不可删除**，但可停用）
    pub required: bool,
    /// 装配位（见 [`KEY_ENABLED`]）
    pub enabled: bool,
    /// 是否已挂载（= `enabled` 且构造成功）
    pub mounted: bool,
}

/// 与 [`crate::symbio_core::PluginMeta`] 的缺省 `order` **同一口径**（未构造的插件没有 meta）
///
/// 直接取 `PluginMeta::default().order` 而不是另写一个字面量：这两个数必须相等，
/// 而「必须相等」的两个字面量迟早会不等。
fn default_entry_order() -> i32 {
    crate::symbio_core::PluginMeta::default().order
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
        n.ext = Some(VDFS_EXT_FORM.to_string());
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
            // 具名写：写的就是本插件那份配置文档，名字是调用方给的
            name: None,
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
        notify_change(self.dir.name(), PLUGIN_FILE);
    }
}

/// 值 → 内容（pretty JSON，`mime = application/json`）
fn encode(value: &Value) -> VdfsResult<VdfsContent> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| VdfsError::internal(format!("配置序列化失败：{e}")))?;
    Ok(VdfsContent::text(text).with_mime("application/json"))
}

#[cfg(test)]
#[path = "plugin_dir.test.rs"]
mod tests;
