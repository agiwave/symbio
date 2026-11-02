//! 世界端口的注册面 —— 端口名字与契约静态，提供者随活插件的能力行动态收集（D-298）。
//!
//! ## 静态的只有名字与契约
//!
//! 端口名是闭集（[`ports::MODEL`] / [`ports::TOOL`]）；`PortDecl`、派发 / 等待两行模板、
//! `Call{port,id,attempt,via}` 的幂等键与记账住 `loop`。**提供者不静态注册**：
//! `submit_*!` 一类的编译期收集器在这里没有位置——它们链接期生效、没有启用过滤，
//! 插件停用后条目仍在（实例不构造不等于链接期条目消失），「停用即缺席」当场判红。
//! 这正是 `submit_port_provider!` 案被否的原因（D-298「为什么不是 (b)」）。
//!
//! ## 提供者随能力表走
//!
//! 提供者行由**活插件**在自己的 `traverse` 里 `put`（与工具 / 模型目录 / 挂载同一广播
//! 机制）：停用的插件不参与装配、不 traverse、不写行；收集器每次装配重建，旧行不残留。
//! 行族**每端口一个单槽**（`port.model` / `port.tool`）——`single` 是族级策略（`put`
//! 按族降位），两个端口挤同一个族会互相顶掉。
//!
//! ## 判重与缺席
//!
//! - `0` 行 = 缺席（合法装配态：没装提供者插件）；响亮失败发生在调用点（消费方拿到
//!   [`PortReading::Absent`] 自行落事实并中止）。
//! - `>1` 行 = 判重（表侧 `demote` 保证一活一降）：[`capability_port_check`] 落一条错误
//!   进 `CAPABILITY_ERRORS`（装载期判红），保留先注册行（与挂载 / 模型的先注册槽位
//!   语义一致），**不静默顶掉**。不做显式优先级——单 provider 场景没有挑选判据，
//!   「挑一个」正是要消灭的静默形态。
//! - 闭集外端口名 = [`PortReading::Unknown`]（漏声明，不是默认值）。
//!
//! ## 不出现在消费面
//!
//! `Registry` 的读方全部按族取（`list` 的签名要求族参数，仓内无全表遍历读点）；
//! 按名取的 `get` 由保留行名前缀挡住（工具名不含尖括号，同 `MOUNT_ROOT_ROW` 手法）。
//! 结构性用例钉住这条事实：port 行在表 ⇒ 六个消费族（工具 / 模型 / 提示词 / 挂载 /
//! 选项 / 可配置）仍然看不到它。

use std::fmt;
use std::sync::Arc;
use symbio_kernel::capability::{CapabilityFamily, CapabilityRow, Registry};
use symbio_kernel::exec::ExecAbortSignal;
use symbio_kernel::llm::ModelProvider;
use symbio_kernel::plugin::{PluginInvokeRequest, PluginInvokeRequestExt};
use symbio_sdk::adapters::LlmAdapter;
use symbio_sdk::capability::{capability_report_error, CAPABILITY_VISITOR};

/// 端口名闭集（宿主词汇，与 [`crate::behaviors`] / [`crate::context::slots`] 同层）。
pub mod ports {
    /// 模型往返端口（一次 LLM 调用）。名字从内核模型族派生：端口词与族词是同一个
    /// 词，写第二份拼写早晚会漂（字面量还会与插件 id 撞审计红线）。
    pub const MODEL: &str = symbio_kernel::capability::family::MODEL.name();
    /// 工具调用端口（一批工具调用里的一次）。同 [`MODEL`]，从内核工具族派生。
    pub const TOOL: &str = symbio_kernel::capability::family::TOOL.name();

    /// 宿主**当前**认识的端口名。
    ///
    /// 闭集：加端口是设计裁决（`PortDecl` + 两行模板 + 端口族三样一起动），不是
    /// 声明方自带新名字的事——这与行为名 / 段身份的开放集合不同，因为端口连着
    /// 环的记账与租约，环必须先认识它。
    pub const KNOWN: &[&str] = &[MODEL, TOOL];
}

/// 一个世界端口的提供者：声明「我服务哪个端口、我叫什么」+ 端口的消费合同。
///
/// 声明位（`port` / `provider_id`）供表造行、消费方判缺席与判重；消费合同
/// （[`llm_round_trip`](PortProvider::llm_round_trip) / [`model`](PortProvider::model)）是
/// model 端口的两条取用形态——`Arc<dyn PortProvider>` 在 Rust 里跨转型不出去，执行面
/// 必须以方法形式落在这里，提供者一侧才能把解析与适配器构造都收走（D-298 M1）。
/// 合同带默认体：非该端口的提供者不实现，默认 `None` 表示「本端口没有这条合同」，
/// 不是缺席的默认值。
pub trait PortProvider: Send + Sync + 'static {
    /// 端口名（必须是 [`ports::KNOWN`] 闭集之一）。
    fn port(&self) -> &'static str;
    /// 提供者 id（行名后缀；判重读数里指认谁被顶掉）。
    fn provider_id(&self) -> &str;

    /// model 端口的消费合同：按首选 id 解析并构造**本次往返**的适配器。
    ///
    /// 解析（严格查 `preferred` → 落回当次生效者，降级方向只有提供者这一处定义）与
    /// 适配器构造都在提供者一侧——环只按端口名取，不知道 provider 怎么来。
    /// `None` = 无生效者：调用方必须响亮失败，不许静默回退。
    fn llm_round_trip(
        &self,
        _preferred: Option<&str>,
        _system_prompt: &str,
        _abort: ExecAbortSignal,
    ) -> Option<Arc<dyn LlmAdapter>> {
        None
    }

    /// model 端口的原始模型面：内部请求（压缩摘要一类，IM-001 登记在调用方）直调
    /// `round_trip` 用的提供者。
    fn model(&self) -> Option<Arc<dyn ModelProvider>> {
        None
    }
}

/// port 行的载荷。
///
/// 消费方经 `row.payload::<PortPayload>()` 下转型取回——类型是声明方与消费方之间的
/// 私有约定，表不参与校验（D-175 的开放载荷形状）。
pub struct PortPayload(Arc<dyn PortProvider>);

impl PortPayload {
    pub fn new(provider: Arc<dyn PortProvider>) -> Self {
        Self(provider)
    }

    /// 取提供者。
    pub fn provider(&self) -> &Arc<dyn PortProvider> {
        &self.0
    }
}

/// 每端口一个单槽族。闭集外 ⇒ `None`（漏声明，不是默认值）。
pub fn port_family(port: &str) -> Option<CapabilityFamily> {
    match port {
        ports::MODEL => Some(CapabilityFamily::single("port.model")),
        ports::TOOL => Some(CapabilityFamily::single("port.tool")),
        _ => None,
    }
}

/// port 行的行名：保留前缀，撞不上任何线上工具名。
///
/// 表键是全表唯一的名字空间（族不参与键），端口名当行名会与同名工具互相顶掉；
/// 工具名不含尖括号，前缀使碰撞不可表达——与 `MOUNT_ROOT_ROW` 同一条手法。
pub fn port_row_name(port: &str, provider_id: &str) -> String {
    format!("<port:{port}>:{provider_id}")
}

/// 提供者行：单槽族的占位行（`active = true`）。
///
/// 闭集外端口 ⇒ `None`——调用方（提供者插件的 traverse 分支）应当跳过注册而不是
/// 猜一个族。
pub fn port_row(provider: &Arc<dyn PortProvider>) -> Option<CapabilityRow> {
    let family = port_family(provider.port())?;
    Some(CapabilityRow::new(
        family,
        port_row_name(provider.port(), provider.provider_id()),
        PortPayload::new(provider.clone()),
        true,
    ))
}

/// 一个端口的装配期读数。
pub enum PortReading {
    /// 端口名不在闭集（漏声明，不是默认值）。
    Unknown(String),
    /// 0 行：端口缺席（没装提供者插件——合法装配态，响亮失败在调用点）。
    Absent,
    /// 1 行：就绪。
    Ready(Arc<CapabilityRow>),
    /// `>1` 行：判重——保留先注册行，被顶掉的名字在 `dropped` 里。
    ///
    /// 装载期已由 [`capability_port_check`] 落错误（不静默顶掉）；调用方拿到本读数
    /// 同样必须响亮处理，不许当作 [`PortReading::Ready`] 用。
    Duplicate {
        kept: Arc<CapabilityRow>,
        dropped: Vec<String>,
    },
}

impl fmt::Debug for PortReading {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(port) => write!(f, "Unknown({port})"),
            Self::Absent => write!(f, "Absent"),
            Self::Ready(row) => write!(f, "Ready({})", row.name()),
            Self::Duplicate { kept, dropped } => {
                write!(
                    f,
                    "Duplicate {{ kept: {}, dropped: {dropped:?} }}",
                    kept.name()
                )
            }
        }
    }
}

/// 读一个端口的提供者行（纯读数，不落错误）。
///
/// 判定的全部输入是族内行数：`0` 缺席、`1` 就绪、`>1` 判重。表侧 `demote` 保证
/// 「两个生效者」不可表达，行数让「静默被顶掉」可判——读数不猜、不回落。
pub async fn capability_port_of(visitor: &dyn Registry, port: &str) -> PortReading {
    let Some(family) = port_family(port) else {
        return PortReading::Unknown(port.to_string());
    };
    let rows = visitor.list(family).await;
    match rows.len() {
        0 => PortReading::Absent,
        // panic-allow crates/symbio-plugin-api/src/port.rs::capability_port_of: 行数已判定为 1，next() 取不到说明 list 与 len 之间表被改写（同一次调用内不会发生），属代码缺陷。
        1 => PortReading::Ready(rows.into_iter().next().expect("len == 1")),
        _ => {
            let mut it = rows.into_iter();
            // panic-allow crates/symbio-plugin-api/src/port.rs::capability_port_of: 行数已判定为 >1，next() 取不到说明 list 与 len 之间表被改写（同一次调用内不会发生），属代码缺陷。
            let kept = it.next().expect("len > 1");
            let dropped = it.map(|r| r.name().to_string()).collect();
            PortReading::Duplicate { kept, dropped }
        }
    }
}

/// 装配期判重检查（D-298）：闭集每个端口读一次族内行数；判重 ⇒ 落一条错误进
/// `CAPABILITY_ERRORS`（装载期判红，session 读桶即红）。
///
/// 缺席**不报错**——「没装提供者插件」是合法装配态（不装 model 插件的部署存在），
/// 响亮失败发生在调用点。没有 `CAPABILITY_VISITOR` 的 ctx 上本函数静默跳过：
/// 收集尚未开始时没有表可读，检查由编排方在收集结束后调用。
pub async fn capability_port_check(ctx: &Arc<dyn PluginInvokeRequest>) {
    let Some(visitor) = ctx.get(CAPABILITY_VISITOR) else {
        return;
    };
    for port in ports::KNOWN {
        if let PortReading::Duplicate { kept, dropped } = capability_port_of(&*visitor, port).await
        {
            capability_report_error(
                ctx,
                "core",
                format!(
                    "世界端口 `{port}` 注册了多个提供者：保留先注册的 `{}`，被顶掉的 {dropped:?}",
                    kept.name(),
                ),
            )
            .await;
        }
    }
}

#[cfg(test)]
#[path = "port.test.rs"]
mod tests;
