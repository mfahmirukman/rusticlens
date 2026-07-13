# Roadmap

## v0.1 (done)

- [x] egui MVP: contexts, namespaces, pods/deployments/services/configmaps/namespaces
- [x] YAML describe, pod logs, delete, kubectl exec copy
- [x] kube-rs watchers, CI pipeline

## v0.2 (done)

- [x] Events panel for selected resources
- [x] Container picker for multi-container pod logs
- [x] Status bar with connection summary
- [x] Persist UI state (context, namespace, kind, container)
- [x] StatefulSets, Jobs, Ingresses, Secrets, Nodes
- [x] Port-forward (kubectl-based)
- [x] External terminal for kubectl exec
- [x] CRD browser
- [x] Command palette (Ctrl+K)
- [x] Helm release listing
- [x] Pod metrics (metrics-server)
- [x] Plugin registry skeleton
- [x] TUI (`rl-tui` with ratatui)
- [x] Documentation (README, ARCHITECTURE, DEVELOPMENT, ROADMAP)
- [x] Detail panel search and selectable describe/events text
- [x] 50/50 log panel split until user resizes

## v0.3 (done)

- [x] Apply YAML from editor (server-side apply via command palette)
- [x] Replica scaling for Deployments and StatefulSets (context menu + dialog)
- [x] Rollout restart for Deployments and StatefulSets
- [x] Log export (copy to clipboard + save to file)
- [x] Log search in GUI panel (filter, highlight, `/` focus, Enter navigation)
- [x] Dark/light theme toggle (command palette, persisted in settings)
- [x] Multi-kubeconfig merge UI (extra paths in cluster settings, reload contexts)
- [x] TUI: logs streaming, context switcher

## v0.4 (done)

- [x] Aggregated cluster dashboard (node pressure, cluster-wide pod counts on Overview)
- [x] Network policy viewer (Network Policies in sidebar)
- [x] PVC / StorageClass browser (Storage section)
- [x] RBAC viewer (Roles, Role Bindings, Cluster Roles, Cluster Role Bindings)
- [x] Favorites / pinned resources (sidebar + context menu, persisted)

## v0.5 (done)

- [x] Plugin loading via TOML manifests in `~/.config/rusticlens/plugins/`
- [x] Built-in port-forward without kubectl subprocess (kube-rs WebSocket)
- [x] Embedded terminal (optional `embedded-terminal` feature flag)
- [x] Multi-cluster tabs (context tab bar + persisted open tabs)
- [x] Packaging scaffolds: Flatpak, .deb, Homebrew formula

## Non-goals

- Replacing `kubectl` for scripting/automation
- Full Helm chart management (use `helm` CLI)
- 1:1 Freelens extension compatibility (Electron extension model does not port)
