# Architecture

rusticlens is a workspace of three crates sharing a single Kubernetes engine.

## Crate layout

```
┌─────────────┐     ┌─────────────┐
│   rl-app    │     │   rl-tui    │
│  (egui GUI) │     │ (ratatui)   │
└──────┬──────┘     └──────┬──────┘
       │                   │
       └─────────┬─────────┘
                 ▼
          ┌─────────────┐
          │   rl-core   │
          │  (kube-rs)  │
          └─────────────┘
```

### rl-core

The cluster engine. Responsibilities:

| Module | Role |
|--------|------|
| `config` | Load kubeconfig, list/switch contexts |
| `cluster` | `ClusterManager` — connection lifecycle, namespace, watches |
| `store` | Namespace/cluster watchers, in-memory snapshots |
| `ops` | List, get YAML, delete, log streaming |
| `events` | Fetch events for a resource |
| `containers` | Resolve pod containers for logs/exec |
| `portforward` | kubectl exec/attach helpers; native port-forward in `native_portforward` |
| `native_portforward` | kube-rs WebSocket port-forward (pods and services) |
| `exec_session` | Bidirectional pod exec attach for embedded terminal |
| `plugin_loader` | Load TOML plugin manifests from config dir |
| `crd` | Discover CRDs, list custom resource instances |
| `helm` | List Helm releases from `owner=helm` secrets |
| `metrics` | Pod metrics via metrics-server API |
| `settings` | Persist last context/namespace/kind/container |
| `plugins` | Plugin registry + built-in hooks |

### rl-app

Native desktop UI using **eframe/egui**.

- **Main thread:** egui event loop, panels, keyboard shortcuts
- **Backend thread:** dedicated Tokio runtime, `mpsc` command/event bridge
- **500 ms tick:** push watch snapshots and forward log lines to UI

UI panels:

1. Cluster tabs — multi-context switcher with persisted open tabs
2. Sidebar — resource kinds by category; CRD type selector
3. Central — virtual-scrolled resource table with filter
4. Bottom — describe / events / logs / metrics tabs
5. Status bar — connection summary, port-forwards, row count

### rl-tui

Lightweight terminal browser for SSH sessions or low-resource environments. Uses the same `ClusterManager` directly on a Tokio runtime (no backend thread).

## Data flow (GUI)

```
User action → BackendCommand → ClusterManager → Kubernetes API
                    ↓
              BackendEvent → UI state update → egui repaint
```

Watchers run continuously per namespace. Helm releases and CRD instances are polled on a slower interval (3 s) when that kind is active.

## Design choices

- **Embedded terminal (optional)** — `embedded-terminal` feature uses kube attach API in a simple in-app window; external `kubectl exec` terminal remains available
- **Native port-forward** — kube-rs `Portforward` by default; kubectl subprocess fallback via settings
- **Plugins** — TOML manifests with shell `on_connect` hooks (WASM deferred)
- **CRD support via discovery** — dynamic `Api<DynamicObject>` instead of codegen for every CRD
- **mimalloc** — optional global allocator to reduce allocator overhead in long-running GUI sessions

## Memory profile

Release build with thin LTO + strip:

- Binary: ~18–25 MB on disk
- Idle RSS target: ~30–50 MB (vs 300–800+ MB for Electron-based Lens/Freelens)
