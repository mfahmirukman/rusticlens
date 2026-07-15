//! Clipboard helpers that avoid hanging on a broken/unreachable X11 display.
//! Prefer: `wl-copy` / `xclip` / `xsel` / `pbcopy` → OSC 52 (via `/dev/tty`) → arboard.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

pub fn copy_text(text: &str) -> Result<(), String> {
    // Prefer real OS clipboard tools first. OSC 52 writes "succeed" even when the
    // terminal ignores them, which made yank look successful without pastable data.
    if copy_via_cli(text) {
        let _ = copy_via_osc52(text);
        return Ok(());
    }

    let has_gui =
        std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some();

    if has_gui {
        if let Ok(()) =
            arboard::Clipboard::new().and_then(|mut clip| clip.set_text(text.to_string()))
        {
            let _ = copy_via_osc52(text);
            return Ok(());
        }
        // Still emit OSC 52 for terminals that allow it, but require a real clipboard
        // backend on local desktops — Wayland often blocks OSC 52.
        let _ = copy_via_osc52(text);
        return Err(
            "clipboard unavailable — install wl-clipboard (`wl-copy`) or xclip, then retry".into(),
        );
    }

    // SSH / no display: OSC 52 may be the only path. Write to /dev/tty so ratatui's
    // stdout buffering does not swallow the sequence.
    copy_via_osc52(text).map_err(|_| {
        "clipboard unavailable (allow OSC 52 in the terminal, or install wl-copy/xclip)".to_string()
    })
}

fn copy_via_cli(text: &str) -> bool {
    if pipe_stdin("wl-copy", &[], text).is_ok() {
        return true;
    }
    if cfg!(target_os = "macos") && pipe_stdin("pbcopy", &[], text).is_ok() {
        return true;
    }
    // Only touch X11 tools when DISPLAY is set — otherwise xclip can hang.
    if std::env::var_os("DISPLAY").is_none() {
        return false;
    }
    pipe_stdin("xclip", &["-selection", "clipboard"], text).is_ok()
        || pipe_stdin("xsel", &["--clipboard", "--input"], text).is_ok()
}

fn pipe_stdin(bin: &str, args: &[&str], text: &str) -> Result<(), ()> {
    let mut child = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ())?;
    {
        let mut stdin = child.stdin.take().ok_or(())?;
        stdin.write_all(text.as_bytes()).map_err(|_| ())?;
    }
    // Don't block forever if the clipboard daemon is wedged.
    let deadline = std::time::Instant::now() + Duration::from_millis(800);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() { Ok(()) } else { Err(()) };
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(());
            }
        }
    }
}

/// OSC 52 — ask the terminal emulator to place text on the system clipboard.
fn copy_via_osc52(text: &str) -> Result<(), String> {
    const MAX: usize = 100_000;
    let slice = if text.len() > MAX { &text[..MAX] } else { text };
    let encoded = base64_encode(slice.as_bytes());
    let seq_bel = format!("\x1b]52;c;{encoded}\x07");
    let seq_st = format!("\x1b]52;c;{encoded}\x1b\\");
    write_tty(seq_bel.as_bytes())?;
    write_tty(seq_st.as_bytes())?;
    Ok(())
}

fn write_tty(bytes: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    {
        if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") {
            tty.write_all(bytes).map_err(|e| e.to_string())?;
            tty.flush().map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    let mut out = std::io::stdout();
    out.write_all(bytes).map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64_encode;

    #[test]
    fn base64_hello() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
    }
}
