# Roadmap

## v0.1 (done)

- [x] egui MVP: contexts, namespaces, pods/deployments/services/configmaps/namespaces
- [x] YAML describe, pod logs, delete, kubectl exec copy
- [x] kube-rs watchers, CI pipeline

## v0.2 (current)

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

## v0.3 (planned)

- [ ] Resource creation / apply YAML from editor
- [ ] Replica scaling for deployments/statefulsets
- [ ] Rollout restart
- [ ] Log search and export
- [ ] Multiple kubeconfig files / merge support
- [ ] Dark/light theme toggle in egui
- [ ] TUI: logs streaming, context switcher

## v0.4 (planned)

- [ ] Aggregated cluster dashboard (node pressure, pod counts)
- [ ] Network policy viewer
- [ ] PVC / StorageClass browser
- [ ] RBAC viewer (roles, bindings)
- [ ] Favorites / pinned resources

## v0.5 (planned)

- [ ] Plugin loading (WASM or native)
- [ ] Built-in port-forward without kubectl subprocess
- [ ] Embedded terminal (optional feature flag)
- [ ] Multi-cluster tabs
- [ ] Packaging: Flatpak, .deb, Homebrew formula

## Non-goals

- Replacing `kubectl` for scripting/automation
- Full Helm chart management (use `helm` CLI)
- 1:1 Freelens extension compatibility (Electron extension model does not port)
