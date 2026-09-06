//! 通用对象创建注册表
//!
//! 完全通用的对象构造机制：
//! - 构造函数统一签名：`fn(Arc<dyn InvokeRequest>) -> Box<dyn Any + Send + Sync>`
//! - 通过 `inventory` 静态收集
//! - 注册表在第一次访问时自动惰性初始化
//! - 不针对任何具体类型做特殊化
//!
//! 公共 API（导出至 `symbio_core`）：
//! - [`create_object`]
//! - [`has_creator`]
//! - 宏 `submit_object_creator!`

use crate::symbio_core::InvokeRequest;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// 通用构造函数签名：输入 `InvokeRequest`，输出装箱的 `Any` 对象
pub(crate) type ObjectConstructor = fn(Arc<dyn InvokeRequest>) -> Box<dyn Any + Send + Sync>;

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
    fn create<T>(&self, id: &str, ctx: Arc<dyn InvokeRequest>) -> Option<Arc<T>>
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
pub fn create_object<T>(id: &str, ctx: Arc<dyn InvokeRequest>) -> Option<Arc<T>>
where
    T: ?Sized + Any + Send + Sync + 'static,
{
    ObjectCreatorRegistry::global().create::<T>(id, ctx)
}

/// 判断指定 id 是否已注册构造函数
pub fn has_creator(id: &str) -> bool {
    ObjectCreatorRegistry::global().has(id)
}

/// 通用对象创建器注册宏
///
/// - 构造函数签名：`fn(Arc<dyn InvokeRequest>) -> Arc<T>`
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
            $crate::symbio_core::creator::Submit {
                id: $id,
                ctor: (|ctx: std::sync::Arc<dyn $crate::symbio_core::InvokeRequest>| -> Box<dyn std::any::Any + Send + Sync> {
                    let result: std::sync::Arc<$target> = $constructor(ctx);
                    Box::new(result) as Box<dyn std::any::Any + Send + Sync>
                }) as $crate::symbio_core::creator::ObjectConstructor,
                type_id: std::any::TypeId::of::<$target>(),
            }
        }
    };
}
