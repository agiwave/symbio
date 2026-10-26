//! `turn` —— 轮次面域：一次轮次**怎么交付**、**怎么造节点**、**产出什么记录**。
//!
//! 三个文件对应轮次面上的三件事：
//!
//! | 文件 | 内容 | 谁用 |
//! |---|---|---|
//! | [`dispatch`] | 交付包（`TurnSpec` / `TurnDispatch` / `TurnReport`）+ 动作词表与载荷 | session（派发）、loop（执行并回报） |
//! | [`node_build`] | 按形状**造**节点的写侧手艺（`llm_build_*` / `dialog_node` / 建树整形） | loop（组装）、session（重建） |
//! | [`facts`] | 轮次**产物记录**（汇报 / 任务 / 委派 / 补充） | loop（写）、session（落与读） |
//!
//! ## 为什么这三样同域、且必须住在 `llm` **之上**
//!
//! `node_build` 消费模型输出（`TurnToolCallInfo`）并产出会话对象（`ChatNode`）；
//! `dispatch` 把 `Arc<dyn ModelProvider>` 交给驱动器。也就是说本域**同时看见**
//! `llm` 与 `chat`——它是两者之上的装配面。反过来 `llm` 只看见 `chat`（一轮请求的
//! 入参是消息），`chat` 谁也不看。
//!
//! 于是依赖图是一条链，没有环：
//!
//! ```text
//! chat  ◄──  llm  ◄──  turn  ◄──  projection
//!  ▲                    │
//!  └────────────────────┘（turn 也直接按 chat 的形状造节点）
//! ```
//!
//! ## 为什么不出根符号面里的「大而全」
//!
//! 三个模块各自是一个**主题命名空间**（§1.2 登记的模块形态），符号走模块路径：
//! `symbio_core::node_build::dialog_node` / `symbio_core::facts::ReportDraft`。
//! 交付包因为被 `session` 与 `loop` 两侧逐字段构造，仍需平铺（见根 `mod.rs`）。

pub mod dispatch;
pub mod facts;
pub mod node_build;
