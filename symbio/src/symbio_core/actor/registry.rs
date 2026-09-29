//! actor 表 —— 进程级的**登记表**与按层的登记约束。
//!
//! ## 为什么又是一张表，而不是一个配置文件
//!
//! 现行"配置"（`SessionConfig`）回答的是"**这棵装配**的默认值是什么"；
//! 本表回答的是"**这个构建**里有哪些执行者"。两者正交：
//! 配置是每棵树一份、可被用户改；表是每份构建一份、由代码登记。
//! 把执行者塞进配置，就得为"用户改坏了怎么办"设计降级；塞进表里，
//! **能登记什么在编译期就定了**——这正是想要的约束。
//!
//! ## 登记约束（断言 A5 的落点）
//!
//! [`ActorSource::register`] 的**返回值**是"这一层允许登记的行"：
//!
//! - [`ActorScope::Root`] 的登记者只能声明 `scope = Root`；
//! - [`ActorScope::SubAgent(id)`] 的登记者只能声明**同 id** 的 `SubAgent(id)`。
//!
//! 越层的行**进不了表**（不是"进了表再被审计出来"）。这是把"越权"从
//! **事后检查**变成**登记期不可能**——与 [`Projection::new`](crate::Projection::new)
//! 用签名拦副作用是同一手法：能在构建期拦住的，不要留给运行期断言。
//!
//! ## 覆盖语义
//!
//! 同 `name` 重复登记时**后者覆盖前者**（与 `creator` / `projection` 同款
//! HashMap 语义）。于是"运行期按真实 `principal` 覆盖登记行"不需要特殊机制——
//! 它就是一次普通的同 `name` 覆盖。

use super::spec::{ActorScope, ActorSpec};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// 登记期的错误 —— **登记不进去**的原因。
///
/// 注意：与 `projection` 的 `ProjectionError` 不同，本枚举描述的是**登记期**
/// 失败（越层 / 名称冲突），不是取用期失败。取用期不会失败——表为空时返回内置默认。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorError {
    /// 越层登记：登记者所在层与行声明的 `scope` 不一致（断言 A5）
    ScopeMismatch { declared: String, layer: String },
}

impl std::fmt::Display for ActorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActorError::ScopeMismatch { declared, layer } => write!(
                f,
                "越层登记：行声明 scope={declared}，但登记者位于 {layer} 层"
            ),
        }
    }
}

impl std::error::Error for ActorError {}

/// 一个登记的 Actor 行（`&'static str` 名字 + 静态模板）。
///
/// 字段 `pub`：`submit_actor!` 宏在别的 crate / 模块位置展开，必须能构造它。
pub struct ActorSpecSubmit {
    pub name: &'static str,
    pub template: fn() -> ActorSpec,
}

impl ActorSpecSubmit {
    pub fn new(name: &'static str, template: fn() -> ActorSpec) -> Self {
        Self { name, template }
    }
}

inventory::collect!(ActorSpecSubmit);

/// 全局 Actor 表：`name → ActorSpec`。
///
/// 用 `RwLock<HashMap>` 而非 `OnceLock<Vec>`（`projection` 的形态）——差别在于
/// **本表是可写的**：运行期要用真实 `principal` 覆盖登记行。写只在"开跑一轮"时
/// 发生一次（同 name 覆盖），读远多于写，`RwLock` 正合适。
static TABLE: OnceLock<RwLock<HashMap<String, ActorSpec>>> = OnceLock::new();

fn table() -> &'static RwLock<HashMap<String, ActorSpec>> {
    TABLE.get_or_init(|| {
        // 首次访问时把编译期登记的模板灌进来（`inventory` 是静态切片，
        // 每次 `iter` 都遍历一遍；灌进 HashMap 后按 name 取用是 O(1)）。
        let mut map = HashMap::new();
        for s in inventory::iter::<ActorSpecSubmit> {
            let spec = (s.template)();
            map.insert(s.name.to_string(), spec);
        }
        RwLock::new(map)
    })
}

/// 登记一行 Actor —— **唯一写入口**。
///
/// `layer` 是**登记者所在装配层**（由调用方给出，不是行自己声明）：root 树传
/// [`ActorScope::Root`]，子智能体树传 [`ActorScope::SubAgent`]。本函数据此校验
/// `spec.scope`（断言 A5）——越层返回 [`ActorError::ScopeMismatch`]，**不改动表**。
///
/// 同 `name` 覆盖。返回 `Ok(())` 表示已生效。
pub fn actor_register(layer: &ActorScope, spec: ActorSpec) -> Result<(), ActorError> {
    if &spec.scope != layer {
        return Err(ActorError::ScopeMismatch {
            declared: spec.scope.wire(),
            layer: layer.wire(),
        });
    }
    let mut t = table()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    t.insert(spec.name.clone(), spec);
    Ok(())
}

/// 取一行 Actor（模板态）。未登记 ⇒ `None`。
pub fn actor_get(name: &str) -> Option<ActorSpec> {
    let t = table()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    t.get(name).cloned()
}

/// 列出全部行（按 `name` 排序，顺序稳定）。
///
/// 用途：`actor/list` 路由 / 审计 / 测试——否则"表里有东西"与"表是空的"
/// 在系统外部无法区分。
pub fn actor_list() -> Vec<ActorSpec> {
    let t = table()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut v: Vec<ActorSpec> = t.values().cloned().collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// 清空整张表 —— **只给测试用**。
///
/// 为什么需要它：`inventory` 表是**进程级**的，多个单测共享同一份注册表，
/// 于是"A 用例登记的 decider 行"会串味到"B 用例断言表里只有 reasoner"。
/// 测试在 `setup` 里 `actor_clear()` 再按需重新登记，隔离由此而来。
///
/// 生产代码**不得**调用（这也是它不叫 `actor_reset` 的原因——清空是全域动作，
/// 一个插件清表会把别的插件的行一起抹掉）。
pub fn actor_clear() {
    let mut t = table()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    t.clear();
}

/// 测试**串行闸** —— 进程级表 + 并行测试 = 必然串味，故测试之间互斥。
///
/// ## 为什么需要这一层，而不是"每个用例自己清表"
///
/// 清表只保证"用例开始时是干净的"，不保证"用例运行期间没人动它"。
/// 两个用例并行时，A 的 `actor_clear()` 可能落在 B 的"登记 → 断言"之间，
/// 于是 B 看到空表。这类失败**随机出现**（依赖调度），比稳定失败更糟——
/// 它会诱使人重跑而不是修。
///
/// 因此测试统一经 [`crate::actor_test_guard`] 取一把全局锁，把
/// "清表 → 登记 → 断言"整段变成临界区。锁是**测试专用**（`#[cfg(test)]` 之外不
/// 导出），不进生产路径。
///
/// 用法见 `actors.rs` / `plugin.test.rs` 的 `setup()`。
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
#[path = "registry.test.rs"]
mod tests;

/// **Actor 登记方** —— 谁想往表里放行，就实现它。
///
/// ## 为什么是 trait 而不是直接调 `actor_register`
///
/// 直接调也能跑，但**登记者所在层**（`layer`）就没人对账了：一个子 Agent 树里的
/// 插件可以随手声明 `scope = Root` 而无人察觉（这正是 A5 要拦的）。
/// 经 trait 登记时，`layer` 由**装配方在构造登记者时给定**，登记者改不了——
/// 于是"我是哪一层"不是自我声明，而是被赋予的事实。
///
/// 与 [`FactSource`](crate::FactSource) 同款：plugin 实现 trait，core 通过 trait
/// 对象收口，双方不必互相认识。
pub trait ActorSource: Send + Sync {
    /// 本登记方所在层。
    fn layer(&self) -> ActorScope;

    /// 要登记的全部行（模板态）。
    fn specs(&self) -> Vec<ActorSpec>;

    /// 按本层校验并登记全部行。默认实现遍历 [`Self::specs`] 逐个
    /// [`actor_register`]，遇首个越层行即返回 `Err`（已登记的前几行**保留**——
    /// 部分成功比"静默全丢"更容易诊断，调用方据 `Err` 决定是否当作致命）。
    fn register_all(&self) -> Result<usize, ActorError> {
        let layer = self.layer();
        let mut n = 0;
        for spec in self.specs() {
            actor_register(&layer, spec)?;
            n += 1;
        }
        Ok(n)
    }
}
