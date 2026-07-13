# Development

## Prerequisites

- Rust toolchain (1.75+)
- `kubectl` configured against a test cluster (kind/minikube recommended)
- Optional: `metrics-server` for pod metrics panel
- Optional: Helm 3 for helm release secrets in cluster namespace

## Workspace commands

```bash
# Format
cargo fmt --all

# Lint
cargo clippy --all -- -D warnings

# Test
cargo test --all

# Debug GUI
cargo run -p rl-app

# Debug TUI
cargo run -p rl-tui

# Release binaries
cargo build --release -p rl-app
cargo build --release -p rl-tui
```

## Project conventions

- **rl-core** must stay UI-agnostic — no egui/ratatui imports
- New resource kinds: add to `ResourceKind` in `resources/mod.rs`, then wire `store.rs` watcher + `ops.rs` get/delete
- On-demand kinds (Helm, CRD) use `ClusterManager::list_rows()` instead of watches
- Settings path: `~/.config/rusticlens/settings.json` (or `$XDG_CONFIG_HOME/rusticlens/`)

## Adding a resource kind

1. Add variant to `ResourceKind` with `label()`, `category()`, `api_kind()`
2. Add watcher in `store.rs` (or on-demand list in `helm.rs` / `crd.rs`)
3. Extend `ops::get_resource_yaml` and `ops::delete_resource`
4. Update sidebar categories if needed (automatic via `category()`)
5. Run `cargo clippy --all -- -D warnings`

## Debugging cluster connection

```bash
# Verify kubectl works
kubectl get pods -A

# Teleport
tsh status
tsh kube ls

# Enable rusticlens logs
RUST_LOG=rl_core=debug,rusticlens=debug cargo run -p rl-app
```

## CI

GitHub Actions runs `fmt`, `clippy`, `test`, and release build on push/PR. See `.github/workflows/ci.yml`.

## Releases (GitHub Actions)

Pushing an annotated semver tag triggers `.github/workflows/release.yml`, which:

1. Builds `rusticlens` (GUI) and `rusticlens-tui` (TUI) on Linux, macOS (Apple Silicon), and Windows
2. Packages per-platform archives (`rusticlens-<version>-<platform>.tar.gz` or `.zip`)
3. Publishes a GitHub Release with binaries and `SHA256SUMS.txt`

**Steps to cut a release:**

```bash
# 1. Bump [workspace.package] version in Cargo.toml (root), commit, push
git add Cargo.toml Cargo.lock
git commit -m "chore(release): bump version to 0.5.0"
git push origin develop

# 2. Tag the release commit (tag must match Cargo.toml version)
git tag -a v0.5.0 -m "rusticlens v0.5.0"
git push origin v0.5.0
```

GitHub Actions will create the release at `https://github.com/mfahmirukman/rusticlens/releases/tag/v0.5.0` with downloadable assets.

The tag version **must** match `version` in the root `Cargo.toml` or the workflow fails.

## Plugin API (experimental)

`rl-core::plugins` exposes `RusticlensPlugin` and `PluginRegistry`. The built-in `LoggingPlugin` fires on cluster connect. Future versions may load shared libraries or WASM — not implemented in v0.2.
