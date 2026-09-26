//! 通用对象创建注册表
//!
//! 完全通用的对象构造机制：
//! - 构造函数统一签名：`fn(Arc<dyn PluginInvokeRequest>) -> Box<dyn Any + Send + Sync>`
//! - 通过 `inventory` 静态收集
//! - 注册表在第一次访问时自动惰性初始化
//! - 不针对任何具体类型做特殊化
//!
//! 公共 API（导出至 `symbio_core`）：
//! - [`create_object`]
//! - [`has_creator`]
//! - 宏 `submit_object_creator!`
//!
//! ## 机制只做「按 id 装配」，不做「处理数据」
//!
//! 构造函数签名**只有 `ctx` 一个入口、没有参数位**，这是刻意的：本机制表达的是
//! 「按 id 装出一个对象」，不是「让这个对象算一段数据」。后者由返回对象自己的方法承担，
//! 数据经方法参数或 `ctx` 键进入。
//!
//! 由此推出与 `ctx` 键的分工——**每次调用变化的值走 `ctx` 键，按 id 选实现走本机制**。
//! 完整判据（provider 化的三个条件）见 `docs/DECISIONS.md` ADR-035。
//!
//! ## ⚠️ 构造函数必须廉价（本机制的隐含契约）
//!
//! **`create_object` 不做缓存**——每次调用都执行一次构造函数，只在容器侧持有 `Arc`
//! （`composite` 就是这么做的：`mount_child` 每个挂载点建一个实例，各带自己的 `ctx`）。
//! 因此调用方可能反复调它，而每次的代价由**构造函数自己**决定。三种策略：
//!
//! | 情形 | 写法 | 范本 |
//! |---|---|---|
//! | 无状态（unit struct） | 直接 `Arc::new(Self)` | `plugins/model/protocols/*.rs` 的 `build` |
//! | **构造昂贵**（建会话 / 载模型 / 开连接） | 实现**自己** `LazyLock` 单例，构造只 `Arc::clone` | `providers/embedding/local.rs` 的 `build_local` |
//! | 每个挂载点必须独立（持目录 / 配置 / 状态） | 就让它每次新建——**这是 `dyn Plugin` 的语义** | `plugins/composite/registry.rs::mount_child` |
//!
//! **机制层不代管缓存**：`dyn Plugin` 的「每挂载点一实例」语义要求同一 id 在
//! 不同子树下返回不同对象（见 ADR-035「被否决的方案」）。

use crate::symbio_core::PluginInvokeRequest;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// 通用构造函数签名：输入 `PluginInvokeRequest`，输出装箱的 `Any` 对象
pub(crate) type ObjectConstructor = fn(Arc<dyn PluginInvokeRequest>) -> Box<dyn Any + Send + Sync>;

/// 注册表中的条目：构造函数 + 期望的 TypeId
struct Entry {
    ctor: ObjectConstructor,
    type_id: TypeId,
}

/// 通用对象创建提交器
pub(crate) struct Submit {
    pub(crate) id: &'static str,
    pub(crate) ctor: ObjectConstructor,
    pub(crate) type_id: TypeId,
}

inventory::collect!(Submit);

/// 通用对象创建器注册表（全局单例）
struct ObjectCreatorRegistry {
    entries: OnceLock<HashMap<&'static str, Entry>>,
}

static REGISTRY: OnceLock<ObjectCreatorRegistry> = OnceLock::new();

impl ObjectCreatorRegistry {
    /// 取得或惰性初始化全局注册表
    fn global() -> &'static Self {
        REGISTRY.get_or_init(|| {
            let mut entries = HashMap::new();
            for submit in inventory::iter::<Submit> {
                tracing::info!(id = %submit.id, "auto-registered object creator");
                entries.insert(
                    submit.id,
                    Entry {
                        ctor: submit.ctor,
                        type_id: submit.type_id,
                    },
                );
            }
            ObjectCreatorRegistry {
                entries: OnceLock::from(entries),
            }
        })
    }

    /// 按 trait 创建对象（运行时校验 TypeId）
    fn create<T>(&self, id: &str, ctx: Arc<dyn PluginInvokeRequest>) -> Option<Arc<T>>
    where
        T: ?Sized + Any + Send + Sync + 'static,
    {
        let entry = self.entries.get()?.get(id)?;
        if entry.type_id != TypeId::of::<T>() {
            return None;
        }
        let boxed = (entry.ctor)(ctx);
        boxed.downcast::<Arc<T>>().ok().map(|b| Arc::clone(&*b))
    }

    /// 检查指定 id 是否已注册构造函数
    fn has(&self, id: &str) -> bool {
        self.entries.get().is_some_and(|m| m.contains_key(id))
    }

    /// 指定 trait 的全部已注册 id（`type_id` 相符者；顺序不保证）
    fn ids_of<T>(&self) -> Vec<&'static str>
    where
        T: ?Sized + Any + Send + Sync + 'static,
    {
        let want = TypeId::of::<T>();
        self.entries
            .get()
            .map(|m| {
                m.iter()
                    .filter(|(_, e)| e.type_id == want)
                    .map(|(id, _)| *id)
                    .collect()
            })
            .unwrap_or_default()
    }
}

// ============ 公共 API ============
//
// 注册表会在第一次调用 `create_object` 或 `has_creator` 时自动惰性初始化。
// 无需手动调用任何 init 函数。

/// 按 trait 创建对象
///
/// - `id` 注册时使用的字符串
/// - `ctx` 构造上下文
/// - 返回 `Some(Arc<T>)` 成功，`None` 未注册或 TypeId 不匹配
///
/// ```ignore
/// let plugin: Arc<dyn Plugin> = create_object("home", ctx).unwrap();
/// ```
pub fn create_object<T>(id: &str, ctx: Arc<dyn PluginInvokeRequest>) -> Option<Arc<T>>
where
    T: ?Sized + Any + Send + Sync + 'static,
{
    ObjectCreatorRegistry::global().create::<T>(id, ctx)
}

/// 判断指定 id 是否已注册构造函数
pub fn has_creator(id: &str) -> bool {
    ObjectCreatorRegistry::global().has(id)
}

/// 指定 trait 的**全部已注册工厂 id**（顺序不保证，调用方需要稳定顺序时自行排序）。
///
/// 用途：「这个构建里能装哪些插件」——插件工厂全在编译期注册（[`submit_object_creator!`](crate::submit_object_creator)），
/// 运行时无法加载新代码，因此「安装一个插件」的可行语义只能是「为某个**已注册**的
/// 工厂建出它的插件目录」。本函数是那份候选清单的唯一来源。
///
/// ⚠️ 按 `T` 过滤是必须的：注册表里同时住着插件工厂（`dyn Plugin`）、模型协议
/// （`dyn ModelProtocol`）、嵌入服务（`dyn EmbeddingService`）……不滤型就会把
/// 「OpenAI 协议」也列成可安装的插件。
pub fn creator_ids<T>() -> Vec<&'static str>
where
    T: ?Sized + Any + Send + Sync + 'static,
{
    ObjectCreatorRegistry::global().ids_of::<T>()
}

/// 通用对象创建器注册宏
///
/// - 构造函数签名：`fn(Arc<dyn PluginInvokeRequest>) -> Arc<T>`
/// - `$target` 是 `T` 本身（具体类型或 `dyn Trait`）
/// - 运行时通过 `create_object::<T>()` 取得对象
///
/// ```ignore
/// // 返回 Arc<ConcreteType>
/// submit_object_creator!("my_id", build_my, ConcreteType);
///
/// // 返回 Arc<dyn MyTrait>
/// submit_object_creator!("my_id", build_my, dyn MyTrait);
/// ```
#[macro_export]
macro_rules! submit_object_creator {
    ($id:expr, $constructor:path, $target:ty) => {
        $crate::symbio_core::inventory::submit! {
            $crate::symbio_core::Submit {
                id: $id,
                ctor: (|ctx: std::sync::Arc<dyn $crate::symbio_core::PluginInvokeRequest>| -> Box<dyn std::any::Any + Send + Sync> {
                    let result: std::sync::Arc<$target> = $constructor(ctx);
                    Box::new(result) as Box<dyn std::any::Any + Send + Sync>
                }) as $crate::symbio_core::ObjectConstructor,
                type_id: std::any::TypeId::of::<$target>(),
            }
        }
    };
}
