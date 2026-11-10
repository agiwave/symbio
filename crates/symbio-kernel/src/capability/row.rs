//! 能力表的**行词汇**——收集口 trait 收敛为 `put` / `get` / `list` 三方法后，
//! 一张表装下全部收集通道（DE-025 §3 的目标形状）。
//!
//! ## 族是**值**，不是内核枚举
//!
//! 族由 [`CapabilityFamily`] 表达：一个名字 + 一条槽位策略（`multi` / `single`）。
//! 行的载荷是 `Arc<dyn Any + Send + Sync>`，读侧用 [`CapabilityRow::payload`] 下转型。
//! 于是**加一族 = 声明一个 `CapabilityFamily` 常量 + 一个载荷类型**，两者都可以住在
//! 声明它的插件里；内核只保留行容器、按族过滤与单槽移交，不需要为新族加枚举臂
//! （[D-175](../../../../docs/003-design/021-横向扩充与插件切分.md "DE-021-横向扩充与插件切分")）。
//!
//! 内置六族（[`family`]）是**跨插件词汇**：工具面、模型、提示词、挂载、选项、可配置
//! 各有 ≥2 个消费方在共享地址上看见它们，故常量住本域。新族若也长到多个消费者，
//! 同样把常量搬到共享处——那是词汇的归属问题，不是内核形状问题。
//!
//! ## 槽位：单槽族把「谁在槽里」变成行的一部分
//!
//! 模型族的注册口原本是一次批量（生效者单槽 + 已启用目录），理由是
//! 「生效者必然是目录里的一员——拆成两次注册就允许了它不在目录里这个非法状态」。
//! 表只有 `put` / `get` / `list` 三个方法，于是那条不变量改由**结构**承载：生效不是
//! 一个与目录并列的槽，而是**目录里某一行的位**——[`CapabilityRow::model`] 的
//! `active` 参数。生效者不在目录里从此不可表达。
//!
//! 唯一生效由写入方维持：`put` 收到带生效位的行时，先把同族旧生效行降为普通行
//! （[`CapabilityRow::demote`]），见 `DefaultToolVisitor::put` 的单槽移交。
//!
//! ## 为什么 schema 不另存一份
//!
//! 工具行的元数据（`CapabilityMeta`）**从能力自述派生**：`CapabilityRow::tool(cap)`
//! 在入表时取一次 `cap.meta()` 的 name 作键，`row.meta()` 每次现取。两份表示（表里的
//! 键与能力自述的 name）只可能在**改名装饰器**上分叉，而那种装饰器
//! （`PrefixedCapability`）改的正是 `meta().name` 本身——现取永远同步。
//!
//! ## 谁住在这里
//!
//! 与 `CapabilityMeta` 同域同理：全部收集通道（工具 / 模型服务 / 提示词 /
//! 挂载 / 选项）共用同一张表，契约是**跨插件**的，故类型在 core；表的默认实现
//! （`DefaultToolVisitor`）在 `providers/collectors`，策略行在 loop 插件的
//! `loop_rows`——三处各住各的，见 DE-025 §3。

use super::family;
use crate::capability::{Capability, CapabilityMeta};
use crate::present::DetailField;
use crate::vdfs::{VdfsItem, VdfsProvider};
use std::any::Any;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// 根行的**保留行名**（表键）。
///
/// 根不是「一个挂载点」而是**服务者单槽**（谁服务 `<根>` 这棵子树），故它不可能从调用方
/// 拿名字——行名由 [`CapabilityRow::mount_root`] 自己填，且谁都不必知道它：读侧按
/// [`CapabilityRow::active`] 找（见 `capability/mount.rs`）。取一个不可能与插件名撞的值
/// （挂载名约定是插件名，插件名不含尖括号），它就不是任何挂载点的键。
const MOUNT_ROOT_ROW: &str = "<root>";

/// 一个能力族：**名字 + 槽位策略**。
///
/// 相等与哈希都只看名字——族身份是「这一行属于哪一族」，而槽位是**声明策略**：
/// 同名不同槽位的两份声明是登记冲突（表侧告警），不会让它们变成两个族。
#[derive(Debug, Clone, Copy)]
pub struct CapabilityFamily {
    name: &'static str,
    single: bool,
}

impl CapabilityFamily {
    /// 多槽族：同族可有任意多行（工具 / 提示词 / 选项 / 可配置）。
    pub const fn multi(name: &'static str) -> Self {
        Self {
            name,
            single: false,
        }
    }

    /// 单槽族：同族至多一行占生效位（模型 / 挂载根）。
    ///
    /// 槽位约束由表的写入实现维持（`put` 收到带生效位的行时先降同族旧行），
    /// 读侧只问 [`CapabilityRow::active`]。
    pub const fn single(name: &'static str) -> Self {
        Self { name, single: true }
    }

    /// 族名（`list` 的过滤键）。
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// 是否是单槽族。
    pub const fn is_single(self) -> bool {
        self.single
    }
}

impl PartialEq for CapabilityFamily {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}
impl Eq for CapabilityFamily {}
impl Hash for CapabilityFamily {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

/// 工具行的载荷：可调用对象（注册收口点已裹审批闸，见 `DefaultToolVisitor::put`）。
struct ToolPayload(Arc<dyn Capability>);

/// 模型行的载荷：一个**空标记**。
///
/// `D-437` 之前这里装的是 `Arc<dyn ModelProvider>`——「谁是可调用对象」那张脸让
/// 执行契约必须住内核（行是内核类型，行认得它，它就得在公共面）。`port.model`
/// 把执行面收进 model 插件之后，这一行只剩下两件事实：**它叫什么**（表键）与
/// **它带不带生效位**。协议串一度也想放进来，但查遍了：目录的选择面走
/// `contribute_options`（`OptionPayload`），执行面走 `port.model`，没有任何读者
/// 需要从行上读协议——留一个没有读者的字段，只会让下一个人以为这里有信息。
///
/// **行是数据，端口是行为**：把行为装进行，行的每一个读者都会顺手变成那个行为的
/// 读者，这正是这条 trait 当初被抬进架构的原因。空标记把这个形状钉死在这里。
struct ModelEntry;

/// 提示词行的载荷：一段文字，行名就是段名（覆盖键）。
struct PromptPayload(String);

/// 挂载行的载荷：一个 provider；根位在 [`CapabilityRow`] 上。
struct MountPayload(Arc<dyn VdfsProvider>);

/// 选项行的载荷：字段与排序号。
///
/// `order` **不下发**（见 `capability/option.rs`）：它是收集层的号段约定，
/// 读侧用它排序后即丢——长在行上而不是长在另一个槽位键里，正是因为排序答案
/// 同样是「这一行自身携带的事实」。
struct OptionPayload {
    field: Box<DetailField>,
    order: i32,
}

/// 可配置行的载荷：整项 `VdfsItem`（地址必须连条目一起交出来，见
/// `capability/configurable.rs`）。
struct ConfigurablePayload(VdfsItem);

/// 表里的一行。
///
/// 四个字段各是一个事实：**族**（过滤键）、**名字**（表键）、**生效位**（单槽族的
/// 「谁在槽里」）与**载荷**（这一行到底是什么）。载荷是开放类型：新族自带载荷类型，
/// 读侧用 [`Self::payload`] 下转型，内核不认识它。
///
/// 构造只走本文件的具名构造器，或新族自己的 `CapabilityRow::new`：
/// 工具与模型行的名字从载荷自述派生（能力的 `meta().name` / provider 的
/// `provider_id()`），提示词行的名字是调用方给的**覆盖键**，调用方没有别的口子可填。
#[derive(Clone)]
pub struct CapabilityRow {
    family: CapabilityFamily,
    name: String,
    active: bool,
    payload: Arc<dyn Any + Send + Sync>,
}

impl CapabilityRow {
    /// **开放构造口**：新族按 `(family, name, payload)` 造行，内核不改。
    ///
    /// 载荷是任意 `Send + Sync + 'static` 的值；读侧必须用同一个具体类型
    /// [`Self::payload`] 取回——类型是声明方与消费方之间的私有约定，表不参与校验
    /// （校验它们是否同族的是 [`Self::family`]）。
    pub fn new<T>(
        family: CapabilityFamily,
        name: impl Into<String>,
        payload: T,
        active: bool,
    ) -> Self
    where
        T: Any + Send + Sync,
    {
        Self {
            family,
            name: name.into(),
            active,
            payload: Arc::new(payload),
        }
    }

    /// 工具行：名字 = `cap.meta().name`（全名，含挂载点前缀）。
    pub fn tool(cap: Arc<dyn Capability>) -> Self {
        let name = cap.meta().name;
        Self::new(family::TOOL, name, ToolPayload(cap), false)
    }

    /// 模型行：名字 = `provider_id`，载荷是空标记（**不装执行对象**，`D-437`）。
    ///
    /// `active` 只由**写出整批的那一方**填（model 插件的 traverse）：它知道这一批里
    /// 哪个是当次生效者。表把「唯一生效」接过去，见 [`Self::demote`]。
    ///
    /// 两个入参都是**数据**：目录项的 id 与它带不带位。这一行因此可以被任何只持有
    /// 目录快照的写方构造——不需要先有一个能跑的 provider 实例。
    pub fn model(provider_id: &str, active: bool) -> Self {
        Self::new(family::MODEL, provider_id.to_string(), ModelEntry, active)
    }

    /// 系统提示词行：名字 = 段名（覆盖键）。
    pub fn prompt(name: &str, text: String) -> Self {
        Self::new(family::PROMPT, name, PromptPayload(text), false)
    }

    /// 选项字段行：名字 = `field.key`（同键覆盖，保留先注册槽位）。
    pub fn option(order: i32, field: DetailField) -> Self {
        let name = field.key.clone();
        Self::new(
            family::OPTION,
            name,
            OptionPayload {
                field: Box::new(field),
                order,
            },
            false,
        )
    }

    /// 可配置声明行：名字 = `item.node.name`（目录名，同名覆盖）。
    pub fn configurable(item: VdfsItem) -> Self {
        let name = item.node.name.clone();
        Self::new(family::CONFIGURABLE, name, ConfigurablePayload(item), false)
    }

    /// 根行：**服务者单槽**（唯一带根位的那一行）。
    ///
    /// 根没有调用方给的名字（它不是某个插件的目录），行名由本构造函数填保留值；
    /// 读侧一律按 [`Self::active`] 找，不按名找。
    pub fn mount_root(provider: Arc<dyn VdfsProvider>) -> Self {
        Self::new(family::MOUNT, MOUNT_ROOT_ROW, MountPayload(provider), true)
    }

    /// 这一行属于哪一族。
    pub fn family(&self) -> CapabilityFamily {
        self.family
    }

    /// 行的名字（表键；工具族即线上全名）。
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 把载荷下转型成声明方给的具体类型（新族的读法）。
    ///
    /// 取错类型 ⇒ `None`，不 panic：族与载荷的配对是声明方的约定，读错就是读错了
    /// 族（先问 [`Self::family`]）。
    pub fn payload<T: Any>(&self) -> Option<&T> {
        self.payload.downcast_ref::<T>()
    }

    /// 工具元数据（每次现取，见模块文档「为什么 schema 不另存一份」）。
    pub fn meta(&self) -> CapabilityMeta {
        self.cap().map(|c| c.meta()).unwrap_or_default()
    }

    /// 取可调用对象；非工具族为 `None`（调用方据此给诚实的 NotFound）。
    pub fn cap(&self) -> Option<&Arc<dyn Capability>> {
        self.payload::<ToolPayload>().map(|p| &p.0)
    }

    /// 取系统提示词正文；非提示词族为 `None`。
    pub fn prompt_text(&self) -> Option<&str> {
        self.payload::<PromptPayload>().map(|p| p.0.as_str())
    }

    /// 取选项字段与它的排序号；非选项族为 `None`。
    pub fn option_entry(&self) -> Option<(i32, &DetailField)> {
        self.payload::<OptionPayload>()
            .map(|p| (p.order, p.field.as_ref()))
    }

    /// 取可配置声明；非可配置族为 `None`。
    pub fn configurable_item(&self) -> Option<&VdfsItem> {
        self.payload::<ConfigurablePayload>().map(|p| &p.0)
    }

    /// 取 VDFS 挂载点（或根）；非挂载族为 `None`。
    pub fn vdfs(&self) -> Option<&Arc<dyn VdfsProvider>> {
        self.payload::<MountPayload>().map(|p| &p.0)
    }

    /// 这一行是否占着**生效位**（单槽族的「谁在槽里」）。
    ///
    /// 两个内置族有位：模型族（谁在生效槽里）与挂载族（谁是 `<根>` 的服务者）。
    /// 多槽族恒 `false`；新族若声明为 `single`，自行用 `active` 标记占位行。
    pub fn active(&self) -> bool {
        self.active
    }

    /// 同一行的**退位副本**（生效位清零）。
    ///
    /// 单槽族的写入方在收下新生效行前用它降下旧行——表的行是 `Arc`，就地改位等于
    /// 让读者看见两次变化；换一个副本入表，读出的一直是一份完整的行。
    pub fn demote(&self) -> Self {
        Self {
            active: false,
            ..self.clone()
        }
    }
}

#[cfg(test)]
#[path = "row.test.rs"]
mod tests;
