//! 能力表的**行词汇**——收集口 trait 收敛为 `put` / `get` / `list` 三方法后，
//! 一张表装下全部收集通道（DE-008 §3 的目标形状）。
//!
//! ## 族是**一个一个**落位的（Tool ✓ → Model ✓ → Prompt ✓ → Mount ✓ → Option ✓ → Configurable ✓）
//!
//! [`CapabilityRowKind`] 现有六个值，**且只装得住「在共享地址上被 ≥2 个消费方看见」的族**
//! （[D-089](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")：`Policy` 变体已删除——策略的判据依赖 loop 私有的
//! 运行时视图，它接不上这张表，不是「还没接上」），
//! [`CapabilityRowPayload`] 随各族落地长变体：4a `Tool`、4b `Model`、4c `Prompt`、
//! 4d `Mount`、4e `Option` + `Configurable`（连同两个专服单族的 trait 退场）。
//! 提前把值写齐的理由：`list(kind)` 的签名由此在 4b–4e **不再变形**（加一个族
//! = 加一个 payload 变体，不是加三个方法）；而不写齐则每一步都要改签名，
//! 正是十六方法糜烂的来路。
//!
//! ## 为什么 schema 不另存一份
//!
//! 工具行的元数据（`CapabilityMeta`）**从能力自述派生**：`Row::tool(cap)` 在入表
//! 时取一次 `cap.meta()` 的 name 作键，`row.meta()` 每次现取。两份表示（表里的
//! 键与能力自述的 name）只可能在**改名装饰器**上分叉，而那种装饰器
//! （`PrefixedCapability`）改的正是 `meta().name` 本身——现取永远同步。
//!
//! ## 生效位（`active`）：单槽族把「谁在槽里」也变成行的一部分
//!
//! 模型族的注册口原本是**一次批量**（生效者单槽 + 已启用目录），理由是
//! 「生效者必然是目录里的一员——拆成两次注册就允许了它不在目录里这个非法状态」。
//! 表只有 `put` / `get` / `list` 三个方法，于是那条不变量改由**结构**承载：生效不是
//! 一个与目录并列的槽，而是**目录里某一行的位**——[`CapabilityRow::model`] 的
//! `active` 参数。生效者不在目录里从此不可表达。
//!
//! 唯一生效由写入方维持：`put` 收到带生效位的行时，先把同族旧生效行降为普通行
//! （[`CapabilityRow::demote`]），见 `DefaultToolVisitor::put` 的单槽移交。
//!
//! ## 谁住在这里
//!
//! 与 `CapabilityMeta` 同域同理：全部收集通道（工具 / 模型服务 / 提示词 /
//! 挂载 / 选项）共用同一张表，契约是**跨插件**的，故类型在 core；表的默认实现
//! （`DefaultToolVisitor`）在 `providers/collectors`，策略行在 loop 插件的
//! `loop_rows`——三处各住各的，见 DE-008 §3。

use crate::symbio_core::DetailField;
use crate::symbio_core::{Capability, CapabilityMeta, ModelProvider, VdfsItem, VdfsProvider};
use std::sync::Arc;

/// 根行的**保留行名**（表键）。
///
/// 根不是「一个挂载点」而是**服务者单槽**（谁服务 `<根>` 这棵子树），故它不可能从调用方
/// 拿名字——行名由 [`CapabilityRow::mount_root`] 自己填，且谁都不必知道它：读侧按
/// [`CapabilityRow::active`] 找（见 `capability/mount.rs`）。取一个不可能与插件名撞的值
/// （挂载名约定是插件名，插件名不含尖括号），它就不是任何挂载点的键。
const MOUNT_ROOT_ROW: &str = "<root>";

/// 行的族——`list(kind)` 的过滤键，也是「这张表装了什么」的清单。
///
/// 六个值的落位时序见本模块文档；本批只有 [`CapabilityRowKind::Tool`] 可达。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CapabilityRowKind {
    /// 能力行：LLM 可见工具（可调用对象）。
    Tool,
    /// 模型服务（生效者单槽 + 已启用目录）。
    Model,
    /// 系统提示词。
    Prompt,
    /// VDFS 挂载点。
    Mount,
    /// 选项字段（会话输入区的可选项）。
    Option,
    /// 可配置声明（设置页里的一份配置条目）。
    ///
    /// 4e 随 `Option` 一并入表：此前它住独立的 `ConfigurableVisitor`，两个 trait
    /// 各 2 方法专服一个族——正是十六方法糜烂的同一条来路，收敛后并入本表。
    Configurable,
}

/// 行的载荷——「这一行到底是什么」。
///
/// 每个族在此加一个变体，**不在 trait 上加方法**——后者是十六方法形态的成因。
#[derive(Clone)]
pub enum CapabilityRowPayload {
    /// 工具：可调用对象（注册收口点已裹审批闸，见 `DefaultToolVisitor::put`）。
    Tool(Arc<dyn Capability>),
    /// 模型服务：一个已启用的 provider。
    ///
    /// `active` 是**生效位**（单槽族的「谁在槽里」）：会话引擎取的是它，插件按 id
    /// 取的是不带位的行——两者同表，故「生效者不在目录里」不可表达（见模块文档）。
    Model {
        provider: Arc<dyn ModelProvider>,
        active: bool,
    },
    /// 系统提示词：一段文字，行名就是段名。
    ///
    /// 段名是**覆盖键**，不是分类——不要用命名前缀去表达「这是人格」「这是记忆」
    /// （`capability/prompt.rs` 的模块文档展开这条）。
    Prompt { text: String },
    /// VDFS 挂载点：一个 provider。
    ///
    /// `active` 是**根位**（单槽）：置位的行是「服务 `<根>` 这棵子树的那一个」。
    /// 挂载点与根同为「目录名 → provider」的映射，差别只在根**不可能由调用方命名**
    /// （它不是某个插件的目录），故同族同载荷、以位相区分——又一条「生效位长在行上」
    /// 的例子，见 [`CapabilityRow::mount_root`]。
    Mount {
        provider: Arc<dyn VdfsProvider>,
        active: bool,
    },
    /// 选项字段：会话配置表单的一个字段，行名 = `field.key`（覆盖键）。
    ///
    /// `order` **不下发**（见 `capability/option.rs`）：它是收集层的号段约定，
    /// 读侧用它排序后即丢——长在行上而不是长在另一个槽位键里，正是因为排序答案
    /// 同样是「这一行自身携带的事实」。
    Option { field: Box<DetailField>, order: i32 },
    /// 可配置声明：设置页里的一项，行名 = `item.node.name`（目录名，覆盖键）。
    ///
    /// 地址必须连条目一起交出来（跨挂载点、推不出来），故整项入行——
    /// 理由见 `capability/configurable.rs` 模块文档。
    Configurable(Box<VdfsItem>),
}

/// 表里的一行。
///
/// 两个字段，各是一个事实：**名字**（表键）与**载荷**（这一行到底是什么）。
/// 族别不单独存——[`Self::kind`] 从载荷派生，结构上不存在「两种答案」（若另存
/// 一格，就得防它俩哪天不一致，而那正是不一致的定义）。
///
/// 构造只走 [`CapabilityRow::tool`] / [`CapabilityRow::model`] / [`CapabilityRow::prompt`]：
/// 工具与模型行的名字从载荷自述派生（能力的 `meta().name` / provider 的 `provider_id()`），
/// 提示词行的名字是调用方给的**覆盖键**，调用方没有别的口子可填。
/// 读取侧三件套——[`Self::kind`] / [`Self::meta`] / [`Self::cap`] 覆盖工具族的
/// 全部现行消费形态（列表喂模型、按名执行、声明位读取），模型族走 [`Self::provider`]。
#[derive(Clone)]
pub struct CapabilityRow {
    name: String,
    payload: CapabilityRowPayload,
}

impl CapabilityRow {
    /// 工具行：名字 = `cap.meta().name`（全名，含挂载点前缀）。
    pub fn tool(cap: Arc<dyn Capability>) -> Self {
        let name = cap.meta().name;
        Self {
            name,
            payload: CapabilityRowPayload::Tool(cap),
        }
    }

    /// 模型行：名字 = `provider.provider_id()`。
    ///
    /// `active` 只由**写出整批的那一方**填（`capability::model::register`）：它知道
    /// 这一批里哪个是当次生效者。表把「唯一生效」接过去，见 [`Self::demote`]。
    pub fn model(provider: Arc<dyn ModelProvider>, active: bool) -> Self {
        let name = provider.provider_id().to_string();
        Self {
            name,
            payload: CapabilityRowPayload::Model { provider, active },
        }
    }

    /// 系统提示词行：名字 = 段名（覆盖键）。
    pub fn prompt(name: &str, text: String) -> Self {
        Self {
            name: name.to_string(),
            payload: CapabilityRowPayload::Prompt { text },
        }
    }

    /// 选项字段行：名字 = `field.key`（同键覆盖，保留先注册槽位）。
    ///
    /// `order` 不对外（读侧 `capability_option_fields` 排完序就丢）。
    pub fn option(order: i32, field: DetailField) -> Self {
        let name = field.key.clone();
        Self {
            name,
            payload: CapabilityRowPayload::Option {
                field: Box::new(field),
                order,
            },
        }
    }

    /// 可配置声明行：名字 = `item.node.name`（目录名，同名覆盖）。
    pub fn configurable(item: VdfsItem) -> Self {
        let name = item.node.name.clone();
        Self {
            name,
            payload: CapabilityRowPayload::Configurable(Box::new(item)),
        }
    }

    /// 挂载点行：名字 = **使用方选定的目录名**（组合根下的一级目录名，宿主机内唯一）。
    pub fn mount(name: &str, provider: Arc<dyn VdfsProvider>) -> Self {
        Self {
            name: name.to_string(),
            payload: CapabilityRowPayload::Mount {
                provider,
                active: false,
            },
        }
    }

    /// 根行：**服务者单槽**（唯一带根位的那一行）。
    ///
    /// 根没有调用方给的名字（它不是某个插件的目录），行名由本构造函数填保留值；
    /// 读侧一律按 [`Self::active`] 找，不按名找。
    pub fn mount_root(provider: Arc<dyn VdfsProvider>) -> Self {
        Self {
            name: MOUNT_ROOT_ROW.to_string(),
            payload: CapabilityRowPayload::Mount {
                provider,
                active: true,
            },
        }
    }

    /// 这一行属于哪个族（与载荷同源，不存在两种答案）。
    pub fn kind(&self) -> CapabilityRowKind {
        match &self.payload {
            CapabilityRowPayload::Tool(_) => CapabilityRowKind::Tool,
            CapabilityRowPayload::Model { .. } => CapabilityRowKind::Model,
            CapabilityRowPayload::Prompt { .. } => CapabilityRowKind::Prompt,
            CapabilityRowPayload::Mount { .. } => CapabilityRowKind::Mount,
            CapabilityRowPayload::Option { .. } => CapabilityRowKind::Option,
            CapabilityRowPayload::Configurable(_) => CapabilityRowKind::Configurable,
        }
    }

    /// 行的名字（表键；工具族即线上全名）。
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 工具元数据（每次现取，见模块文档「为什么 schema 不另存一份」）。
    pub fn meta(&self) -> CapabilityMeta {
        self.cap().map(|c| c.meta()).unwrap_or_default()
    }

    /// 取可调用对象；非工具族为 `None`（调用方据此给诚实的 NotFound）。
    pub fn cap(&self) -> Option<&Arc<dyn Capability>> {
        match &self.payload {
            CapabilityRowPayload::Tool(cap) => Some(cap),
            _ => None,
        }
    }

    /// 取模型服务；非模型族为 `None`。
    pub fn provider(&self) -> Option<&Arc<dyn ModelProvider>> {
        match &self.payload {
            CapabilityRowPayload::Model { provider, .. } => Some(provider),
            _ => None,
        }
    }

    /// 取系统提示词正文；非提示词族为 `None`。
    pub fn prompt_text(&self) -> Option<&str> {
        match &self.payload {
            CapabilityRowPayload::Prompt { text } => Some(text),
            _ => None,
        }
    }

    /// 取选项字段与它的排序号；非选项族为 `None`。
    pub fn option_entry(&self) -> Option<(i32, &DetailField)> {
        match &self.payload {
            CapabilityRowPayload::Option {
                field: box_field,
                order,
            } => Some((*order, box_field.as_ref())),
            _ => None,
        }
    }

    /// 取可配置声明；非可配置族为 `None`。
    pub fn configurable_item(&self) -> Option<&VdfsItem> {
        match &self.payload {
            CapabilityRowPayload::Configurable(item) => Some(&**item),
            _ => None,
        }
    }

    /// 取 VDFS 挂载点（或根）；非挂载族为 `None`。
    pub fn vdfs(&self) -> Option<&Arc<dyn VdfsProvider>> {
        match &self.payload {
            CapabilityRowPayload::Mount { provider, .. } => Some(provider),
            _ => None,
        }
    }

    /// 这一行是否占着**生效位**（单槽族的「谁在槽里」）。
    ///
    /// 两族有位：模型族（谁在生效槽里）与挂载族（谁是 `<根>` 的服务者）。
    /// 无槽的族恒 `false`。
    pub fn active(&self) -> bool {
        match &self.payload {
            CapabilityRowPayload::Model { active, .. }
            | CapabilityRowPayload::Mount { active, .. } => *active,
            CapabilityRowPayload::Tool(_)
            | CapabilityRowPayload::Prompt { .. }
            | CapabilityRowPayload::Option { .. }
            | CapabilityRowPayload::Configurable(_) => false,
        }
    }

    /// 同一行的**退位副本**（生效位清零；无位的族原样返回）。
    ///
    /// 单槽族的写入方在收下新生效行前用它降下旧行——表的行是 `Arc`，就地改位等于
    /// 让读者看见两次变化；换一个副本入表，读出的一直是一份完整的行。
    pub fn demote(&self) -> Self {
        match &self.payload {
            CapabilityRowPayload::Model { provider, .. } => Self {
                name: self.name.clone(),
                payload: CapabilityRowPayload::Model {
                    provider: Arc::clone(provider),
                    active: false,
                },
            },
            CapabilityRowPayload::Mount { provider, .. } => Self {
                name: self.name.clone(),
                payload: CapabilityRowPayload::Mount {
                    provider: Arc::clone(provider),
                    active: false,
                },
            },
            // 无位的族（含 4e 入表的选项 / 可配置两族）：退位副本就是它自己
            _ => self.clone(),
        }
    }
}

#[cfg(test)]
#[path = "row.test.rs"]
mod tests;
