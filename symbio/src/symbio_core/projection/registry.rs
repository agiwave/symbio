//! 投影登记表 —— 进程级、按名字寻址、纯函数的唯一入口。
//!
//! ## 唯一构造入口 [`Projection::new`]：签名即约束
//!
//! ```text
//! Projection::new(f)  where  f: Fn(&ProjectionInput<'_>) -> V
//! ```
//!
//! `f` 的形参表**就是纯净性的全部表达**：
//!
//! | 拿不到的东西 | 代表什么 |
//! |---|---|
//! | `&Store` / `&PersistentChatSession` | 不能读任意存储 |
//! | `&mut _` | 不能就地改写（投影不可有副作用） |
//! | `Clock` / `clock_now_ms()` | 结果不随系统时钟变化（可双跑） |
//! | `FactPrincipal` / 会话 id | 不依赖"谁在问"（投影是客观折叠） |
//!
//! 这些都是**类型系统层面**的不可能，不是口头纪律——它在 `cargo build` 时就报错，
//! 无需 CI 扫描（v2 的二级强制可后置）。
//!
//! ## 为什么返回 [`View`] 而非 `Result`
//!
//! 纯函数没有失败路径：输入是 `&[Fact]` 与一个时刻，输出是折叠结果。
//! 让它返回 `Result` 只会诱使实现方把 IO 塞进来（"反正能报错"）。
//! 需要表达"没折出东西"时用 `View::new(empty)`；表达"没生效"用 `View::trivial`。
//!
//! ## 登记：沿用 `inventory`（与 [`creator`](crate::creator) 同一手法）
//!
//! 投影**仍住各自插件**，只在编译期经 [`submit_projection!`](crate::submit_projection)
//! 登记进一张进程级表。表按 `name` 寻址，消费方取用时**不必认识产生方**——
//! 这与 [`FactSource`](crate::FactSource) 收口跨域知识的动机一致。
//!
//! ## 无状态：为什么是 `fn` 指针而不是闭包
//!
//! [`Projection`] 内部存的是 `fn` 指针，不是 `Box<dyn Fn>`。这是**有意的约束**：
//! 投影是"无状态纯函数"，不该捕获任何上下文（捕获取到的就可能是个 `Arc<Store>`，
//! 那正是要拦的东西）。`fn` 指针在类型上就杜绝了捕获。
//!
//! 于是 `Projection::new` 接受 `fn(&ProjectionInput<'_>) -> View<V>`，
//! 既有的三个纯函数用**薄适配器**包一层即可登记（见 `session` 插件的 `projections` 模块）。

use super::view::View;
use crate::symbio_core::Fact;
use std::sync::OnceLock;

/// 投影的输入 —— 投影能看见的**全部世界**。
///
/// 只有两样：事实序列（B1 信封）与一个时刻。没有存储、没有时钟函数、
/// 没有主体——这不是"现在只给两个字段、以后再放宽"，而是**投影的定义**：
/// 一个只能看事实与时刻的函数，其输出必然可双跑比对（A4）。
#[derive(Clone, Copy)]
pub struct ProjectionInput<'a> {
    /// 事实序列（`seq` 严格递增，见 [`Fact`]）。投影按需过滤 / 折叠。
    pub facts: &'a [Fact],
    /// 本次投影的"当前时刻"（Unix 毫秒）。**由调用方给定**而非投影自取——
    /// 于是同一份输入双跑得到同一结果（投影不读系统时钟）。
    pub at_ms: i64,
}

impl<'a> ProjectionInput<'a> {
    pub fn new(facts: &'a [Fact], at_ms: i64) -> Self {
        Self { facts, at_ms }
    }
}

/// 投影错误 —— 取用投影时的**寻址**失败。
///
/// 注意：**投影本身永不失败**（返回 `View` 不返回 `Result`）。本枚举描述的是
/// "按这个名字取不到投影"，与"投影运行出错"是两回事。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionError {
    /// 该名字没有登记的投影（"未接入"由调用方按平凡值处理）
    NotFound(String),
}

impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectionError::NotFound(name) => write!(f, "投影未登记：{name}"),
        }
    }
}

impl std::error::Error for ProjectionError {}

/// 投影的**类型擦除**签名：把 `&ProjectionInput` 折成已序列化的 `View<serde_json::Value>`。
///
/// 为什么擦除成 JSON：登记表是**异质**的（不同投影的 `V` 不同），
/// 只能统一到"可序列化的值"这层。这也顺带保证了 A4 的双跑比对有统一的比较基准
/// （比 JSON 字节，而不用为每个 `V` 写比较器）。
pub type ProjectionFn = fn(&ProjectionInput<'_>) -> View<serde_json::Value>;

/// 一个登记的投影条目。
///
/// 字段 `pub`：`submit_projection!` 宏在**别的 crate / 模块**位置展开，
/// 必须能构造它（与 `creator::Submit` 同款——那是 `pub(crate)`，
/// 但宏经 `#[macro_export]` 导出到 crate 根，故这里取 `pub`）。
pub struct ProjectionSubmit {
    /// 投影名（全局唯一，如 `session.snapshot`）。重复登记时**后者覆盖前者**
    /// （与 `creator` 同款：HashMap 语义）。
    pub name: &'static str,
    /// 类型擦除后的投影函数。
    pub run: ProjectionFn,
}

inventory::collect!(ProjectionSubmit);

/// 投影 —— 受约束的纯函数。
///
/// 内部持 [`ProjectionFn`]（`fn` 指针，非闭包）：投影**无状态、不捕获**。
/// 唯一构造入口是 [`Projection::new`]，且其签名即纯净性约束（见模块文档）。
pub struct Projection<V> {
    inner: fn(&ProjectionInput<'_>) -> View<V>,
}

impl<V> Projection<V>
where
    V: serde::Serialize + 'static,
{
    /// **唯一构造入口** —— 形参 `f` 的类型就是纯净性的全部表达（见模块文档）。
    ///
    /// ```ignore
    /// let p = Projection::new(|input: &ProjectionInput| {
    ///     View::new(input.facts.len())
    /// });
    /// ```
    ///
    /// 试一下在 `f` 里写 `clock_now_ms()` 或要求一个 `&Store` 参数：编译不过。
    /// 这不是惯例，是约束。
    pub fn new(f: fn(&ProjectionInput<'_>) -> View<V>) -> Self {
        Self { inner: f }
    }

    /// 运行投影，得到具体类型的视图。
    pub fn run(&self, input: &ProjectionInput<'_>) -> View<V> {
        (self.inner)(input)
    }
}

/// 全局投影表（惰性初始化，仅缓存名字清单供内省）。
static TABLE: OnceLock<Vec<&'static str>> = OnceLock::new();

fn names() -> &'static Vec<&'static str> {
    TABLE.get_or_init(|| {
        inventory::iter::<ProjectionSubmit>
            .into_iter()
            .map(|s| s.name)
            .collect()
    })
}

/// 列出全部已登记的投影名（顺序不保证，调用方需稳定顺序时自行排序）。
///
/// 用途：审计者 / 测试 / 调试者需要看见"这个构建里有哪些投影"，
/// 否则"投影表在跑"与"投影表没在跑"在系统外部无法区分。
pub fn projection_list() -> Vec<&'static str> {
    let mut v = names().clone();
    v.sort_unstable();
    v.dedup();
    v
}

/// 某投影名是否已登记。
pub fn projection_has(name: &str) -> bool {
    names().contains(&name)
}

/// 运行一个**已登记**的投影，得到 JSON 视图。
///
/// - 未登记 ⇒ `Err(NotFound)`。调用方若把"未接入"视为平凡值，应先
///   [`projection_has`] 判断，或直接按 `Err` 走退化路径。
/// - `trivial` 标记原样透传 —— 投影若自己返回平凡值，调用方看得见。
pub fn projection_run(
    name: &str,
    input: &ProjectionInput<'_>,
) -> Result<View<serde_json::Value>, ProjectionError> {
    for s in inventory::iter::<ProjectionSubmit> {
        if s.name == name {
            return Ok((s.run)(input));
        }
    }
    Err(ProjectionError::NotFound(name.to_string()))
}

/// 投影登记宏 —— 把一个具名纯函数登记进进程级投影表。
///
/// ## 用法
///
/// ```ignore
/// // 具名纯函数（签名即约束）
/// fn my_projection(input: &ProjectionInput<'_>) -> View<usize> {
///     View::new(input.facts.len())
/// }
/// submit_projection!("demo.count", my_projection);
/// ```
///
/// ## 它做了什么
///
/// 1. 生成一个类型擦除的 `fn(&ProjectionInput) -> View<serde_json::Value>`：
///    先调 `$f`，再把 `View<V>` 的 `value` 序列化成 JSON，`trivial` 原样保留；
/// 2. 经 `inventory::submit!` 在**编译期**把 `{ name, run }` 塞进静态表。
///
/// ## 为什么擦除是"每登记点一个 fn"
///
/// `$f` 是 `fn` 指针（无捕获），因此可以在宏里直接**生成一个具名 `fn`** 去包它——
/// 不需要 `OnceLock` 静态宿主，也没有运行期开销。这也是 [`Projection::new`]
/// 只接受 `fn` 指针的原因：无捕获 ⇒ 擦除是零成本的一层包装。
///
/// 注意：`$f` 必须能强制转成 `fn(&ProjectionInput<'_>) -> View<V>`（即无捕获函数）。
/// 带捕获的闭包在这里编译不过——这正是想要的（投影不该捕获上下文）。
#[macro_export]
macro_rules! submit_projection {
    ($name:expr, $f:path) => {
        $crate::symbio_core::inventory::submit! {
            $crate::symbio_core::ProjectionSubmit {
                name: $name,
                run: {
                    // 擦除函数：$f 是 fn 指针，无捕获，故这里是纯 static fn。
                    fn erased(
                        input: &$crate::symbio_core::ProjectionInput<'_>,
                    ) -> $crate::symbio_core::View<serde_json::Value> {
                        let v = $f(input);
                        $crate::symbio_core::View {
                            value: serde_json::to_value(&v.value)
                                .expect("投影输出必须可序列化（View<V: Serialize>）"),
                            trivial: v.trivial,
                        }
                    }
                    erased
                },
            }
        }
    };
}
