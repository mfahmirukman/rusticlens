use k8s_openapi::api::core::v1::Event;
use kube::api::{Api, ListParams};
use kube::Client;

use crate::error::Result;

#[derive(Debug, Clone)]
pub struct EventRow {
    pub type_: String,
    pub reason: String,
    pub message: String,
    pub age: String,
}

pub async fn list_events_for_resource(
    client: &Client,
    namespace: &str,
    kind: &str,
    name: &str,
) -> Result<Vec<EventRow>> {
    let api: Api<Event> = Api::namespaced(client.clone(), namespace);
    let field_selector = format!(
        "involvedObject.kind={kind},involvedObject.name={name},involvedObject.namespace={namespace}"
    );
    let lp = ListParams::default().fields(&field_selector);
    let list = api.list(&lp).await?;

    let mut rows: Vec<EventRow> = list
        .items
        .into_iter()
        .map(|event| {
            let type_ = event.type_.unwrap_or_else(|| "Normal".into());
            let reason = event.reason.unwrap_or_else(|| "-".into());
            let message = event.message.unwrap_or_default();
            let age = crate::resources::format_age(event.last_timestamp.as_ref());
            EventRow {
                type_,
                reason,
                message,
                age,
            }
        })
        .collect();

    rows.sort_by(|a, b| b.age.cmp(&a.age));
    Ok(rows)
}

pub fn format_events_text(events: &[EventRow]) -> String {
    if events.is_empty() {
        return "No events for this resource.".to_string();
    }
    events
        .iter()
        .map(|e| format!("[{}] {} — {} ({})", e.type_, e.reason, e.message, e.age))
        .collect::<Vec<_>>()
        .join("\n")
}
