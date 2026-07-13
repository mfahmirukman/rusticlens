# rusticlens

A native Rust Kubernetes IDE — a lightweight Freelens/Lens alternative without Electron.

Built with **egui** for the desktop UI, **ratatui** for the terminal UI, and **kube-rs** for cluster communication. Typical GUI memory use is ~100–150 MB RSS (egui + async runtime + cluster watches), vs 300–800+ MB for Electron-based clients.

**Current version: 0.5.0**

## What's implemented

| Area | GUI (`rl-app`) | TUI (`rl-tui`) |
|------|----------------|----------------|
| Context / namespace switching | Yes | Yes |
| Multi-cluster tabs | Yes | — |
| Multi-kubeconfig merge | Yes | Uses merged config from disk |
| Resource browsers (20 kinds + CRD + Helm) | Yes | Yes |
| Live watches + virtual scroll | Yes | Yes |
| Describe / Events / Metrics | Yes | Yes |
| Pod logs | Yes (polling) | Yes |
| Scale / restart workloads | Yes | — |
| Apply YAML | Yes | — |
| Favorites | Yes | — |
| Cluster overview dashboard | Yes | — |
| Port-forward | Yes (native + kubectl) | — |
| Embedded shell | Yes (optional feature) | — |
| External terminal (`kubectl exec`) | Yes | — |
| Dark / light theme | Yes | — |
| TOML plugins (`on_connect`) | Yes | — |

See [Roadmap](docs/ROADMAP.md) for version history (v0.1–v0.5) and future work.

## Features (GUI)

### Cluster & navigation
- Load kubeconfig and switch contexts (including Teleport after `tsh login`)
- **Icon rail** for pinned contexts + **multi-cluster tabs** (open/switch/close contexts; tab list persisted)
- Freelens-inspired layout: collapsible sidebar, workload tabs, resource list + bottom logs panel
- **50/50 vertical split** between resource list and logs on first launch (until you resize)
- **Detail panel** (Describe / Events / Metrics) on the right when a resource is selected — close with **✕** or **Esc**
- Namespace selector; UI state persisted in `~/.config/rusticlens/settings.json`
- **Cluster settings** — merge extra kubeconfig files and toggle native port-forward (Ctrl+K → Cluster settings)

### Resources (20 built-in kinds)
- **Workloads:** Pods, Deployments, StatefulSets, Jobs, Cron Jobs
- **Network:** Services, Ingresses, Network Policies
- **Storage:** PVCs, Storage Classes
- **Access:** Roles, Role Bindings, Cluster Roles, Cluster Role Bindings
- **Config:** ConfigMaps, Secrets
- **Cluster:** Namespaces, Nodes, Helm releases (from `owner=helm` secrets)
- **Custom:** CRD browser with per-type instance listing
- **Favorites:** pin from context menu; jump from sidebar
- **Overview:** namespace workload counts + cluster-wide pod stats + per-node CPU/memory (metrics-server)

### Operations
- Live watches with filterable, virtual-scrolled tables
- YAML describe with selectable text and in-panel search (**Ctrl+F**, Enter / Shift+Enter)
- Kubernetes events panel
- Pod logs via **time-based polling** (~10s) with container picker, follow-tail, load-older, search (`/`), copy/save
- Pod metrics panel (requires metrics-server)
- Delete with confirmation; CronJob trigger / suspend / resume
- Deployment & StatefulSet **scale** and **rollout restart** (context menu)
- **Apply YAML** (server-side apply) via command palette (**Ctrl+K**)
- **Dark / light theme** toggle (persisted)
- Copy `kubectl edit` / `kubectl exec`; open **external terminal** (`kubectl exec -it`)
- **Embedded shell** — in-app `/bin/sh` attach via kube API (see [known limitations](#known-limitations--bugs))
- **Port-forward** — native kube-rs for pods (default); kubectl fallback for services or when native fails
- **TOML plugins** — `~/.config/rusticlens/plugins/*.toml` with optional `on_connect` shell hook
- Command palette (**Ctrl+K**)

### Plugins

Drop a `.toml` file in `~/.config/rusticlens/plugins/` (an `example.toml` is created on first launch):

```toml
name = "my-hook"
description = "Runs when a cluster connection succeeds"
on_connect = "echo \"connected to $RUSTICLENS_CONTEXT\""
```

`on_connect` runs as `sh -c` in the background when you connect or switch context.

## Features (TUI)

- Sidebar navigation across all resource kinds (including Storage, Access, Network Policies)
- Resource table with filter/search on pods
- Detail panel: Describe / Events / Metrics
- Pod log viewer with search (`/`), follow (`f`), scroll
- Context picker (`c`), namespace picker (`n`)
- Keyboard: `q` quit, `r` refresh, `d` detail, `L` logs, `Tab` cycle kinds, `y` cycle CRD type, `1`/`2`/`3` detail tabs

The TUI does **not** yet include favorites, overview dashboard, port-forward, apply YAML, scale/restart dialogs, theme toggle, or embedded shell.

## Known limitations & bugs

These are current behavioral limits worth knowing before daily use:

### Port-forward
- **Fixed ports** — palette “Start port-forward” always uses `localhost:8080 → :80` (no port picker yet).
- **Pods only (native)** — kube-rs port-forward works for pods. **Services** always use a kubectl subprocess.
- **kubectl fallback not tracked** — when port-forward falls back to kubectl, the UI “Stop” button removes the entry but **does not kill** the background `kubectl port-forward` process. Use `pkill -f "kubectl port-forward"` or restart the app if needed.
- **Single local bind** — if port 8080 is already in use, forward setup fails.

### Embedded shell
- **Not a full terminal** — line-based stdout/stderr in a text window; no PTY, no raw mode, no resize signal.
- **Shell only** — attaches with `/bin/sh`; interactive TUIs (`vim`, `top`, `htop`) will not work well.
- **One session** — only one embedded exec at a time.

### Logs
- **Polling, not streaming** — new log lines are fetched on a ~10s interval (Freelens-style `sinceTime` polling), not a live Kubernetes watch stream.
- **Large logs** — “load older” chunks are capped; very chatty pods may feel sluggish.

### Multi-cluster
- **One active connection** — tabs switch contexts quickly and restore cached lists per context/namespace, but watches run for **one cluster at a time** (not parallel multi-cluster dashboards).

### Cluster & auth
- **Teleport / OIDC** — relies on kubeconfig `exec` plugins; run `tsh login` (or equivalent) before starting rusticlens.
- **Metrics & overview** — node CPU/memory on Overview needs **metrics-server**; without it, those fields stay empty.
- **Helm releases** — listed from Helm ownership secrets in the **current namespace** only (not cluster-wide release inventory).

### Plugins
- **Shell hooks only** — no WASM or native library plugins; no `on_disconnect` or UI extension points.
- **Fire-and-forget** — `on_connect` spawns `sh -c` with no output capture or error surfacing in the app.

### External terminal
- Requires a supported emulator on `PATH` (`gnome-terminal`, `konsole`, `kitty`, `alacritty`, `xterm`, etc.). If none is found, use **Copy kubectl exec** or the embedded shell.

### Packaging
- Flatpak / Debian / Homebrew files under `packaging/` are **scaffolds** — not published to Flathub, apt repos, or Homebrew core yet.

### General
- **Early-stage** — error handling and edge cases (RBAC denied, stale watches, very large clusters) are still being hardened.
- **No automated E2E tests** against a live cluster in CI; manual verification with kind/minikube is recommended after changes.

Please [open an issue](https://github.com/mfahmirukman/rusticlens/issues) if you hit something not listed here.

## Requirements

- Rust 1.75+
- A valid `~/.kube/config` (or `KUBECONFIG`)
- `kubectl` on PATH — optional for most GUI flows if native port-forward stays enabled; still needed for service port-forward, external terminal, and kubectl fallback
- Access to a Kubernetes cluster

### Teleport

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

### Build options

```bash
# Without embedded shell (smaller feature set)
cargo run -p rl-app --no-default-features --features mimalloc

# Without mimalloc allocator
cargo build -p rl-app --release --no-default-features --features embedded-terminal
```

### Packaging (scaffolds)

- Flatpak: `packaging/flatpak/com.rusticlens.Rusticlens.yml`
- Debian: `packaging/debian/`
- Homebrew: `packaging/homebrew/rusticlens.rb`

## Keyboard shortcuts (GUI)

| Key | Action |
|-----|--------|
| `Ctrl+K` | Command palette |
| `R` | Refresh watches |
| `D` | Describe selected resource |
| `L` | Open logs for selected pod |
| `E` | Show events for selected resource |
| `Esc` | Close detail panel |
| `Ctrl+F` | Focus search in Describe / Events |
| `/` | Focus log search (when a log tab is open) |

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
packaging/   # Flatpak, Debian, Homebrew scaffolds
docs/
  ARCHITECTURE.md
  DEVELOPMENT.md
  ROADMAP.md
```

## Memory tuning

Release builds use thin LTO and `mimalloc` (enabled by default).

## Documentation

- [Architecture](docs/ARCHITECTURE.md)
- [Development guide](docs/DEVELOPMENT.md)
- [Roadmap](docs/ROADMAP.md)

## License

MIT
