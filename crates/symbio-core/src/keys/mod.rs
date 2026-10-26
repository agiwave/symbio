//! SymbioKey —— 类型安全的**上下文键**
//!
//! ## 键与值同住
//!
//! 一个键有两半：**类型**（`PathKey`，实现 [`SymbioKey`]）与**实例**（`PATH`，
//! `ctx.get(&PATH)` 的凭据）。两半的放置规则是**同一条**：跟着值类型走——
//! 值类型属某个域，键就住那个域。已按此迁出的三批：
//!
//! | 键 | 值类型 | 现住 |
//! |---|---|---|
//! | `EVENT_SINK` / `ABORT_SIGNAL` | `ExecEventSink` / `ExecAbortSignal` | `symbio_core::exec` |
//! | `PLUGIN_DIR` · `PLUGIN_PARENT`（原 `PARENT`） | `PluginDir` · `Option<Weak<dyn Plugin>>` | `symbio_core::plugin` |
//! | `CAPABILITY_VISITOR` / `OPTION_VISITOR` / `CONFIGURABLE_VISITOR` · `CAPABILITY_ERRORS` | `Arc<dyn Registry>` · 带 `CapabilityError` 的桶 | `symbio_core::capability` |
//!
//! 本域自此只留**值类型为通用词汇**（`String` / `Value` / `Vec<String>`）的键——
//! 那些键没有「原生域」可回，机制（[`SymbioKey`] trait）是它们唯一的家。
//! 先例：`TURN_DISPATCH`（值类型 `TurnDispatch`）本就住 `turn` 域。
//! 判据见 `symbio_core/README.md` §1.2 与 D-095。
//!
//! ## 命名：两种形态，互不重叠
//!
//! | 形态 | 写成 | 例 |
//! |---|---|---|
//! | 键**类型**（`SymbioKey` 的实现） | `…Key`（**后缀**） | `PathKey` · `ConfigKey` |
//! | 键**实例**（该类型的唯一实例） | **裸名** | `PATH` · `ID` · `CONFIG` |
//!
//! 后缀与裸名不是「风格不统一」，而是**有判别力**的一条规则：类型名带 `Key` 后缀
//! ⇒ 归属本域一眼可辨；实例名不加前缀 ⇒ `ctx.get(&PATH)` 不会读成「键的键」
//! （`ctx.get(&KEY_PATH)` 才是那个坏形态）。
//!
//! 新增时照上表选形态：先定类型（`XxxKey`），再给实例（裸名，直接描述语义）。
//! **但先问一句「值类型住哪」**——答案不是「没有域」，就把它连同键一起放过去。
//!
//! ## 不在本域的东西
//!
//! 本域**只收上下文键**。跨进程 / 跨文件的字面量按「它描述什么」归各自的域——
//! 插件 id 归装配方（`crates/crates/crates/assembly.rs`），路由地址归 `crate::routes`，
//! 遍历端点归 `plugin::traverse`，
//! 嵌入服务 id 归 `embedding::ids`，信封载荷键归 `plugin`。判据见
//! `symbio_core/README.md` §1.2。

pub use symbio_kernel::keys::SymbioKey;

macro_rules! define_string_key {
    // 带承认标记的形态（`dead-code-audit` R-002）：lint 属性**由宏转发**到生成的结构体与
    // 常量上——调用点的属性进不到 `macro_rules!` 的生成项（rustc 报 `unused attribute`，
    // 且死码照旧）。理由仍写在调用点同行，扫描器只认源码里的字面属性行。
    (#[$attr:meta] $struct_name:ident, $const_name:ident, $key_name:expr) => {
        #[$attr]
        pub struct $struct_name;
        impl SymbioKey for $struct_name {
            type Value = String;
            fn name(&self) -> &'static str {
                $key_name
            }
            fn parse(&self, s: &str) -> Option<Self::Value> {
                Some(s.to_string())
            }
            fn format(&self, v: &Self::Value) -> String {
                v.clone()
            }
        }
        #[$attr]
        pub const $const_name: $struct_name = $struct_name;
    };
    ($struct_name:ident, $const_name:ident, $key_name:expr) => {
        pub struct $struct_name;
        impl SymbioKey for $struct_name {
            type Value = String;
            fn name(&self) -> &'static str {
                $key_name
            }
            fn parse(&self, s: &str) -> Option<Self::Value> {
                Some(s.to_string())
            }
            fn format(&self, v: &Self::Value) -> String {
                v.clone()
            }
        }
        pub const $const_name: $struct_name = $struct_name;
    };
}

define_string_key!(PathKey, PATH, "path");
define_string_key!(WorkdirKey, WORKDIR, "workdir");
define_string_key!(AgentIdKey, AGENT_ID, "agent_id");
define_string_key!(SessionIdKey, SESSION_ID, "session_id");
// 入站请求的**来源主体**（DE-001 §12.2 跨体承诺的「向谁宣告」）：调用方经请求
// `metadata.source_principal` 声明「这条消息是谁发的」（完整主体名，如
// `agent:main`）；网关 `build_ctx` 把 metadata 落进 ctx，会话插件的收件箱入队口
// 取走它随条目寄存，轮末 `record_derived` 用它做 `commitment.*` 的 `to`。
// 缺省（同机直呼 / 心跳等无来源声明的路径）⇒ 本机用户——与接线前写死的常量
// 逐字一致（D-067：`to` 从调用点常量变成数据）。
define_string_key!(SourcePrincipalKey, SOURCE_PRINCIPAL, "source_principal");
// 请求链路 id：同一条链上的多次请求共用它，把跨插件的日志串回一条线。
// 线上形态 = 请求 `metadata.trace_id`，因此读它的地方就是读 metadata 的地方
// （壳层 `route_v2`、网关 `dispatch_once`）。
define_string_key!(TraceIdKey, TRACE_ID, "trace_id");
// 当前父地址：本插件在地址空间中挂载点的绝对地址。与 `WORKDIR` / `SESSION_ID`
// 同类的**上下文数据**：父插件把请求转发给子插件时（route / traverse）改写它，
// 子插件在少数需要协议级绝对地址的场合读它拼接（见 `symbio_core::vdfs::address`）。
// 缺省（顶层请求 / 无 vdfs 装配）为空。
define_string_key!(VdfsParentAddrKey, VDFS_PARENT_ADDR, "vdfs_parent_addr");
define_string_key!(ToolCallIdKey, TOOL_CALL_ID, "tool_call_id");
// 流式工具的结果消息 id：工具据此以该 id 广播消息帧（增量走 `delta`），
// 最终帧由 tool_executor 捕获为工具结果（与 result_msg_id 占位节点同 id 合并）。
define_string_key!(ResultMsgIdKey, RESULT_MSG_ID, "result_msg_id");
// 会话运行模式：auto（无人值守，失败不弹交互）| interactive（人在环，失败可交互，但 confirm/ask_user 仍不弹框）
define_string_key!(ModeKey, MODE, "mode");
// 会话选定的 Model Provider ID（与 agent_id 同级别：随 chat_send 传输 + session.metadata 持久化）
define_string_key!(ProviderIdKey, PROVIDER_ID, "provider_id");
// 会话执行风险等级阈值：low / medium / high（与 agent_id 同级别：随 chat_send 传输 + session.metadata 持久化）
define_string_key!(RiskLevelKey, RISK_LEVEL, "risk_level");
// 跨会话工具授权清单（JSON：能力名 → 允许的最高风险档）：与会话阈值同一份配置解析点
// 落进 ctx，审批闸门与授权级联都读它（D-170）。
define_string_key!(ApprovalGrantsKey, APPROVAL_GRANTS, "approval_grants");

// 常用业务属性 Key
define_string_key!(IdKey, ID, "id");
define_string_key!(NameKey, NAME, "name");
// dead-code-allow R-002: 键常量表（N-001 裸名规则）·协议契约键先于接线（004-PROTOCOLS.md 字段表）（04 §3.1 C 类：消费方=004-PROTOCOLS.md 字段表 + 三方规范键清单）
define_string_key!(
    #[allow(dead_code)]
    KindKey,
    KIND,
    "kind"
);
// dead-code-allow R-002: 键常量表（N-001 裸名规则）·协议契约键先于接线（004-PROTOCOLS.md 字段表）（04 §3.1 C 类：消费方=004-PROTOCOLS.md 字段表 + 三方规范键清单）
define_string_key!(
    #[allow(dead_code)]
    ScopeKey,
    SCOPE,
    "scope"
);
define_string_key!(ContentKey, CONTENT, "content");
// dead-code-allow R-002: 键常量表（N-001 裸名规则）·协议契约键先于接线（004-PROTOCOLS.md 字段表）（04 §3.1 C 类：消费方=004-PROTOCOLS.md 字段表 + 三方规范键清单）
define_string_key!(
    #[allow(dead_code)]
    DescriptionKey,
    DESCRIPTION,
    "description"
);

// 注：插件域的两个键不在本域——`PLUGIN_DIR`（值类型 `PluginDir`）随值搬去
// `symbio_core::plugin::dir`；`PARENT` 改名 `PLUGIN_PARENT`（值类型
// `Option<Weak<dyn Plugin>>`）随值搬去 `symbio_core::plugin`。判据同 D-095 根因一。
// `CONFIG` 与 trait 同行（契约面），已在 `symbio_kernel::keys`。

/// 必需插件清单 Key —— **构造者**告诉容器「哪些插件即使没有目录也要补出来」
///
/// 容器是通用容器（可以嵌套另一个容器），因此它**不内置**任何插件清单：
/// 清单是构造者的策略，经本键随构造一起传入。缺省（未传）= 空清单 = 纯扫描。
pub struct RequiredPluginsKey;
impl SymbioKey for RequiredPluginsKey {
    type Value = Vec<String>;
    fn name(&self) -> &'static str {
        "required_plugins"
    }
    fn parse(&self, s: &str) -> Option<Self::Value> {
        // 逗号分隔（与其它字符串键在请求头等字符串载体中的表示一致）
        Some(
            s.split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect(),
        )
    }
    fn format(&self, v: &Self::Value) -> String {
        v.join(",")
    }
}
pub const REQUIRED_PLUGINS: RequiredPluginsKey = RequiredPluginsKey;

// 注：插件**装配策略**常量（子树清单、不可停用清单）不在本域——它们不是上下文键，
// 也不在 core：它们是装配方的数据，见 `crate::assembly`。

// 注：capability 域的四条通道键不在本域——三条收集通道
// （`CAPABILITY_VISITOR` / `OPTION_VISITOR` / `CONFIGURABLE_VISITOR`）随值类型
// `Arc<dyn Registry>` 搬去 `symbio_core::capability::collect`；错误桶键
// `CAPABILITY_ERRORS`（值类型带 `CapabilityError`）搬去 `symbio_core::capability::error`。
// 判据同 D-095 根因一：键与值同住，`keys` 只留机制与通用词汇键。
