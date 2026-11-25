//! 运行时组合边界：装配方注入根工厂，运行时不认识具体插件。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use symbio_platform::kernel::plugin::{Plugin, PluginSimpleRequest};

pub type RootFactoryFuture = Pin<Box<dyn Future<Output = Arc<dyn Plugin>> + Send + 'static>>;

pub trait RootFactory: Send + Sync + 'static {
    fn create_root(&self, context: Arc<PluginSimpleRequest>) -> RootFactoryFuture;
}

pub struct RootFactoryFn<F>(pub F);

impl<F, Fut> RootFactory for RootFactoryFn<F>
where
    F: Fn(Arc<PluginSimpleRequest>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Arc<dyn Plugin>> + Send + 'static,
{
    fn create_root(&self, context: Arc<PluginSimpleRequest>) -> RootFactoryFuture {
        Box::pin((self.0)(context))
    }
}

impl RootFactory for Arc<dyn RootFactory> {
    fn create_root(&self, context: Arc<PluginSimpleRequest>) -> RootFactoryFuture {
        Box::pin(self.as_ref().create_root(context))
    }
}

#[cfg(test)]
#[path = "root_factory.test.rs"]
pub(crate) mod tests;
