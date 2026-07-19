# GUI follow-TUI behaviors — Design

**Date:** 2026-07-19  
**Status:** Approved (approach + option B UX); awaiting spec file review before implementation  
**Scope:** Port TUI concurrency / cache / soft-refuse / on-demand detail UX into `rl-app` (GUI)

## Goal

Make the GUI match the TUI’s *user-facing behaviors* for cluster I/O and detail viewing, without replacing the egui ↔ backend-thread architecture.

## Non-goals

- Mouse CSI enable/restore (TUI/SSH-only)
- Replacing the mpsc backend with TUI’s single-process async app loop
- Line-by-line port of TUI `PendingOp` onto the egui thread
- Redesigning cluster tabs, sidebar, or log panels beyond busy/detail changes

## Current state

| Layer | Today |
|-------|--------|
| GUI UI | Sync egui; never owns `ClusterManager` |
| Backend | Exclusive `Option<ClusterManager>`; each command fully awaited in `select!` (stalls 500 ms ticks) |
| Cache | `rl-core::ClusterCache` unused by GUI |
| Exclusive ops | UI can queue many `SwitchContext` / `SetNamespace`; no soft-refuse |
| Detail | Side panel auto-opens when a table row is selected; Esc clears selection |

## Target behavior (parity with TUI)

1. **Shared manager in backend** — `Option<Arc<RwLock<ClusterManager>>>`; exclusive work spawned; ticks/`Fetch*` use `try_read` or short `read`.
2. **Soft-refuse** — While an exclusive op is open (connect / switch context / set namespace / reconnect / refresh watch), refuse starting another; show status `Busy — wait for current request…`; navigation, selection, logs remain usable.
3. **Cluster cache** — Load/save `~/.cache/rusticlens/cluster-cache.json` (same as TUI); seed context/namespace lists; update on successful `Connected`.
4. **On-demand detail** — Panel hidden until user opens Describe (`d` / explicit open); Esc closes panel **without** clearing row selection; table uses full width when closed.
5. **Detail scroll** — Rely on egui `ScrollArea` horizontal+vertical (no TUI pan keys required as primary UX).

## Architecture

```
egui thread                          backend thread (Tokio, 1 worker)
─────────────                        ────────────────────────────────
RusticlensApp                        SharedManager = Arc<RwLock<ClusterManager>>
  ClusterCache (memory)                exclusive_in_flight: bool / JoinHandle
  exclusive_busy: bool                 spawn switch/connect/refresh
  detail_panel_visible: bool           tick: try_read → snapshots / log poll
  soft_refuse() → status               Fetch*: read lock, emit events
       │  BackendCommand                    │
       └──────── mpsc ──────────────────────┘
       ┌──────── mpsc events ───────────────┘
  drain: Connected / Busy / Error / Snapshot / …
```

### Backend changes (`crates/rl-app/src/backend.rs`)

- Introduce `type SharedManager = Arc<RwLock<ClusterManager>>` (local to backend, or shared type alias if useful).
- `run_backend_loop`: hold `Option<SharedManager>` plus `exclusive_op: Option<JoinHandle<()>>` (or equivalent flag + channel).
- Exclusive commands (`ConnectDefault`, `SwitchContext`, `SetNamespace`, `Reconnect`, `RefreshWatch`):
  - If exclusive already open → emit `BackendEvent::Busy` (new) and return.
  - Else emit `Connecting` (where applicable), spawn task that write-locks, does work, emits `Connected` / `Error`, clears exclusive flag.
- Periodic ticks and read commands (`FetchYaml`, logs, etc.): `manager.try_read()` / `.read().await`; skip tick iteration if write-locked.
- `SetActiveKind` / `SetCrdTarget` / `RefreshList`: keep semantics; prefer read or short write as today via manager APIs; do not block exclusive soft-refuse unless they take exclusive write for a long time (match TUI: kind switch may be inline if fast).

### UI changes (`crates/rl-app/src/app.rs` + detail panel)

- On startup: `load_cluster_cache()` → populate `contexts` / `namespaces` when possible; show cached lists before first `Connected`.
- On `Connected`: update lists + `save_cluster_cache` (spawn or sync write off hot path).
- Track `exclusive_busy` from `Connecting` / `Busy` / `Connected` / `Error`.
- Before sending switch/ns/refresh: if `exclusive_busy`, set busy status and do not send.
- Add `detail_panel_visible: bool` (default `false`).
  - Show right `SidePanel` only when `detail_panel_visible && connected && !overview`.
  - Open: `d` / Describe actions / context-menu Describe — keep selection, fetch yaml.
  - Close: Esc or header close — clear detail text/search, set `detail_panel_visible = false`, **keep** `table.selected`.
  - Row selection alone must **not** auto-open the panel (change from today).

### Events

Add:

```rust
BackendEvent::Busy, // optional message payload if useful
```

Reuse `Connecting` / `Connected` / `Error` for lifecycle.

## Approaches considered

1. **Backend SharedManager + UI soft-refuse + cache + detail toggle** — chosen.
2. Full GUI rewrite onto TUI in-process async — rejected (too large).
3. Cache + soft-refuse only — rejected (misses concurrency win).

## Testing

- `cargo check -p rl-app`
- Manual: switch context while logs open — UI stays responsive; second switch soft-refuses; namespaces appear from cache after first visit.
- Manual: select row → no detail panel; press `d` → panel; Esc → panel gone, row still selected.
- Confirm cache file shared with TUI after GUI connect.

## Docs

- Brief README / DEVELOPMENT note that GUI uses the same cluster cache path and soft-refuse semantics as TUI (only if README already documents TUI cache).

## Success criteria

- GUI no longer blocks backend ticks for the full duration of context/namespace switches.
- Soft-refuse prevents stacked exclusive ops with clear status text.
- Instant context/ns lists from disk cache when available.
- Detail panel is opt-in like TUI, Esc does not deselect the row.
