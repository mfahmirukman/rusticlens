# GUI Follow-TUI Behaviors Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port TUI SharedManager concurrency, soft-refuse, cluster cache, and on-demand detail panel into `rl-app` while keeping the egui ↔ backend mpsc bridge.

**Architecture:** Backend holds `Option<Arc<RwLock<ClusterManager>>>` and spawns exclusive connect/switch/refresh work so ticks/`Fetch*` keep running via `try_read`. UI tracks exclusive busy + `ClusterCache`, soft-refuses duplicate exclusive cmds, and opens the detail side panel only on explicit Describe (`d`).

**Tech Stack:** Rust, Tokio (`RwLock`, `spawn`), egui, `rl-core::{ClusterManager, ClusterCache, load_cluster_cache, save_cluster_cache}`.

**Spec:** `docs/superpowers/specs/2026-07-19-gui-follow-tui-design.md`

## Global Constraints

- Do not put `ClusterManager` on the egui thread.
- Do not remove the mpsc `BackendCommand` / `BackendEvent` bridge.
- Soft-refuse status text: `Busy — wait for current request…` (match TUI intent).
- Cluster cache path: same as TUI (`load_cluster_cache` / `save_cluster_cache`).
- Detail: Esc closes panel without clearing `table.selected`.
- `SetActiveKind` / `RefreshList` are **not** exclusive (short/inline like TUI kind switch); exclusive = connect, switch context, set namespace, reconnect, refresh watch.
- Out of scope: mouse CSI, TUI pan keys as primary UX.

## File map

| File | Responsibility |
|------|----------------|
| `crates/rl-app/src/backend.rs` | SharedManager, exclusive spawn, Busy event, try_read ticks |
| `crates/rl-app/src/app.rs` | Cache, soft-refuse, detail_panel_visible, event handling |
| `crates/rl-app/src/ui/detail_panel.rs` | No API change required unless scroll needs tweak |
| `README.md` | Note GUI shares TUI cache + soft-refuse + opt-in detail |

---

### Task 1: Backend SharedManager + Busy event + exclusive spawn

**Files:**
- Modify: `crates/rl-app/src/backend.rs`

**Interfaces:**
- Produces: `BackendEvent::Busy`, `type SharedManager = Arc<RwLock<ClusterManager>>`, exclusive soft-refuse inside backend
- Consumes: existing `BackendCommand` variants

- [ ] **Step 1: Add types and Busy event**

Add after imports / near top of `backend.rs`:

```rust
use std::sync::Arc;
use tokio::sync::RwLock;

type SharedManager = Arc<RwLock<rl_core::ClusterManager>>;

struct ExclusiveSlot {
    handle: tokio::task::JoinHandle<()>,
}
```

In `BackendEvent`:

```rust
/// Exclusive op already running (switch/connect/refresh).
Busy,
```

- [ ] **Step 2: Change loop state**

In `run_backend_loop`:

```rust
let mut manager: Option<SharedManager> = None;
let mut exclusive: Option<ExclusiveSlot> = None;
```

On each loop iteration (before `select!` or after command), reap finished exclusive:

```rust
if exclusive.as_ref().is_some_and(|e| e.handle.is_finished()) {
    exclusive = None;
}
```

Tick arms:

```rust
_ = tick.tick() => {
    if let Some(mgr) = manager.as_ref() {
        if let Ok(guard) = mgr.try_read() {
            poll_log_tabs(&guard, &mut log_polls, log_poll_interval, event_tx).await;
            push_all_snapshots(&guard, event_tx);
        }
    }
}
_ = list_tick.tick() => {
    if let Some(mgr) = manager.as_ref() {
        if let Ok(guard) = mgr.try_read() {
            refresh_on_demand_list(&guard, active_kind, event_tx).await;
        }
    }
}
```

Pass `&mut exclusive` into `handle_command`.

- [ ] **Step 3: Helper to start exclusive work**

```rust
fn try_begin_exclusive(
    exclusive: &mut Option<ExclusiveSlot>,
    event_tx: &std::sync::mpsc::Sender<BackendEvent>,
) -> bool {
    if exclusive.as_ref().is_some_and(|e| !e.handle.is_finished()) {
        let _ = event_tx.send(BackendEvent::Busy);
        return false;
    }
    exclusive.take(); // drop finished
    true
}
```

For `ConnectDefault` / `Reconnect` (replace manager):

```rust
BackendCommand::ConnectDefault => {
    if !try_begin_exclusive(exclusive, event_tx) {
        return true;
    }
    let _ = event_tx.send(BackendEvent::Connecting);
    let kind = *active_kind;
    let event_tx = event_tx.clone();
    let manager_slot = /* need Arc<Mutex<Option<SharedManager>>> OR complete inline then set */;
}
```

**Preferred pattern (simpler, matches “spawn exclusive” without racing manager pointer):** keep `manager: Option<SharedManager>` on the loop thread; for exclusive ops that **mutate** connection:

1. Soft-refuse if exclusive open.
2. `let shared = manager.clone();` (for switch/ns/refresh) or for connect create new manager in task then send result via oneshot / event only and set manager on completion via a small `ExclusiveOutcome` channel polled in the loop.

**Concrete pattern used in this plan:**

Use an `exclusive_outcome_rx: mpsc::UnboundedReceiver<ExclusiveOutcome>` filled by spawned tasks:

```rust
enum ExclusiveOutcome {
    Connected {
        manager: SharedManager, // new or same Arc after write
        context: String,
        namespace: String,
        contexts: Vec<String>,
        namespaces: Vec<String>,
        crd_targets: Vec<CrdTarget>,
    },
    Failed(String),
    RefreshDone,
}
```

Loop `select!` also receives outcomes → apply to `manager`, push snapshots under read lock, clear exclusive.

For **SwitchContext** spawn:

```rust
let Some(shared) = manager.clone() else { return true };
if !try_begin_exclusive(exclusive, event_tx) { return true; }
let _ = event_tx.send(BackendEvent::Connecting);
let kind = *active_kind;
let outcome_tx = exclusive_outcome_tx.clone();
let handle = tokio::spawn(async move {
    let mut guard = shared.write().await;
    match guard.switch_context(&context, kind).await {
        Ok(()) => {
            let namespaces = guard.list_namespaces().await.unwrap_or_default();
            let crd_targets = guard.crd_targets().to_vec();
            let contexts = rl_core::ClusterManager::list_contexts().await.unwrap_or_default();
            let _ = outcome_tx.send(ExclusiveOutcome::Connected {
                manager: shared.clone(),
                context: guard.context().to_string(),
                namespace: guard.namespace().to_string(),
                contexts,
                namespaces,
                crd_targets,
            });
        }
        Err(err) => {
            let _ = outcome_tx.send(ExclusiveOutcome::Failed(err.user_message()));
        }
    }
});
*exclusive = Some(ExclusiveSlot { handle });
```

On outcome `Connected`: set `*manager = Some(outcome.manager)`, emit `BackendEvent::Connected { ... }`, `push_all_snapshots` under read, `refresh_on_demand_list`.

For **ConnectDefault** / **Reconnect**: spawn without existing lock; on success wrap in `Arc::new(RwLock::new(mgr))` and send `Connected` outcome.

For **RefreshWatch**: exclusive spawn with write lock; outcome `RefreshDone` → push snapshots + emit nothing extra (or keep silent like today) but clear exclusive; on error `Failed`.

For **SetNamespace**: same as SwitchContext (Connecting optional — TUI shows “Switching…”; GUI may emit `Connecting` for consistency or only status on UI; emit `Connecting` for exclusive busy tracking).

- [ ] **Step 4: Convert read-path commands to SharedManager**

Change helpers to take `&rl_core::ClusterManager` still (via `guard` dereference).

In `handle_command` for Fetch*/Delete*/logs/port-forward:

```rust
BackendCommand::FetchYaml { kind, name } => {
    let Some(shared) = manager.clone() else { return true };
    let event_tx = event_tx.clone();
    tokio::spawn(async move {
        let guard = shared.read().await;
        match guard.resource_yaml(kind, &name).await {
            Ok(yaml) => { let _ = event_tx.send(BackendEvent::YamlLoaded { name, yaml }); }
            Err(err) => { let _ = event_tx.send(BackendEvent::Error(err.user_message())); }
        }
    });
}
```

Same spawn-read pattern for FetchEvents, FetchContainers, FetchMetrics, FetchOlderLogs, FetchDashboard, Delete*, Trigger*, Scale*, ApplyYaml, StartLogs (StartLogs must update `log_polls` on loop thread — **keep StartLogs synchronous with `read().await` on loop** OR send a `LogPollStarted` internal event. **Simplest:** `StartLogs` / `CloseLog` / port-forward stay **inline** `read().await` on the backend loop (may briefly wait behind write lock; acceptable). FetchYaml/Events/Metrics/Containers/Dashboard/mutations that don't touch loop-local maps: spawn with read lock.

Port-forward and StartLogs touch `log_polls` / `port_forwards` → keep inline:

```rust
if let Some(shared) = manager.as_ref() {
    let guard = shared.read().await;
    start_log_poll(&guard, ...).await;
}
```

`SetActiveKind` / `SetCrdTarget` / `RefreshList`: inline write or read as needed (not exclusive soft-refuse).

- [ ] **Step 5: Verify compile**

Run: `cargo check -p rl-app 2>&1`

Expected: success (fix any type errors from manager Option change).

- [ ] **Step 6: Commit**

```bash
git add crates/rl-app/src/backend.rs
git commit -m "feat(gui): SharedManager backend with exclusive soft-refuse"
```

---

### Task 2: UI cluster cache + soft-refuse

**Files:**
- Modify: `crates/rl-app/src/app.rs`

**Interfaces:**
- Consumes: `BackendEvent::Busy`, `Connecting`, `Connected`, `Error`
- Produces: `exclusive_busy: bool`, cache load/save, `soft_refuse_exclusive() -> bool`

- [ ] **Step 1: Add fields and helpers**

```rust
use rl_core::{load_cluster_cache, save_cluster_cache, ClusterCache, ...};

// in RusticlensApp:
exclusive_busy: bool,
cluster_cache: ClusterCache,

fn soft_refuse_exclusive(&mut self) -> bool {
    if self.exclusive_busy {
        if self.status_message.is_empty() || !self.status_message.starts_with("Busy") {
            self.status_message = "Busy — wait for current request…".into();
        }
        true
    } else {
        false
    }
}

fn save_cluster_cache_async(&self) {
    let cache = self.cluster_cache.clone();
    std::thread::spawn(move || {
        let _ = save_cluster_cache(&cache);
    });
}

fn update_cache_from_connected(&mut self, context: &str, contexts: &[String], namespaces: &[String]) {
    self.cluster_cache.contexts = contexts.to_vec();
    self.cluster_cache
        .namespaces_by_context
        .insert(context.to_string(), namespaces.to_vec());
    self.save_cluster_cache_async();
}
```

In `new()`:

```rust
let disk_cache = load_cluster_cache();
let contexts = disk_cache.contexts.clone();
let namespaces = disk_cache
    .namespaces_by_context
    .values()
    .next()
    .cloned()
    .unwrap_or_default();
// ...
contexts,
namespaces,
cluster_cache: disk_cache,
exclusive_busy: true, // ConnectDefault already sent
```

- [ ] **Step 2: Handle events**

```rust
BackendEvent::Connecting => {
    self.connecting = true;
    self.exclusive_busy = true;
    self.status_message = "Connecting…".into();
}
BackendEvent::Busy => {
    self.status_message = "Busy — wait for current request…".into();
}
BackendEvent::Connected { context, namespace, contexts, namespaces, crd_targets } => {
    self.connecting = false;
    self.exclusive_busy = false;
    self.connected = true;
    // existing field updates...
    self.update_cache_from_connected(&context, &contexts, &namespaces);
}
BackendEvent::Error(msg) => {
    self.connecting = false;
    self.exclusive_busy = false;
    // existing...
}
```

- [ ] **Step 3: Gate exclusive sends**

Before every `SwitchContext`, `SetNamespace`, `RefreshWatch`, `Reconnect`, `ConnectDefault` (if any):

```rust
if self.soft_refuse_exclusive() {
    return; // or skip send
}
self.backend.send(...);
```

Apply at all call sites in `app.rs` (cluster tabs, sidebar namespace callback, keyboard `R`, palette Refresh, reconnect button).

- [ ] **Step 4: Verify**

Run: `cargo check -p rl-app 2>&1`

Expected: success.

- [ ] **Step 5: Commit**

```bash
git add crates/rl-app/src/app.rs
git commit -m "feat(gui): cluster cache and exclusive soft-refuse"
```

---

### Task 3: On-demand detail panel

**Files:**
- Modify: `crates/rl-app/src/app.rs`
- Modify: `crates/rl-app/src/ui/detail_panel.rs` only if ScrollArea lacks horizontal scroll

**Interfaces:**
- Produces: `detail_panel_visible: bool`

- [ ] **Step 1: Field + open/close**

```rust
detail_panel_visible: bool, // default false in new()

fn open_detail_panel(&mut self) {
    if self.table.selected.is_none() || !self.connected {
        return;
    }
    self.detail_panel_visible = true;
    self.fetch_yaml_for_selection();
    self.detail_tab = DetailTab::Describe;
    self.detail.tab = DetailTab::Describe;
}

fn close_detail_panel(&mut self) {
    self.detail_panel_visible = false;
    self.detail.clear();
    self.detail_search.reset();
    // do NOT clear table.selected
}
```

- [ ] **Step 2: Change show condition**

Replace:

```rust
let show_detail =
    self.table.selected.is_some() && !self.sidebar.show_overview && self.connected;
```

With:

```rust
let show_detail = self.detail_panel_visible
    && self.table.selected.is_some()
    && !self.sidebar.show_overview
    && self.connected;
```

- [ ] **Step 3: Keyboard and selection**

`Key::D` → `self.open_detail_panel();` (not only fetch_yaml).

`Key::Escape` → if `detail_panel_visible` { `close_detail_panel(); return; }` (keep current early return).

`on_table_selection_changed`: do **not** auto-fetch/open; only reset container state. Optionally clear detail text if panel closed.

Context-menu / actions that mean “Describe” → `open_detail_panel()`.

Header close button already calls `close_detail_panel()` — ensure it uses new semantics.

- [ ] **Step 4: Horizontal scroll**

In `detail_panel.rs` `show_content`, ensure `ScrollArea::both()` or `.horizontal_scroll(true)` for Describe/Events/Metrics text.

- [ ] **Step 5: Verify**

Run: `cargo check -p rl-app 2>&1`

Expected: success.

- [ ] **Step 6: Commit**

```bash
git add crates/rl-app/src/app.rs crates/rl-app/src/ui/detail_panel.rs
git commit -m "feat(gui): on-demand detail panel like TUI"
```

---

### Task 4: README note + final check

**Files:**
- Modify: `README.md` (GUI / usage section near TUI cache bullets)

- [ ] **Step 1: Document parity**

Add under GUI or shared features:

```markdown
- GUI shares the TUI cluster cache file and soft-refuse for exclusive context/namespace/refresh ops
- GUI detail panel opens on demand (`d` / Describe); Esc closes without clearing the selected row
```

- [ ] **Step 2: Final verify**

Run: `cargo fmt --all -- --check && cargo check -p rl-app -p rl-tui -p rl-core 2>&1`

Expected: fmt OK, check OK.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: note GUI TUI parity for cache, soft-refuse, detail"
```

---

## Spec coverage checklist

| Spec requirement | Task |
|------------------|------|
| SharedManager in backend | 1 |
| Exclusive soft-refuse + Busy | 1 + 2 |
| try_read ticks | 1 |
| ClusterCache load/save | 2 |
| On-demand detail + Esc keeps selection | 3 |
| ScrollArea horizontal | 3 |
| README | 4 |
| No egui-owned manager | all |
| SetActiveKind not exclusive | 1 |
