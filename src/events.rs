//! In-process event bus used to push live updates to the browser UI.

use serde::Serialize;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_id: Option<String>,
    pub data: serde_json::Value,
}

impl Event {
    pub fn new(kind: &str, server_id: Option<String>, data: serde_json::Value) -> Self {
        Self {
            kind: kind.to_string(),
            server_id,
            data,
        }
    }
}

pub type EventBus = broadcast::Sender<Event>;

pub fn event_bus() -> EventBus {
    let (tx, _rx) = broadcast::channel(1024);
    tx
}

/// Silently ignore receiver-less sends: events are best-effort UI updates and
/// must never block or fail the operation that produced them.
pub fn emit(bus: &EventBus, event: Event) {
    let _ = bus.send(event);
}
