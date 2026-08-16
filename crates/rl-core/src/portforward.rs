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
        args.push("sh".to_string());
        args.push("-c".to_string());
        args.push(crate::ops::POD_SHELL_WRAPPER.to_string());
    }
    args
}

/// Shell-quote one argument for POSIX shells.
fn shell_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

/// Map an ancestor process name (comm) to a terminal emulator binary.
fn terminal_from_process_name(comm: &str) -> Option<&'static str> {
    // The kernel truncates comm to 15 chars, so prefix-match the long ones
    // ("gnome-terminal-server" shows up as "gnome-terminal-").
    if comm.starts_with("gnome-terminal") {
        return Some("gnome-terminal");
    }
    if comm.starts_with("mate-terminal") {
        return Some("mate-terminal");
    }
    match comm {
        "tilix" => Some("tilix"),
        "kgx" => Some("kgx"),
        "ptyxis" => Some("ptyxis"),
        "xfce4-terminal" => Some("xfce4-terminal"),
        "konsole" => Some("konsole"),
        "kitty" => Some("kitty"),
        "alacritty" => Some("alacritty"),
        "wezterm" | "wezterm-gui" => Some("wezterm"),
        "ghostty" => Some("ghostty"),
        "xterm" => Some("xterm"),
        "uxterm" => Some("uxterm"),
        "foot" => Some("foot"),
        "st" => Some("st"),
        "terminator" => Some("terminator"),
        "urxvt" => Some("urxvt"),
        "lxterminal" => Some("lxterminal"),
        _ => None,
    }
}

fn parent_pid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm (field 2) may contain spaces/parens; fields after ')' are: state ppid …
    let rest = stat.get(stat.rfind(')')? + 2..)?;
    rest.split_whitespace().nth(1)?.parse().ok()
}

/// Walk the ancestor chain via /proc — sees through shells and tmux/screen.
fn terminal_from_ancestors() -> Option<&'static str> {
    let mut pid = std::process::id();
    for _ in 0..24 {
        let ppid = parent_pid(pid)?;
        if ppid <= 1 {
            return None;
        }
        if let Ok(comm) = std::fs::read_to_string(format!("/proc/{ppid}/comm")) {
            if let Some(term) = terminal_from_process_name(comm.trim()) {
                return Some(term);
            }
        }
        pid = ppid;
    }
    None
}

/// Env fingerprints terminals export into their child shells. Also covers macOS,
/// where /proc does not exist.
fn terminal_from_env() -> Option<&'static str> {
    if std::env::var_os("TILIX_ID").is_some() {
        return Some("tilix");
    }
    if std::env::var_os("KONSOLE_VERSION").is_some() {
        return Some("konsole");
    }
    if std::env::var_os("GNOME_TERMINAL_SERVICE").is_some() {
        return Some("gnome-terminal");
    }
    if std::env::var_os("KITTY_WINDOW_ID").is_some() {
        return Some("kitty");
    }
    if std::env::var_os("WEZTERM_EXECUTABLE").is_some() {
        return Some("wezterm");
    }
    if std::env::var_os("GHOSTTY_BIN_DIR").is_some() {
        return Some("ghostty");
    }
    if std::env::var_os("ALACRITTY_LOG").is_some() {
        return Some("alacritty");
    }
    match std::env::var("TERM_PROGRAM").as_deref() {
        Ok("kitty") => Some("kitty"),
        Ok("wezterm") => Some("wezterm"),
        Ok("ghostty") => Some("ghostty"),
        _ => None,
    }
}

/// Terminal emulator hosting the current session, if detectable.
fn detect_current_terminal() -> Option<&'static str> {
    terminal_from_ancestors().or_else(terminal_from_env)
}

/// Launch styles for a specific emulator, running the command through the user's
/// `$SHELL` (e.g. zsh in Tilix). `quoted_cmd` is the shell-quoted kubectl command.
fn detected_terminal_attempts<'a>(
    term: &'a str,
    shell: &str,
    quoted_cmd: &str,
) -> Vec<(&'a str, Vec<String>)> {
    // `term -- $SHELL -c '<cmd>'` — GNOME/VTE family
    let dash_dash = vec!["--".into(), shell.into(), "-c".into(), quoted_cmd.into()];
    // `term -e $SHELL -c '<cmd>'` — -e takes the remaining argv
    let dash_e = vec!["-e".into(), shell.into(), "-c".into(), quoted_cmd.into()];
    // `term -e '<shell> -c <cmd>'` — -e parses one command string, so the whole
    // shell invocation needs an extra quoting layer for the terminal's parser.
    let dash_e_single = vec![
        "-e".into(),
        format!("{shell} -c {}", shell_quote(quoted_cmd)),
    ];
    // `term $SHELL -c '<cmd>'` — bare command style
    let bare = vec![shell.into(), "-c".into(), quoted_cmd.into()];

    match term {
        "gnome-terminal" | "kgx" | "ptyxis" | "xfce4-terminal" | "mate-terminal" => {
            vec![(term, dash_dash)]
        }
        "konsole" | "xterm" | "uxterm" | "alacritty" | "ghostty" | "st" | "urxvt"
        | "lxterminal" => vec![(term, dash_e)],
        "kitty" | "foot" => vec![(term, bare)],
        "wezterm" => vec![(
            term,
            vec![
                "start".into(),
                "--".into(),
                shell.into(),
                "-c".into(),
                quoted_cmd.into(),
            ],
        )],
        // Tilix's -e takes one command string; try the VTE `--` form as backup.
        "tilix" | "terminator" => vec![(term, dash_e_single), (term, dash_dash)],
        _ => vec![(term, dash_dash), (term, dash_e)],
    }
}

/// Launch `kubectl …` in a new GUI terminal window.
///
/// Prefers the emulator hosting the current session (ancestor process walk, then
/// env fingerprints; override with `RUSTICLENS_TERMINAL=<binary>`), running the
/// command through the user's `$SHELL`. Falls back to trying several common
/// emulators and argument styles.
fn spawn_in_external_terminal(kubectl_args: &[String]) -> Result<std::process::Child> {
    let kubectl = "kubectl";
    let joined = std::iter::once(kubectl)
        .chain(kubectl_args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    let quoted_cmd = std::iter::once(kubectl)
        .chain(kubectl_args.iter().map(String::as_str))
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ");

    // (binary, argv after binary). Use placeholders for clarity in failures.
    let mut attempts: Vec<(&str, Vec<String>)> = Vec::new();

    // Same terminal the user runs rusticlens in (e.g. Tilix), via their shell.
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let override_term = std::env::var("RUSTICLENS_TERMINAL")
        .ok()
        .filter(|value| !value.is_empty());
    let detected = override_term.or_else(|| detect_current_terminal().map(str::to_string));
    if let Some(term) = detected.as_deref() {
        for (term, argv) in detected_terminal_attempts(term, &shell, &quoted_cmd) {
            if let Ok(child) = Command::new(term).args(&argv).spawn() {
                return Ok(child);
            }
        }
    }

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
                format!("{quoted_cmd}; echo; echo '[Press enter to close]'; read _"),
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

fn kubectl_logs_argv(
    context: &str,
    namespace: &str,
    pod_name: &str,
    container: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "logs".to_string(),
        "-f".to_string(),
        "--context".to_string(),
        context.to_string(),
        "-n".to_string(),
        namespace.to_string(),
        pod_name.to_string(),
    ];
    if let Some(container) = container {
        args.push("-c".to_string());
        args.push(container.to_string());
    }
    args
}

/// Launch `kubectl logs -f` for a pod in a new GUI terminal window.
///
/// Passes `--context` because rusticlens may be browsing a context that is not the
/// kubeconfig `current-context`.
pub fn spawn_kubectl_logs_terminal(
    context: &str,
    namespace: &str,
    pod_name: &str,
    container: Option<&str>,
) -> Result<std::process::Child> {
    let args = kubectl_logs_argv(context, namespace, pod_name, container);
    spawn_in_external_terminal(&args)
}

/// Launch `kubectl logs -f -l <selector>` (all matching pods) in a new GUI terminal window.
pub fn spawn_kubectl_selector_logs_terminal(
    context: &str,
    namespace: &str,
    label_selector: &str,
    max_log_requests: usize,
) -> Result<std::process::Child> {
    let args = vec![
        "logs".to_string(),
        "-f".to_string(),
        "--context".to_string(),
        context.to_string(),
        "-n".to_string(),
        namespace.to_string(),
        "-l".to_string(),
        label_selector.to_string(),
        format!("--max-log-requests={max_log_requests}"),
    ];
    spawn_in_external_terminal(&args)
}

#[cfg(test)]
mod tests {
    use super::{
        detected_terminal_attempts, kubectl_argv, kubectl_logs_argv, shell_quote,
        terminal_from_process_name,
    };

    #[test]
    fn exec_argv_includes_exec_verb_and_shell() {
        let args = kubectl_argv("exec", "default", "nginx", Some("app"), true);
        assert_eq!(args[0], "exec");
        assert!(args.windows(2).any(|w| w == ["--", "sh"]));
        assert!(args.iter().any(|a| a == "nginx"));
        assert!(args.windows(2).any(|w| w == ["-c", "app"]));
        assert!(args
            .iter()
            .any(|a| a.contains("bash") && a.contains("ash") && a.contains("sh")));
    }

    #[test]
    fn logs_argv_follows_with_context_and_container() {
        let args = kubectl_logs_argv("prod", "default", "nginx-abc", Some("app"));
        assert_eq!(args[0], "logs");
        assert!(args.contains(&"-f".to_string()));
        assert!(args.windows(2).any(|w| w == ["--context", "prod"]));
        assert!(args.windows(2).any(|w| w == ["-n", "default"]));
        assert!(args.windows(2).any(|w| w == ["-c", "app"]));
        assert!(args.iter().any(|a| a == "nginx-abc"));

        let no_container = kubectl_logs_argv("prod", "default", "nginx-abc", None);
        assert!(!no_container.contains(&"-c".to_string()));
    }

    #[test]
    fn detects_terminal_process_names() {
        assert_eq!(terminal_from_process_name("tilix"), Some("tilix"));
        // 15-char kernel truncation of "gnome-terminal-server".
        assert_eq!(
            terminal_from_process_name("gnome-terminal-"),
            Some("gnome-terminal")
        );
        assert_eq!(terminal_from_process_name("wezterm-gui"), Some("wezterm"));
        assert_eq!(terminal_from_process_name("zsh"), None);
    }

    #[test]
    fn shell_quotes_arguments() {
        assert_eq!(shell_quote("nginx-abc"), "'nginx-abc'");
        assert_eq!(shell_quote("a'b c"), "'a'\\''b c'");
    }

    #[test]
    fn detected_attempts_run_through_user_shell() {
        let attempts = detected_terminal_attempts("tilix", "/usr/bin/zsh", "'kubectl' 'logs' '-f'");
        assert!(!attempts.is_empty());
        assert!(attempts.iter().all(|(term, _)| *term == "tilix"));
        assert!(attempts
            .iter()
            .any(|(_, argv)| argv.iter().any(|a| a == "/usr/bin/zsh")));
        assert!(attempts
            .iter()
            .any(|(_, argv)| argv.iter().any(|a| a.contains("'kubectl' 'logs' '-f'"))));
    }
}
