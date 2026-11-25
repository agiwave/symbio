use std::sync::Arc;

use serde_json::Value;
use symbio::symbio_kernel::plugin::PluginFrame;

const RELAY_MARKER: &str = "__relayed";

pub fn mark_relayed(frame: PluginFrame) -> PluginFrame {
    let PluginFrame::Data(value) = frame else {
        return frame;
    };
    let mut outer = value.as_object().cloned().unwrap_or_default();
    outer.insert(RELAY_MARKER.to_string(), Value::Bool(true));
    PluginFrame::Data(Arc::new(Value::Object(outer)))
}

pub fn is_relayed(frame: &PluginFrame) -> bool {
    frame
        .value()
        .and_then(|value| value.get(RELAY_MARKER))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "event_relay.test.rs"]
mod tests;
