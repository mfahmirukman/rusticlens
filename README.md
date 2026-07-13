# rusticlens

A native Rust Kubernetes IDE — a lightweight Freelens/Lens alternative without Electron.

Built with **egui** for the desktop UI, **ratatui** for the terminal UI, and **kube-rs** for cluster communication. Targets ~30–50 MB idle memory vs 300–800+ MB for Electron-based clients.

**Current version: 0.2.0**

## Features (v0.2)

### Cluster & navigation
- Load kubeconfig and switch contexts (including Teleport after `tsh login`)
- Freelens-inspired dark layout: icon rail, collapsible sidebar, workload tabs, resource list + bottom logs panel
- **50/50 vertical split** between the resource list and logs panel on first launch (until you resize the divider)
- **Detail panel** (Describe / Events / Metrics) opens on the right **only when you select a resource** — close with **✕** or **Esc** to reclaim space for the main area
- Namespace selector with persisted UI state (`~/.config/rusticlens/settings.json`)
- Categorized sidebar: Workloads, Network, Config, Cluster, Custom

### Resources
- **Workloads:** Pods, Deployments, StatefulSets, Jobs, Cron Jobs
- **Network:** Services, Ingresses
- **Config:** ConfigMaps, Secrets
- **Cluster:** Namespaces, Nodes, Helm releases (via helm secrets)
- **Custom:** CRD browser with per-type instance listing

### Operations
- Live watches with virtual-scrolled tables
- YAML describe panel with **selectable text** and **in-panel search** (highlight matches, **Enter** / **Shift+Enter** or ▲/▼ for next/previous, **Ctrl+F** to focus search)
- Kubernetes events panel
- Pod logs via **time-based polling** (~10s, Freelens-style) with **container picker** (multi-container pods), follow-tail, and load-older
- Pod metrics (requires metrics-server)
- Delete resources (with confirmation)
- Deployment rollout restart; CronJob trigger / suspend / resume
- Copy `kubectl edit` / `kubectl exec` / open external terminal
- Port-forward via `kubectl port-forward` (copy command or spawn)
- Command palette (**Ctrl+K**)

### Terminal UI (`rl-tui`)
- Freelens-inspired layout: navigation sidebar, resource table, detail panel (Describe / Events / Metrics)
- All resource kinds from the GUI (Workloads, Network, Config, Cluster, Helm, CRD)
- Pod log viewer with search (`/`), follow (`f`), and scroll
- Context picker (`c`) and namespace picker (`n`)
- Non-blocking startup with visible connecting/error states
- Keyboard-driven: `q` quit, `r` refresh/retry, `d` load detail, `L` logs, `Tab`/`Shift+Tab` switch kind, `y` cycle CRD type, `h`/`l` focus panes, `1`/`2`/`3` detail tabs

## Not yet implemented

See [Roadmap](docs/ROADMAP.md) for planned work. Highlights:

- **v0.3:** In-app YAML apply/create, replica scaling, GUI log search/export, multi-kubeconfig merge, light theme
- **v0.4:** Cluster dashboard, NetworkPolicy/PVC/RBAC viewers, pinned resources
- **v0.5:** Plugin loading, native port-forward, embedded terminal, multi-cluster tabs, distro packaging

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
| `D` | Describe selected resource (opens detail panel) |
| `L` | Open logs for selected pod |
| `E` | Show events for selected resource |
| `Esc` | Close detail panel (when a resource is selected) |
| `Ctrl+F` | Focus search in Describe / Events detail panel |

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
