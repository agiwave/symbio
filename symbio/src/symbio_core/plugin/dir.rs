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
//! 插件根**不额外嵌套一层**：容器扫描的那一层既不承载语义、又让每个插件的路径深
//! 一段，而装配根下本来就是「一个插件一个目录」的扁平结构，再加一层纯属重复。
//!
//! 这同时消掉一个自举环：若 `home` 住在 `<homedir>/home`，容器扫描插件根时会把它
//! 当成普通插件再构造一次，而那个 `home` 又会构造容器……系统级插件不参与扫描。
//!
//! ## 插件不认识全局布局
//!
//! 插件**不知道、也不该知道**自己被放在哪。目录一律由容器经 `PLUGIN_DIR` 告知
//! （[`plugin_dir_from_ctx`]），插件只持有 [`PluginDir`] 并向下传：
//!
//! - ❌ 不要按插件名反推落位（`HomedirRegistry::get().join(PLUGIN_X)` 或等价的
//!   「类别根 + 类别名」拼法）——那等于把「装配决策」写死进插件，插件挪个位置就全错；
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
//! plugin_provider: web      # 装配方：工厂 id（据此构造）
//! plugin_name: web          # 装配方：实例名；缺省 = 目录名
//! plugin_title: 网络工具      # 身份：展示标题（装配期从 Plugin::meta() 投影一次）
//! plugin_description: …      # 身份：语义描述
//! plugin_version: 0.1.0     # 身份：版本
//! web_enabled: true         # 以下都是该插件自己的配置
//! web_timeout: 30
//! ```
//!
//! - **加载判据**（由装配方 `composite` 执行）：文件存在、可解析、且
//!   `plugin_provider` 指向一个已注册的工厂（[`creator_has`](crate::symbio_core::creator_has)）。
//! - **保留键是装配方的**：见 [`PLUGIN_RESERVED_KEYS`]（单一清单）——它们不参与插件配置的
//!   反序列化（[`PluginDir::load`] 剥离），插件配置也不得占用同名键；写入时
//!   （[`PluginDir::save`] 等）自动补回。
//! - **身份键是 manifest 的**：出厂声明在 `Plugin::meta()`，装配期经
//!   [`PluginDir::seed_identity`] 投影一次；此后 manifest 权威，运行期经
//!   [`PluginDir::identity`] 读——**停用的插件因此也有身份**（ADR-032）。
//!
//! ## 职责划分（本方案的全部要点）
//!
//! | 谁 | 负责什么 |
//! |---|---|
//! | 容器（`composite`） | 发现插件目录、构造插件、把目录告知插件 |
//! | 插件自己 | 读写自己的 `PLUGIN.yml` |
//! | 父插件 | **什么都不负责**——不持有 / 不合并 / 不转发子插件配置 |
//!
//! 配置**回到拥有者手上**：插件目录里的一个文件，谁写谁读。父插件不必认识「合并
//! 规则」，容器也不必认识「分发规则」。
//!
//! ## 与 VDFS 的关系
//!
//! [`PluginConfigFile`] 只做两件事：把配置文件**当作一个文件**呈现（`ext = form` +
//! `schema` = 定义），并在写入前按定义校验。它不持有生命周期、不认识父插件、
//! 不向上推任何东西——「写完之后还要做什么」（重启监听 / 重建缓存）留在插件
//! 自己的 `write` 里。

use crate::symbio_core::schemas::detail::DetailDefinition;
use crate::symbio_core::vdfs_notify_change;
use crate::symbio_core::{
    VdfsAccess, VdfsContent, VdfsError, VdfsNode, VdfsResult, VdfsWriteResponse, VDFS_EXT_FORM,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

/// 插件配置文件名（插件目录下）
pub const PLUGIN_FILE: &str = "PLUGIN.yml";

// 一个插件 = 一个目录：装配根下**不额外嵌套一层**（那一层既不承载语义、也不参与
// 寻址，只把每个插件的路径都加深一段）。
//
// ⚠️ **core 不认识任何「系统根」**：插件的目录一律由父插件经 `PLUGIN_DIR` 告知
//（[`plugin_dir_from_ctx`]）。这里**没有** `plugins_root` / `dir_of` 这类按插件名反推落位
// 的函数——它们会读全局 homedir，在子智能体里必然指错作用域（homedir 归 `home` 独有）。

// ==================== 保留键（单一清单见 `PLUGIN_RESERVED_KEYS`） ====================
//
// 「保留键」= `PLUGIN.yml` 里属于**装配方**的键，插件自己的配置不得占用同名键。
// 读写时的「剥离 / 保留」都遍历 [`PLUGIN_RESERVED_KEYS`]，新增保留键只改那一处。

/// 保留键：工厂 id（构造插件用）
pub const PLUGIN_KEY_PROVIDER: &str = "plugin_provider";
/// 保留键：实例名（缺省 = 目录名）
pub const PLUGIN_KEY_NAME: &str = "plugin_name";
/// 保留键（**装配位**）：`false` = 停用（缺省 / 键不存在 = 启用）
///
/// 与 [`PLUGIN_KEY_PROVIDER`] / [`PLUGIN_KEY_NAME`] 同类——它不是插件自己的配置，而是
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
pub const PLUGIN_KEY_ENABLED: &str = "plugin_enabled";

// ==================== 身份键：manifest 是运行期唯一来源（ADR-032） ====================
//
// 出厂声明在 `Plugin::meta()`，装配期由容器经 [`PluginDir::seed_identity`] **投影**
// 进 manifest（键缺失才写，用户改过的不覆盖）。此后 manifest 权威：插件列表、
// 挂载点目录标题都读它（[`PluginDir::identity`]）。
//
// 因此**停用的插件也有身份**——它只是不被构造，不是不存在。这正是把身份从
// `PluginMeta`（构造物）搬到这里的原因：`order` / `hidden` 那类「挂载点呈现」没有
// 挂载点就无从谈起，而「这个插件叫什么」与它开没开无关。

/// 身份键：展示标题 / 挂载点标题（原 `PluginMeta::name`）
pub const PLUGIN_KEY_TITLE: &str = "plugin_title";
/// 身份键：语义描述（原 `PluginMeta::description`）
pub const PLUGIN_KEY_DESCRIPTION: &str = "plugin_description";
/// 身份键：版本（原 `PluginMeta::version`）
///
/// 与另两个投影键（[`PLUGIN_KEY_REQUIRED`] / [`PLUGIN_KEY_CAN_DISABLE`]）不同，它是**真键**：
/// 落在 manifest 里，只是顺带被投影进插件管理插件的表单模型——投影值与真值
/// **同源**，不再绕经 `PluginMeta`。
pub const PLUGIN_KEY_VERSION: &str = "plugin_version";
/// 身份键：作者（原 `PluginMeta::author`）
pub const PLUGIN_KEY_AUTHOR: &str = "plugin_author";

// ==================== 第三方插件保留键（A5：本期只定义与解析，不强制） ====================

/// 保留键：要求的**宿主插件 API 版本**（见 design/third-party-plugin-spec.md §4）
///
/// 宿主必须在**启动子进程之前**就知道「这个插件我认不认识」——否则要么盲目启动
/// （可能挂），要么启动后才发现不兼容（已产生副作用）。
pub const PLUGIN_KEY_API: &str = "plugin_api";
/// 保留键：宿主**授予**的能力（闭集见 design/third-party-plugin-spec.md §7）
///
/// 权限是**宿主**的决定，必须在宿主侧可审计。「插件**需要**什么」不在这里——
/// 那是插件自己在 `init` 响应里声明的；两者分开，才能有「授予 < 需要」这个可检测状态。
pub const PLUGIN_KEY_GRANTS: &str = "plugin_grants";

/// 全部保留键 —— **单一清单**
///
/// [`PluginDir::load`] 遍历它**剥离**（这些键不属于插件配置）；`save` /
/// `set_enabled` 遍历它**保留**（写配置不得冲掉装配方的状态）。
/// 新增一个保留键只需加进这里 + 定义常量。
///
/// 注意 [`PLUGIN_KEY_REQUIRED`] / [`PLUGIN_KEY_CAN_DISABLE`] **不在**此列：它们只注入表单模型、
/// 从不落盘，因此不是 manifest 的键。
pub const PLUGIN_RESERVED_KEYS: &[&str] = &[
    PLUGIN_KEY_PROVIDER,
    PLUGIN_KEY_NAME,
    PLUGIN_KEY_ENABLED,
    PLUGIN_KEY_TITLE,
    PLUGIN_KEY_DESCRIPTION,
    PLUGIN_KEY_VERSION,
    PLUGIN_KEY_AUTHOR,
    PLUGIN_KEY_API,
    PLUGIN_KEY_GRANTS,
];

// ==================== 装配态在**配置表单模型**里的投影键 ====================
//
// 插件管理插件的条目表单，其模型就是那个插件的配置（`vdfs/read` 的结果）。但条目上
// 的按钮（启用 / 停用 / 卸载）要按**装配态**显隐，而 `DetailAction.when` 只能对表单
// 模型求值（见 `schemas/detail.rs`）——于是装配态得一并放进那份模型。
//
// 这些键在 `PLUGIN.yml` 里同样是**保留键**（插件配置不得占用），而表单保存时只回传
// **定义声明过的字段**（见前端 `DetailForm.buildValues`），因此注入它们既不会显示成
// 字段，也不会写进配置文件。
//
// 键名与它投影的来源**逐字对应**，改一处就能顺着找到另一处：
// `plugin_version` ← `PluginEntry::version`（**即 [`PLUGIN_KEY_VERSION`] 的真值**）、
// `plugin_required` ← `PluginEntry::required`、
// `plugin_can_disable` ← `ASSEMBLY_UNDISABLABLE_PLUGINS` 的补集。

/// 投影键：构造者是否声明为必需（`PluginEntry::required`）——必需即**不可删除**
pub const PLUGIN_KEY_REQUIRED: &str = "plugin_required";
/// 投影键：是否允许停用（= 不在 [`crate::symbio_core::ASSEMBLY_UNDISABLABLE_PLUGINS`] 里）
pub const PLUGIN_KEY_CAN_DISABLE: &str = "plugin_can_disable";

/// 从插件上下文读自己的目录（装配方经 [`PLUGIN_DIR`](crate::symbio_core::PLUGIN_DIR) 告知）
///
/// 插件构造时的**标准入口**：目录由父插件传下，插件不查任何全局落位。
///
/// **没有 `PLUGIN_DIR` 是装配缺陷**（调用方拿的是请求级 ctx 而非装配 ctx，
/// 或父插件没传），因此这里**不回退**到任何「常规位置」——回退必然读全局
/// homedir，在子智能体里指错作用域。运行时若需要自己的目录，用插件持有
/// 的 `self.dir`（构造时经本函数取得），不要拿请求 ctx 再取一次。
pub fn plugin_dir_from_ctx(
    ctx: &dyn crate::symbio_core::PluginInvokeRequest,
    plugin: &str,
) -> PluginDir {
    use crate::symbio_core::{PluginInvokeRequestExt, PLUGIN_DIR};
    match ctx.get(PLUGIN_DIR) {
        Some(dir) => dir,
        None => panic!(
            "插件 `{plugin}` 缺少 PLUGIN_DIR：目录必须由父插件经 PLUGIN_DIR 告知\
             （运行时请用插件持有的 self.dir，不要拿请求 ctx 再取）"
        ),
    }
}

/// 展开 `~` 前缀到用户主目录
///
/// **纯函数**：只读操作系统用户主目录（`dirs::home_dir`），不读任何全局「系统根」。
/// 因此它可以留在 core 供各插件共用，而 `homedir` 注册表不能。
pub fn plugin_expand_tilde_path(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if s == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    }
    if let Some(stripped) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(s.as_ref())
}

// ==================== 插件身份 ====================

/// 一个插件的**身份** —— 从 `PLUGIN.yml` 读出的四个身份键（ADR-032）
///
/// 与 [`PluginMeta`](crate::symbio_core::PluginMeta) 的关系：`PluginMeta` 是插件的
/// **出厂自述**（代码里声明的），本结构是它的**落盘形态**（manifest 里的值）。
/// 运行期一切消费（插件列表 / 挂载点目录节点）都读本结构——因此**停用的插件也有
/// 身份**：它只是不被构造，不是不存在。
///
/// 各字段都可能缺省：manifest 缺失 / 不可解析 / **从未落位过**（新装但从未构造成功）
/// 时全为空，消费方按目录名兜底。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginIdentity {
    /// 展示标题 / 挂载点标题（缺省 → 消费方用目录名）
    pub title: String,
    /// 语义描述
    pub description: Option<String>,
    /// 版本
    pub version: Option<String>,
    /// 作者
    pub author: Option<String>,
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
    /// 显式指定目录（唯一构造入口：目录一律由调用方给出）
    ///
    /// 目录来自父插件的 `PLUGIN_DIR`（[`plugin_dir_from_ctx`]）或测试构造里的临时目录；
    /// core 不提供「按插件名反推落位」的 `of`——那会读全局 homedir。
    pub fn at(dir: impl Into<PathBuf>, name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            dir: dir.into(),
            provider: name.clone(),
            name,
        }
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
    /// **保留键被剥离**后再反序列化——它们属于「这是哪个插件、它叫什么、它开没开」，
    /// 不属于配置。清单见 [`PLUGIN_RESERVED_KEYS`]（一处定义，新增键不必改这里）。
    pub fn load<C: DeserializeOwned>(&self) -> Result<Option<C>, String> {
        let Some(mut map) = self.read_manifest()? else {
            return Ok(None);
        };
        for key in PLUGIN_RESERVED_KEYS {
            map.remove(*key);
        }
        serde_json::from_value(Value::Object(map))
            .map(Some)
            .map_err(|e| {
                format!(
                    "{} 与当前版本的配置结构不符：{e}",
                    self.config_path().display()
                )
            })
    }

    // ==================== 身份（ADR-032） ====================

    /// 读本插件的身份 —— manifest 是运行期**唯一**来源
    ///
    /// 与 [`enabled`](Self::enabled) 同一口径：文件缺失 / 不可读 / 不可解析一律按
    /// 「无身份」处理（各字段取缺省），由消费方按目录名兜底——「读不出来」不是本判据
    /// 该报警的事。
    ///
    /// 之所以能从 manifest 读（而不问 `Plugin::meta()`）：**停用的插件不被构造**，
    /// 而插件列表必须能显示它的名字。见 ADR-032。
    pub fn identity(&self) -> PluginIdentity {
        let map = self.read_manifest().ok().flatten().unwrap_or_default();
        PluginIdentity {
            title: string_of(&map, PLUGIN_KEY_TITLE),
            description: optional_string_of(&map, PLUGIN_KEY_DESCRIPTION),
            version: optional_string_of(&map, PLUGIN_KEY_VERSION),
            author: optional_string_of(&map, PLUGIN_KEY_AUTHOR),
        }
    }

    /// 装配期**一次性投影**出厂身份（`Plugin::meta()`）进 manifest
    ///
    /// **只补缺失的键**：manifest 是权威，用户改过的标题 / 描述不被出厂声明覆盖。
    /// 键都在时**不碰文件**——无谓的写盘会搅乱 mtime，而变更通知的消费者只关心
    /// 真正的变化。
    ///
    /// 调用点在容器的装配路径（子插件构造**成功之后**）——那时手里才有 `PluginMeta`。
    /// 停用的插件不被构造、因此不会走到这里；它的身份早在首次装配时就已落位，
    /// 这正是「停用后仍显示名字」的来路。
    pub fn seed_identity(&self, meta: &crate::symbio_core::PluginMeta) -> Result<(), String> {
        let mut map = self.read_manifest()?.unwrap_or_default();
        let mut changed = false;

        // 逐字段「缺失才补」——与 `identity()` 的字段一一对应，顺序即优先级
        let seeds: [(&str, Option<&str>); 4] = [
            (PLUGIN_KEY_TITLE, Some(meta.name.as_str())),
            (PLUGIN_KEY_DESCRIPTION, meta.description.as_deref()),
            (PLUGIN_KEY_VERSION, meta.version.as_deref()),
            (PLUGIN_KEY_AUTHOR, meta.author.as_deref()),
        ];
        for (key, value) in seeds {
            if map.contains_key(key) {
                continue;
            }
            let Some(text) = value.filter(|s| !s.is_empty()) else {
                continue;
            };
            map.insert(key.to_string(), Value::String(text.to_string()));
            changed = true;
        }

        if !changed {
            return Ok(());
        }
        self.carry_over_reserved(&mut map);
        self.write_map(&map)
    }

    /// 写盘前的统一收尾：**保留键恒在**
    ///
    /// - `plugin_provider` / `plugin_name` 强制为当前值（实例名 = 目录名，由装配方
    ///   决定，不由文件内容决定）；
    /// - 其余保留键从**磁盘上的旧值**取回——插件写自己的配置时不得冲掉装配方的状态
    ///   （身份 / 装配位 / 授予），否则「在设置页保存一次配置」就等于把停用的插件
    ///   启用了，或把用户改过的标题还原成出厂值。
    ///
    /// ⚠️ 想**主动改**某个保留键的调用方（如 [`set_enabled`](Self::set_enabled)）
    /// 必须在本方法**之后**写那一个键。
    fn carry_over_reserved(&self, map: &mut Map<String, Value>) {
        if let Ok(Some(existing)) = self.read_manifest() {
            for key in PLUGIN_RESERVED_KEYS {
                if *key == PLUGIN_KEY_PROVIDER || *key == PLUGIN_KEY_NAME {
                    continue; // 下面强制写当前值
                }
                if let Some(v) = existing.get(*key) {
                    map.insert((*key).to_string(), v.clone());
                }
            }
        }
        map.insert(
            PLUGIN_KEY_PROVIDER.to_string(),
            Value::String(self.provider.clone()),
        );
        map.insert(
            PLUGIN_KEY_NAME.to_string(),
            Value::String(self.name.clone()),
        );
    }

    // ==================== 装配位 ====================

    /// 本插件是否启用（见 [`PLUGIN_KEY_ENABLED`]）。
    ///
    /// 文件缺失 / 不可读 / 不可解析一律按**启用**处理：「读不出来」不该把一个插件
    /// 静默停掉——那是装配期告警的职责，不是本判据的（否则一次磁盘故障会让整棵树
    /// 的插件集体隐形，而日志里只有解析错误）。
    pub fn enabled(&self) -> bool {
        match self.read_manifest() {
            Ok(Some(map)) => !matches!(map.get(PLUGIN_KEY_ENABLED), Some(Value::Bool(false))),
            _ => true,
        }
    }

    /// 写装配位：`true` = 启用（摘掉键，保持文件干净）；`false` = 停用。
    ///
    /// 只动这一个键，其余内容原样保留（身份 / 授予等保留键经
    /// [`carry_over_reserved`](Self::carry_over_reserved) 取回；手写的 manifest
    /// 可能漏了身份字段，那里也会补上）。
    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        let mut map = self.read_manifest()?.unwrap_or_default();
        // 顺序不能反：carry 会把磁盘上的旧 `plugin_enabled` 取回来
        self.carry_over_reserved(&mut map);
        if enabled {
            map.remove(PLUGIN_KEY_ENABLED);
        } else {
            map.insert(PLUGIN_KEY_ENABLED.to_string(), Value::Bool(false));
        }
        self.write_map(&map)
    }

    // ==================== 写 ====================

    /// 原子写插件配置（保留键自动补回）
    ///
    /// 同步实现：配置文件只有几百字节，且 `composite::build` 本身在同步上下文里
    /// 补默认配置——一处实现能同时服务装配期与运行期，不值得为此分两份。
    ///
    /// **保留键随写保留**（见 [`PLUGIN_RESERVED_KEYS`]）：身份 / 装配位 / 授予都不是插件的
    /// 配置，插件写自己的配置时不得把它们冲掉——否则「在设置页里保存一次配置」就等于
    /// 悄悄把停用的插件启用了、或把用户改过的标题还原成出厂值。
    pub fn save<C: Serialize>(&self, value: &C) -> Result<(), String> {
        let mut map =
            match serde_json::to_value(value).map_err(|e| format!("配置序列化失败：{e}"))? {
                Value::Object(m) => m,
                other => {
                    return Err(format!("配置必须是映射，实得 {other}"));
                }
            };
        self.carry_over_reserved(&mut map);
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
            PLUGIN_KEY_PROVIDER.to_string(),
            Value::String(self.provider.clone()),
        );
        map.insert(
            PLUGIN_KEY_NAME.to_string(),
            Value::String(self.name.clone()),
        );
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
/// ## 身份从 manifest 读，不依赖「被构造」
///
/// `title` / `description` / `version` 来自 `PLUGIN.yml` 的身份键
/// （[`PluginDir::identity`]），因此**停用的插件也有身份**——它只是不被构造，
/// 不是不存在（ADR-032）。
///
/// 字段仍可能为空：该目录**从未落位过**（新装 / 手写，但从未构造成功 ⇒ 出厂身份
/// 还没被投影）。那时消费方按 `name`（目录名）兜底。
///
/// `order` 是另一回事：它是**挂载点呈现**（仍在 `PluginMeta` 上），没有挂载点就
/// 无从谈起，因此停用时取缺省值——这与身份「必须在」正好相反，是刻意的。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PluginEntry {
    /// 插件名 = 目录名 = 挂载名（路由前缀）
    pub name: String,
    /// 工厂 id（`PLUGIN.yml` 的 `plugin_provider`）
    pub provider: String,
    /// 展示标题（来自 manifest 的身份键；从未落位时为空串，消费方按 `name` 兜底）
    #[serde(default)]
    pub title: String,
    /// 语义描述（同上，可为空）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 版本（同上，可为空）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// 导航排序（**挂载点呈现**，来自 `PluginMeta`；未挂载时用缺省值，同一口径）
    #[serde(default = "default_entry_order")]
    pub order: i32,
    /// 构造者声明为必需（**不可删除**，但可停用）
    pub required: bool,
    /// 装配位（见 [`PLUGIN_KEY_ENABLED`]）
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
/// if path == PLUGIN_FILE { return self.config_file.read(&self.config).await; }
/// // 写入侧
/// if path == PLUGIN_FILE {
///     let resp = self.config_file.apply(&self.config, content).await?;
///     self.apply_side_effects().await;   // 仅需要副作用的插件（如重启监听）
///     return Ok(resp);
/// }
/// ```
#[derive(Debug, Clone)]
pub struct PluginConfigFile {
    /// 配置文件所在的插件目录
    dir: PluginDir,
    /// 展示标题（如「网络工具设置」）
    label: String,
    /// 字段定义（呈现与校验同源）
    definition: DetailDefinition,
}

impl PluginConfigFile {
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
        vdfs_notify_change(self.dir.name(), PLUGIN_FILE);
    }
}

/// 值 → 内容（pretty JSON，`mime = application/json`）
fn encode(value: &Value) -> VdfsResult<VdfsContent> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| VdfsError::internal(format!("配置序列化失败：{e}")))?;
    Ok(VdfsContent::text(text).with_mime("application/json"))
}

/// 取字符串值（缺失 / 非字符串 → 空串）
fn string_of(map: &Map<String, Value>, key: &str) -> String {
    match map.get(key) {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// 取可选字符串值（缺失 / 非字符串 / 空串 → `None`）
///
/// 空串按「没有」处理：手写的 manifest 里 `plugin_version: ""` 与不写这个键
/// 是同一个意思，消费方不该为前者多一个分支。
fn optional_string_of(map: &Map<String, Value>, key: &str) -> Option<String> {
    match map.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "dir.test.rs"]
mod tests;
