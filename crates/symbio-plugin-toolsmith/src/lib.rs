//! 制造工具插件 —— **本插件自己的工具定义 + 读数**（名册 #41，DE-026 §2.3，受限）。
//!
//! | 面 | 用在哪 | 谁读 |
//! |---|---|---|
//! | 工具（`Capability`） | `toolsmith_define` | 模型（经工具面调用） |
//! | 行为（`ToolBehavior`） | 一次成功登记 ⇒ 一条 `toolsmith.defined` 草稿 | 宿主唯一的写口（I-1） |
//! | 事实（`EventDecl` / `FactDecl` / 归宿行） | `toolsmith.defined` 落在 `domain × asserted` | 写闸按声明校验名字与格位 |
//! | 视图（`ViewDecl`） | `toolsmith.recipes` 折出当前工具定义清单 | 本包私有：没有第二读者，名字不进 `shared_views`（D-221 的公开动作只对真有读方的视图做） |
//!
//! 没有新路由、没有配置、没有新端口——`kernel` / `sdk` / `loop` / `session` 一行都不动。
//!
//! ## 本条的「受限」写在这里
//!
//! **只能造自己插件的工具**：工具入参里没有 `owner` / `target`，行为把 owner 写成常量
//! `toolsmith`；视图再按 owner 过滤一遍，别人的工具定义既不产生也读不出来。一个模型
//! 想在参数里夹带 `owner: "vdfs"` 只会被忽略，落格的仍是本插件自己的定义。
//!
//! **本片只交定义与清单，不改本轮工具面**：运行期装配不存在（D-217），名册 #42（造工具
//! 的工具，L3）因此继续留空。定义进入可调用工具面要等装配期读取——那是 #42 的事，
//! 不是本包用运行期开关去凑的形状。本包对那条路的承诺只有一句：定义带着 owner，
//! 将来按 owner 过滤即可，不需要改本包的形状。
//!
//! ## 依赖面
//!
//! `[dependencies]` 只有 `symbio-plugin-api`（加 `serde_json` / `tokio` / `async-trait`
//! 这些通用库），HS-001 判的 `kernel` / `sdk` / `deploy` 一个都不列。

mod behavior;
mod decl;
mod plugin;
mod tool;
