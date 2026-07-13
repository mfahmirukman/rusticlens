use std::process::{Command, Stdio};

use crate::error::Result;
use crate::resources::ResourceKind;

pub fn kubectl_port_forward_command(
    namespace: &str,
    kind: ResourceKind,
    name: &str,
    local_port: u16,
    remote_port: u16,
) -> String {
    let target = match kind {
        ResourceKind::Service => format!("service/{name}"),
        _ => format!("pod/{name}"),
    };
    format!("kubectl port-forward -n {namespace} {target} {local_port}:{remote_port}")
}

pub fn spawn_kubectl_port_forward(
    namespace: &str,
    kind: ResourceKind,
    name: &str,
    local_port: u16,
    remote_port: u16,
) -> Result<std::process::Child> {
    let target = match kind {
        ResourceKind::Service => format!("service/{name}"),
        _ => format!("pod/{name}"),
    };
    Command::new("kubectl")
        .args([
            "port-forward",
            "-n",
            namespace,
            &target,
            &format!("{local_port}:{remote_port}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| crate::error::Error::Message(err.to_string()))
}

pub fn spawn_kubectl_attach_terminal(
    namespace: &str,
    pod_name: &str,
    container: Option<&str>,
) -> Result<std::process::Child> {
    let mut args = vec![
        "attach".to_string(),
        "-it".to_string(),
        "-n".to_string(),
        namespace.to_string(),
        pod_name.to_string(),
    ];
    if let Some(container) = container {
        args.push("-c".to_string());
        args.push(container.to_string());
    }

    let terminals = [
        "x-terminal-emulator",
        "gnome-terminal",
        "konsole",
        "xfce4-terminal",
        "alacritty",
        "kitty",
        "xterm",
    ];

    for term in terminals {
        let child = Command::new(term)
            .arg("-e")
            .arg("kubectl")
            .args(&args)
            .spawn();
        if let Ok(child) = child {
            return Ok(child);
        }
        let child = Command::new(term)
            .args(["--", "kubectl"])
            .args(&args)
            .spawn();
        if let Ok(child) = child {
            return Ok(child);
        }
    }

    Err(crate::error::Error::Message(
        "no terminal emulator found; use Copy attach command instead".into(),
    ))
}

pub fn spawn_kubectl_exec_terminal(
    namespace: &str,
    pod_name: &str,
    container: Option<&str>,
) -> Result<std::process::Child> {
    let mut args = vec![
        "exec".to_string(),
        "-it".to_string(),
        "-n".to_string(),
        namespace.to_string(),
        pod_name.to_string(),
    ];
    if let Some(container) = container {
        args.push("-c".to_string());
        args.push(container.to_string());
    }
    args.push("--".to_string());
    args.push("/bin/sh".to_string());

    let terminals = [
        "x-terminal-emulator",
        "gnome-terminal",
        "konsole",
        "xfce4-terminal",
        "alacritty",
        "kitty",
        "xterm",
    ];

    for term in terminals {
        let child = Command::new(term)
            .arg("-e")
            .arg("kubectl")
            .args(&args[1..])
            .spawn();
        if let Ok(child) = child {
            return Ok(child);
        }
        let child = Command::new(term)
            .args(["--", "kubectl"])
            .args(&args)
            .spawn();
        if let Ok(child) = child {
            return Ok(child);
        }
    }

    Err(crate::error::Error::Message(
        "no terminal emulator found; use Copy exec instead".into(),
    ))
}
