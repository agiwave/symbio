//! Symbio 平台——插件编译图的最底层，**零工作区内依赖**。
//!
//! 一个 crate、四个子模块，各管一类东西：
//!
//! - [`kernel`]：**封闭语法**——事件信封与 Entity×Verb、视图与预算、声明容器、
//!   几个端口 trait 的形状。只含语法类型与冻结机制原语，字符串里没有能力名 / owner 名 /
//!   业务概念名。
//! - [`sdk`]：**公共机制**——配置 IO、事件总线、VDFS 地址装配、对象构造器登记、日志、
//!   协议键、能力调用栈、纯折叠。有实现，但不认容器概念，只碰内核词汇与事件信封上的字段。
//! - [`contract`]：**插件 ABI 的纯形状**——对话面控制契约与模型输入词汇，零状态零 IO。
//! - [`deploy`]：**部署事实**（生成物）——`PLUGIN_ID_*` / `ASSEMBLY_*` 由各插件 `PLUGIN.yml`
//!   投影而来。
//!
//! 四者同住一个 crate 是为了少三道包边界、少三条依赖边；**语义边界没有消失**，改由目录
//! 承担：`arch-shape-audit` 只扫 `kernel/`，`domain-vocab-audit` 只扫 `sdk/`。
//!
//! 谁可以直接依赖平台：`symbio-plugin-api`（门面，扩展层插件经它取用）、控制面
//! （`session` / `loop`）、`symbio-providers`、宿主与入口。**扩展层插件不直接依赖本 crate**
//! ——判据是 `plugin-surface-audit` 的 HS-001。

pub mod contract;
pub mod deploy;
pub mod kernel;
pub mod sdk;
