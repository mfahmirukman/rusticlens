use std::net::SocketAddr;

use k8s_openapi::api::core::v1::Pod;
use kube::api::Api;
use kube::Client;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::error::{Error, Result};
use crate::resources::ResourceKind;

#[derive(Debug, Clone)]
pub struct PortForwardInfo {
    pub id: u64,
    pub label: String,
    pub local_port: u16,
    pub remote_port: u16,
}

pub struct PortForwardHandle {
    pub info: PortForwardInfo,
    cancel_tx: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl PortForwardHandle {
    pub fn stop(self) {
        let _ = self.cancel_tx.send(());
        self.task.abort();
    }
}

pub async fn probe_native_port_forward(
    client: Client,
    namespace: &str,
    pod_name: &str,
    remote_port: u16,
) -> Result<()> {
    let api: Api<Pod> = Api::namespaced(client, namespace);
    let mut forwarder = api
        .portforward(pod_name, &[remote_port])
        .await
        .map_err(|err| Error::Message(format!("native port-forward unavailable: {err}")))?;
    if forwarder.take_stream(remote_port).is_none() {
        return Err(Error::Message(
            "native port-forward unavailable: port stream missing".into(),
        ));
    }
    let _ = forwarder.join().await;
    Ok(())
}

pub async fn start_port_forward(
    client: Client,
    namespace: &str,
    kind: ResourceKind,
    name: &str,
    local_port: u16,
    remote_port: u16,
    id: u64,
) -> Result<PortForwardHandle> {
    let label = format!("{}/{}:{remote_port}", kind.api_kind(), name);
    let addr = SocketAddr::from(([127, 0, 0, 1], local_port));
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|err| Error::Message(format!("bind {addr}: {err}")))?;
    let bound_port = listener
        .local_addr()
        .map_err(|err| Error::Message(err.to_string()))?
        .port();

    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    let namespace = namespace.to_string();
    let name = name.to_string();

    let task = match kind {
        ResourceKind::Service => {
            return Err(Error::Message(
                "native port-forward for services is not supported; use kubectl fallback".into(),
            ));
        }
        _ => {
            probe_native_port_forward(client.clone(), &namespace, &name, remote_port).await?;
            let api: Api<Pod> = Api::namespaced(client, &namespace);
            spawn_forward_loop(listener, cancel_rx, move |conn| {
                let api = api.clone();
                let name = name.clone();
                async move { forward_pod(&api, &name, remote_port, conn).await }
            })
        }
    };

    Ok(PortForwardHandle {
        info: PortForwardInfo {
            id,
            label,
            local_port: bound_port,
            remote_port,
        },
        cancel_tx,
        task,
    })
}

fn spawn_forward_loop<F, Fut>(
    listener: TcpListener,
    mut cancel_rx: oneshot::Receiver<()>,
    handler: F,
) -> JoinHandle<()>
where
    F: Fn(tokio::net::TcpStream) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut cancel_rx => break,
                accepted = listener.accept() => {
                    let Ok((conn, _)) = accepted else { continue };
                    let fut = handler(conn);
                    tokio::spawn(fut);
                }
            }
        }
    })
}

async fn forward_pod(
    api: &Api<Pod>,
    pod_name: &str,
    port: u16,
    mut client_conn: tokio::net::TcpStream,
) {
    let forwarder_result = api.portforward(pod_name, &[port]).await;
    let Ok(mut forwarder) = forwarder_result else {
        if let Err(err) = forwarder_result {
            tracing::warn!(pod = pod_name, port, error = %err, "portforward API call failed");
        }
        return;
    };
    let Some(mut upstream) = forwarder.take_stream(port) else {
        tracing::warn!(pod = pod_name, port, "port not available in forwarder");
        return;
    };
    let copy_result = tokio::io::copy_bidirectional(&mut client_conn, &mut upstream).await;
    drop(upstream);
    if let Err(err) = copy_result {
        tracing::warn!(pod = pod_name, port, error = %err, "port-forward copy failed");
    }
    if let Err(err) = forwarder.join().await {
        tracing::warn!(pod = pod_name, port, error = %err, "port-forward join failed");
    }
}
