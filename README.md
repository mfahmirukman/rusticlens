# rusticlens

A native Rust Kubernetes IDE — a lightweight Freelens/Lens alternative without Electron.

Built with **egui** for the desktop UI, **ratatui** for the terminal UI, and **kube-rs** for cluster communication. Targets ~30–50 MB idle memory vs 300–800+ MB for Electron-based clients.

## Features (v0.2)

### Cluster & navigation
- Load kubeconfig and switch contexts (including Teleport after `tsh login`)
- Freelens-inspired dark layout: icon rail, collapsible sidebar, workload tabs, split list + bottom panel
- Namespace selector with persisted UI state (`~/.config/rusticlens/settings.json`)
- Categorized sidebar: Workloads, Network, Config, Cluster, Custom

### Resources
- **Workloads:** Pods, Deployments, StatefulSets, Jobs, **Cron Jobs**
- **Network:** Services, Ingresses
- **Config:** ConfigMaps, Secrets
- **Cluster:** Namespaces, Nodes, Helm releases (via helm secrets)
- **Custom:** CRD browser with per-type instance listing

### Operations
- Live watches with virtual-scrolled tables
- YAML describe panel
- Kubernetes events panel
- Streaming pod logs with **container picker** (multi-container pods)
- Pod metrics (requires metrics-server)
- Delete resources (with confirmation)
- Copy `kubectl exec` / open external terminal
- Port-forward via `kubectl port-forward` (copy command or spawn)
- Command palette (**Ctrl+K**)

### Terminal UI (`rl-tui`)
- Browse pods, deployments, services, configmaps, namespaces
- Describe selected resource as YAML
- Keyboard-driven: `q` quit, `r` refresh, `d` describe, `Tab` switch kind, `Ctrl+n` namespace

## Requirements

- Rust 1.75+
- A valid `~/.kube/config` (or `KUBECONFIG` env var)
- `kubectl` on PATH (for exec, port-forward, external terminal)
- Access to a Kubernetes cluster (local or remote)

### Teleport

If your kubeconfig uses Teleport exec authentication:

```bash
tsh login --proxy=your-teleport.example.com
cargo run -p rl-app --release
```

## Build

```bash
cargo build --release
```

### Desktop GUI

```bash
cargo run -p rl-app --release
```

### Terminal UI

```bash
cargo run -p rl-tui --release
```

## Keyboard shortcuts (GUI)

| Key | Action |
|-----|--------|
| `Ctrl+K` | Command palette |
| `R` | Refresh watches |
| `D` | Describe selected resource |
| `L` | Stream logs (pods only) |
| `E` | Show events for selected resource |

## Testing with a local cluster

### kind

```bash
kind create cluster
cargo run -p rl-app --release
```

### minikube

```bash
minikube start
cargo run -p rl-app --release
```

Deploy a sample workload:

```bash
kubectl create deployment nginx --image=nginx
kubectl get pods
```

## Project structure

```
crates/
  rl-core/   # Kubernetes engine (kube-rs, watchers, ops, CRD, metrics, helm)
  rl-app/    # egui desktop application
  rl-tui/    # ratatui terminal browser
docs/
  ARCHITECTURE.md
  DEVELOPMENT.md
  ROADMAP.md
```

## Memory tuning

Release builds use thin LTO and `mimalloc` (enabled by default). To build without mimalloc:

```bash
cargo build -p rl-app --release --no-default-features
```

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Development guide](docs/DEVELOPMENT.md)
- [Roadmap](docs/ROADMAP.md)

## License

MIT
