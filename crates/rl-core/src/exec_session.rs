use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, AttachParams};
use kube::Client;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::containers;
use crate::error::{Error, Result};

/// Bidirectional pod exec session (stdin/stdout/stderr over kube attach API).
pub async fn run_pod_exec(
    client: Client,
    namespace: &str,
    pod_name: &str,
    container: Option<&str>,
    output_tx: mpsc::UnboundedSender<String>,
    mut input_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<()> {
    let resolved = containers::resolve_container(&client, namespace, pod_name, container).await?;
    let api: Api<Pod> = Api::namespaced(client, namespace);
    let attach = AttachParams {
        stdin: true,
        stdout: true,
        stderr: true,
        tty: true,
        container: Some(resolved),
        ..Default::default()
    };

    let mut attached = api
        .exec(pod_name, vec!["/bin/sh"], &attach)
        .await
        .map_err(|err| Error::Message(err.to_string()))?;

    let mut stdin = attached
        .stdin()
        .ok_or_else(|| Error::Message("exec stdin unavailable".into()))?;
    let stdout = attached
        .stdout()
        .ok_or_else(|| Error::Message("exec stdout unavailable".into()))?;
    let stderr = attached.stderr();

    if let Some(stderr) = stderr {
        let err_tx = output_tx.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(text)) = lines.next_line().await {
                if err_tx.send(format!("[stderr] {text}")).is_err() {
                    break;
                }
            }
        });
    }

    let mut stdout_lines = BufReader::new(stdout).lines();
    loop {
        tokio::select! {
            _ = &mut cancel_rx => break,
            line = stdout_lines.next_line() => {
                match line {
                    Ok(Some(text)) => {
                        if output_tx.send(text).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(err) => {
                        let _ = output_tx.send(format!("[stdout error] {err}"));
                        break;
                    }
                }
            }
            input = input_rx.recv() => {
                match input {
                    Some(bytes) => {
                        if stdin.write_all(&bytes).await.is_err() {
                            break;
                        }
                        let _ = stdin.flush().await;
                    }
                    None => break,
                }
            }
        }
    }

    Ok(())
}
