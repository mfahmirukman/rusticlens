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
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| crate::error::Error::Message(err.to_string()))
}

fn kubectl_argv(
    subcommand: &str,
    namespace: &str,
    pod_name: &str,
    container: Option<&str>,
    with_shell: bool,
) -> Vec<String> {
    let mut args = vec![
        subcommand.to_string(),
        "-it".to_string(),
        "-n".to_string(),
        namespace.to_string(),
        pod_name.to_string(),
    ];
    if let Some(container) = container {
        args.push("-c".to_string());
        args.push(container.to_string());
    }
    if with_shell {
        args.push("--".to_string());
        args.push("/bin/sh".to_string());
    }
    args
}

/// Launch `kubectl …` in a new GUI terminal window.
///
/// Tries several emulator argument styles. The previous `-e kubectl` + `args[1..]` path
/// dropped the `exec`/`attach` verb and produced a broken command.
fn spawn_in_external_terminal(kubectl_args: &[String]) -> Result<std::process::Child> {
    let kubectl = "kubectl";
    let joined = std::iter::once(kubectl)
        .chain(kubectl_args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");

    // (binary, argv after binary). Use placeholders for clarity in failures.
    let mut attempts: Vec<(&str, Vec<String>)> = Vec::new();

    // gnome-terminal / kgx / ptyxis: `term -- kubectl exec …`
    for term in ["gnome-terminal", "kgx", "ptyxis", "xfce4-terminal"] {
        let mut argv = vec!["--".into(), kubectl.into()];
        argv.extend(kubectl_args.iter().cloned());
        attempts.push((term, argv));
    }

    // konsole / xterm / alacritty / ghostty: `term -e kubectl exec …`
    for term in ["konsole", "xterm", "alacritty", "ghostty", "uxterm"] {
        let mut argv = vec!["-e".into(), kubectl.into()];
        argv.extend(kubectl_args.iter().cloned());
        attempts.push((term, argv));
    }

    // kitty: `kitty kubectl exec …` (no -e)
    {
        let mut argv = vec![kubectl.into()];
        argv.extend(kubectl_args.iter().cloned());
        attempts.push(("kitty", argv));
    }

    // wezterm: `wezterm start -- kubectl exec …`
    {
        let mut argv = vec!["start".into(), "--".into(), kubectl.into()];
        argv.extend(kubectl_args.iter().cloned());
        attempts.push(("wezterm", argv));
    }

    // Debian/Ubuntu alternate
    {
        let mut argv = vec!["-e".into(), kubectl.into()];
        argv.extend(kubectl_args.iter().cloned());
        attempts.push(("x-terminal-emulator", argv));
    }

    // Last resort: shell -c inside a known emulator
    for term in ["konsole", "xterm", "alacritty", "gnome-terminal"] {
        attempts.push((
            term,
            vec![
                if term == "gnome-terminal" {
                    "--".into()
                } else {
                    "-e".into()
                },
                "sh".into(),
                "-c".into(),
                format!("{joined}; echo; echo '[Press enter to close]'; read _"),
            ],
        ));
    }

    let mut last_err = String::from("no terminal emulator found");
    for (term, argv) in attempts {
        match Command::new(term).args(&argv).spawn() {
            Ok(child) => return Ok(child),
            Err(err) => {
                last_err = format!("{term}: {err}");
            }
        }
    }

    Err(crate::error::Error::Message(format!(
        "{last_err}; tried launching: {joined}"
    )))
}

pub fn spawn_kubectl_attach_terminal(
    namespace: &str,
    pod_name: &str,
    container: Option<&str>,
) -> Result<std::process::Child> {
    let args = kubectl_argv("attach", namespace, pod_name, container, false);
    spawn_in_external_terminal(&args)
}

pub fn spawn_kubectl_exec_terminal(
    namespace: &str,
    pod_name: &str,
    container: Option<&str>,
) -> Result<std::process::Child> {
    let args = kubectl_argv("exec", namespace, pod_name, container, true);
    spawn_in_external_terminal(&args)
}

#[cfg(test)]
mod tests {
    use super::kubectl_argv;

    #[test]
    fn exec_argv_includes_exec_verb_and_shell() {
        let args = kubectl_argv("exec", "default", "nginx", Some("app"), true);
        assert_eq!(args[0], "exec");
        assert!(args.windows(2).any(|w| w == ["--", "/bin/sh"]));
        assert!(args.iter().any(|a| a == "nginx"));
        assert!(args.windows(2).any(|w| w == ["-c", "app"]));
    }
}
