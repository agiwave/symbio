//! Symbio 初始化模块

use crate::symbio_core::{create_object, Plugin, PLUGIN_HOME};
use std::sync::Arc;

pub fn initialize() {
    crate::symbio_core::init_logger();
}

pub async fn create_root_plugin() -> Arc<dyn Plugin> {
    let context = Arc::new(crate::symbio_core::SimpleRequest::new(None, None));

    create_object::<dyn Plugin>(PLUGIN_HOME, context).expect(
        "home plugin creator not found. Make sure 'home' is registered via submit_object_creator!",
    )
}
