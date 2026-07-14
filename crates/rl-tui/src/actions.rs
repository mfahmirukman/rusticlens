use rl_core::ResourceKind;

use crate::app::PendingAction;

#[derive(Debug, Clone)]
pub struct ActionItem {
    pub label: String,
    pub action: PendingAction,
}

pub fn actions_for_kind(kind: ResourceKind, has_selection: bool) -> Vec<ActionItem> {
    let mut items = Vec::new();
    if !has_selection {
        return items;
    }

    match kind {
        ResourceKind::Pod => {
            items.push(item("logs", "Open logs", PendingAction::OpenLogs));
            items.push(item(
                "exec",
                "Exec shell (new window)",
                PendingAction::ExecShell,
            ));
            items.push(item("pf", "Port-forward", PendingAction::StartPortForward));
            items.push(item(
                "fav",
                "Toggle favorite",
                PendingAction::ToggleFavorite,
            ));
            items.push(item("apply", "Apply YAML…", PendingAction::ApplyYaml));
            items.push(item("edit", "Edit YAML…", PendingAction::EditYaml));
            items.push(item("delete", "Delete", PendingAction::Delete));
        }
        ResourceKind::Deployment | ResourceKind::StatefulSet => {
            items.push(item("scale", "Scale", PendingAction::Scale));
            items.push(item("restart", "Rollout restart", PendingAction::Restart));
            items.push(item("pf", "Port-forward", PendingAction::StartPortForward));
            items.push(item(
                "fav",
                "Toggle favorite",
                PendingAction::ToggleFavorite,
            ));
            items.push(item("apply", "Apply YAML…", PendingAction::ApplyYaml));
            items.push(item("edit", "Edit YAML…", PendingAction::EditYaml));
            items.push(item("delete", "Delete", PendingAction::Delete));
        }
        ResourceKind::CronJob => {
            items.push(item(
                "trigger",
                "Trigger Job",
                PendingAction::TriggerCronJob,
            ));
            items.push(item("suspend", "Suspend", PendingAction::SuspendCronJob));
            items.push(item("resume", "Resume", PendingAction::ResumeCronJob));
            items.push(item(
                "fav",
                "Toggle favorite",
                PendingAction::ToggleFavorite,
            ));
            items.push(item("apply", "Apply YAML…", PendingAction::ApplyYaml));
            items.push(item("edit", "Edit YAML…", PendingAction::EditYaml));
            items.push(item("delete", "Delete", PendingAction::Delete));
        }
        ResourceKind::Service => {
            items.push(item(
                "logs",
                "Open logs (all pods)",
                PendingAction::OpenServiceLogs,
            ));
            items.push(item("pf", "Port-forward", PendingAction::StartPortForward));
            items.push(item(
                "fav",
                "Toggle favorite",
                PendingAction::ToggleFavorite,
            ));
            items.push(item("apply", "Apply YAML…", PendingAction::ApplyYaml));
            items.push(item("edit", "Edit YAML…", PendingAction::EditYaml));
            items.push(item("delete", "Delete", PendingAction::Delete));
        }
        _ => {
            items.push(item(
                "fav",
                "Toggle favorite",
                PendingAction::ToggleFavorite,
            ));
            items.push(item("apply", "Apply YAML…", PendingAction::ApplyYaml));
            items.push(item("edit", "Edit YAML…", PendingAction::EditYaml));
            items.push(item("delete", "Delete", PendingAction::Delete));
        }
    }
    items
}

fn item(_id: &'static str, label: &str, action: PendingAction) -> ActionItem {
    ActionItem {
        label: label.to_string(),
        action,
    }
}
