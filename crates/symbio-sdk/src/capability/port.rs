//! 世界端口的**读/校验半**：族与行名、端口名校验、按行名前缀分组的读数与装配期判重。
//!
//! ## 为什么这一半住 sdk
//!
//! 注册面（`PortProvider` / `PortPayload` / `port_rows`）是**插件写的**形状——能力包
//! 申报端口时用它，住 `symbio-plugin-api`。读/校验半是**宿主与编排方问表**的机制：
//! 按行名前缀分组、判重、从已注册行派生端口集、把非法端口名挡在注册期、装配期把判重
//! 落进能力错误桶。它不认识任何世界词，也不被插件实现，与本域其余读口同一条边界
//! （D-404 / D-414）。
//!
//! 族与行名形状是同一条契约的两半：`PORT_FAMILY` 的声明、`port_row_name` 的拼法、
//! `capability_port_of` 的分组必须同源，否则读数与注册面会各说各话。

use std::fmt;
use std::sync::Arc;

use crate::capability::{capability_report_error, CAPABILITY_VISITOR};
use symbio_kernel::capability::{CapabilityFamily, CapabilityRow, Registry};
use symbio_kernel::plugin::{PluginInvokeRequest, PluginInvokeRequestExt};

/// 全部端口共用的唯一 `multi` 族（D-299）：族不区分端口，端口级分组由行名前缀
/// （[`port_row_name`]）在读数里做——加端口不加族。
pub const PORT_FAMILY: CapabilityFamily = CapabilityFamily::multi("port");

/// 任何端口都落在唯一族里（端口集开放，`DE-047` I-6.7）：族不区分端口，端口级
/// 分组由行名前缀（[`port_row_name`]）在读数里做——加端口不加族。
pub fn port_family(_port: &str) -> Option<CapabilityFamily> {
    Some(PORT_FAMILY)
}

/// port 行的行名：保留前缀，撞不上任何线上工具名。
///
/// 表键是全表唯一的名字空间（族不参与键），端口名当行名会与同名工具互相顶掉；
/// 工具名不含尖括号，前缀使碰撞不可表达——与 `MOUNT_ROOT_ROW` 同一条手法。
pub fn port_row_name(port: &str, provider_id: &str) -> String {
    format!("<port:{port}>:{provider_id}")
}

/// 端口名的保留字符：环事件 id 的分隔符。
///
/// 事件 id 形如 `ring:{port}:{id}:{attempt}:{kind}`，端口位是首个字段；端口名一旦
/// 含 `:`，解析会把端口位截断（`model:assist` 会被读成 `model`），事实键与债键对不上。
/// 这里 **fail-closed**：注册期直接拒绝，不转义、不猜（D-336「已知编码风险」）。
const PORT_NAME_RESERVED: &[char] = &[':'];

/// 端口名校验：种进事件 id 之前就把保留字符挡在注册期。
pub fn check_port_name(port: &str) {
    if let Some(ch) = port.chars().find(|ch| PORT_NAME_RESERVED.contains(ch)) {
        // panic-allow crates/symbio-sdk/src/capability/port.rs::check_port_name: 注册期 fail-closed——含保留字符的端口名会让环事件 id 的端口位被截断，注册当场拒绝；带病启动比崩掉更难诊断（D-336）。
        panic!(
            "端口名 {port:?} 含有环事件 id 的保留字符 {ch:?}，端口注册失败。\
             事件 id 是 `ring:{{port}}:{{id}}:{{attempt}}:{{kind}}`，端口位按 `:` 定界；\
             命名端口时请避开保留集 {PORT_NAME_RESERVED:?}（D-336）。"
        );
    }
}

/// 一个端口的装配期读数。
pub enum PortReading {
    /// 0 行：端口缺席（没装提供者插件——合法装配态，响亮失败在调用点）。
    Absent,
    /// 1 行：就绪。
    Ready(Arc<CapabilityRow>),
    /// `>1` 行：判重——同端口的多个提供者**两行俱在**（单族不降位），`dropped` 是
    /// 其余提供者的行名。
    ///
    /// 装载期已由门面侧的 `capability_port_check` 落错误（不静默顶掉）；调用方拿到本读数
    /// 同样必须响亮处理，不许当作 [`PortReading::Ready`] 用。
    Duplicate {
        kept: Arc<CapabilityRow>,
        dropped: Vec<String>,
    },
}

impl fmt::Debug for PortReading {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
/// 判定的全部输入是**按行名前缀分组**后的行数（单一 `port` 族里其他端口的行不可
/// 见）：`0` 缺席、`1` 就绪、`>1` 判重。行不降位（`active = false`），行数让
/// 「多提供者」可判——读数不猜、不回落。
pub async fn capability_port_of(visitor: &dyn Registry, port: &str) -> PortReading {
    let prefix = port_row_name(port, "");
    let rows: Vec<_> = visitor
        .list(PORT_FAMILY)
        .await
        .into_iter()
        .filter(|r| r.name().starts_with(&prefix))
        .collect();
    match rows.len() {
        0 => PortReading::Absent,
        // panic-allow crates/symbio-sdk/src/capability/port.rs::capability_port_of: 行数已判定为 1，next() 取不到说明 list 与 len 之间表被改写（同一次调用内不会发生），属代码缺陷。
        1 => PortReading::Ready(rows.into_iter().next().expect("len == 1")),
        _ => {
            let mut it = rows.into_iter();
            // panic-allow crates/symbio-sdk/src/capability/port.rs::capability_port_of: 行数已判定为 >1，next() 取不到说明 list 与 len 之间表被改写（同一次调用内不会发生），属代码缺陷。
            let kept = it.next().expect("len > 1");
            let dropped = it.map(|r| r.name().to_string()).collect();
            PortReading::Duplicate { kept, dropped }
        }
    }
}

/// 从 `port` 族里已注册的行名派生端口集（`DE-047` I-6.7）。
///
/// 行名形状唯一来源是 [`port_row_name`]：`<port:{port}>:{provider_id}`。解析不出的行
/// （不是本族造的）由 `filter_map` 跳过——它们本来就不可见（`Registry::list` 按族取）。
pub async fn registered_ports(visitor: &dyn Registry) -> Vec<String> {
    let mut out = Vec::new();
    for row in visitor.list(PORT_FAMILY).await {
        let Some(rest) = row.name().strip_prefix("<port:") else {
            continue;
        };
        let Some((port, _)) = rest.split_once(">:") else {
            continue;
        };
        let port = port.to_string();
        if !out.contains(&port) {
            out.push(port);
        }
    }
    out
}

/// 装配期判重检查（D-298 / D-299 / `DE-047` I-6.7）：端口集从**已注册行**派生
/// （`<port:{port}>:` 前缀），每个端口按行名前缀读一次分组行数；判重 ⇒ 落一条错误
/// 进 `CAPABILITY_ERRORS`。
///
/// **时点契约**：必须在能力收集结束、编排方读错误桶（`capability_take_errors`）
/// 裁决**之前**调用——判重错误与收集期错误同桶同路，裁决即中止会话（装载期硬红）。
/// 落桶之后才裁决 ⇒ 错误留在桶里无人读，判重退化为「照常跑」，那是本检查要消灭的
/// 形态。缺席**不报错**——「没装提供者插件」是合法装配态（不装 model 插件的部署
/// 存在），响亮失败发生在调用点。没有 `CAPABILITY_VISITOR` 的 ctx 上本函数静默跳过：
/// 收集尚未开始时没有表可读。
pub async fn capability_port_check(ctx: &Arc<dyn PluginInvokeRequest>) {
    let Some(visitor) = ctx.get(CAPABILITY_VISITOR) else {
        return;
    };
    for port in registered_ports(&*visitor).await {
        if let PortReading::Duplicate { kept, dropped } = capability_port_of(&*visitor, &port).await
        {
            capability_report_error(
                ctx,
                "core",
                format!(
                    "世界端口 `{port}` 注册了多个提供者：`{}` 与 {dropped:?} 两行俱在",
                    kept.name(),
                ),
            )
            .await;
        }
    }
}
