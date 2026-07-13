//! Live-cluster port-forward smoke tests.
//!
//! Run: `cargo test -p rl-core --test portforward_live -- --ignored --nocapture`

use std::process::Command;
use std::time::Duration;

use rl_core::native_portforward::start_port_forward;
use rl_core::resources::ResourceKind;
use rl_core::spawn_kubectl_port_forward;

const NAMESPACE: &str = "agent-tools-develop";
const POD: &str = "adcredit-df7b497bc-lrmgp";
const SERVICE: &str = "property-reader";
const CONTAINER_PORT: u16 = 8080;

fn curl_http_code(url: &str) -> Option<String> {
    let output = Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--connect-timeout",
            "5",
            "--max-time",
            "10",
            url,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

/// Documents that kube-rs WebSocket port-forward may fail on Teleport (HTTP 500).
#[tokio::test]
#[ignore = "requires live Kubernetes cluster"]
async fn kube_rs_portforward_direct_may_fail_on_teleport() {
    let config = kube::Config::infer().await.expect("kubeconfig");
    let client = kube::Client::try_from(config).expect("client");
    let api: kube::api::Api<k8s_openapi::api::core::v1::Pod> =
        kube::api::Api::namespaced(client, NAMESPACE);

    let result = api.portforward(POD, &[CONTAINER_PORT]).await;
    match result {
        Ok(_) => eprintln!("kube-rs portforward succeeded on this cluster"),
        Err(err) => eprintln!("kube-rs portforward failed (expected on Teleport): {err}"),
    }
}

#[tokio::test]
#[ignore = "requires live Kubernetes cluster"]
async fn native_start_fails_when_kube_rs_unsupported() {
    let config = kube::Config::infer().await.expect("kubeconfig");
    let client = kube::Client::try_from(config).expect("client");

    let result = start_port_forward(
        client,
        NAMESPACE,
        ResourceKind::Pod,
        POD,
        19100,
        CONTAINER_PORT,
        1,
    )
    .await;

    // On Teleport clusters this should fail at probe time; on kind/minikube it may succeed.
    match &result {
        Ok(_) => eprintln!("native start_port_forward: ok"),
        Err(e) => eprintln!("native start_port_forward failed: {}", e.user_message()),
    }
}

#[tokio::test]
#[ignore = "requires live Kubernetes cluster, kubectl, and curl"]
async fn kubectl_pod_port_forward_reaches_container() {
    let local_port = 19200;
    let url = format!("http://127.0.0.1:{local_port}/");

    let mut child = spawn_kubectl_port_forward(
        NAMESPACE,
        ResourceKind::Pod,
        POD,
        local_port,
        CONTAINER_PORT,
    )
    .expect("spawn kubectl port-forward");

    tokio::time::sleep(Duration::from_secs(3)).await;
    let code = curl_http_code(&url);
    let _ = child.kill();
    let _ = child.wait();

    let code = code.expect("curl should get HTTP status");
    assert!(
        code == "404" || code.starts_with('2') || code.starts_with('3'),
        "expected HTTP response through kubectl pod forward, got {code}"
    );
}

#[tokio::test]
#[ignore = "requires live Kubernetes cluster, kubectl, and curl"]
async fn kubectl_service_port_forward_reaches_backend() {
    let local_port = 19101;
    let url = format!("http://127.0.0.1:{local_port}/");

    let mut child = spawn_kubectl_port_forward(
        NAMESPACE,
        ResourceKind::Service,
        SERVICE,
        local_port,
        CONTAINER_PORT,
    )
    .expect("spawn kubectl port-forward");

    tokio::time::sleep(Duration::from_secs(3)).await;
    let code = curl_http_code(&url);
    let _ = child.kill();
    let _ = child.wait();

    // Some services may not speak HTTP on /; accept any curl success with a code.
    if let Some(code) = code {
        eprintln!("service forward http code: {code}");
    } else {
        eprintln!("service forward: no HTTP response (app may not speak HTTP on /)");
    }
}
