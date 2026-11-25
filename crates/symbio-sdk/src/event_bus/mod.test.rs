use super::*;
use serde_json::json;

#[test]
fn relay_marker_does_not_change_the_business_envelope() {
    let frame = PluginFrame::data(json!({
        "type": "bus_event",
        "data": {
            "kind": "vdfs",
            "session_id": null,
            "data": { "path": "session/child" }
        }
    }));
    assert!(!event_bus_is_relayed(&frame));
    let relayed = event_bus_mark_relayed(frame);
    assert!(event_bus_is_relayed(&relayed));
    let PluginFrame::Data(value) = relayed else {
        panic!("relayed frame must remain data");
    };
    assert_eq!(value["data"]["data"]["path"], "session/child");
}

#[test]
fn relay_marker_leaves_non_data_frames_unchanged() {
    let frame = PluginFrame::Error("boom".to_string(), None);
    assert!(!event_bus_is_relayed(&frame));
    assert!(matches!(
        event_bus_mark_relayed(frame),
        PluginFrame::Error(_, None)
    ));
}
