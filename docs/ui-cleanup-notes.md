# Desktop-mobile UI cleanup notes

These are the states that currently feel useless, ambiguous, or too close to a
blank screen. They are intentionally separated from valid product states such
as an empty intent history or a masked wallet value.

## Fix next (P0)

| Screen | Current behavior | Problem | Suggested replacement |
|---|---|---|---|
| `new` | `if (!options) return null` | The whole screen disappears while IPC is loading or fails. | Render `LOADING FORM OPTIONS…`; on failure render `FORM UNAVAILABLE` with `RETRY`. |
| global mobile shell | No mobile `ErrorBoundary` | A Tauri-only API failure can leave a blank WebView. | Add a recovery panel with `RELOAD APP` and the non-sensitive error category. |

## Fix next (P1)

| Screen | Current behavior | Problem | Suggested replacement |
|---|---|---|---|
| `detail` | `LOADING…` in a plain panel | No visual progress and no retry action. | Add a compact skeleton/status line; add `RETRY` after a failed `get_intent`. |
| `settled` | `LOADING PROOF…` in a plain panel | User cannot tell whether settlement is still running or the id is invalid. | Separate `FETCHING PROOF…` from `INTENT NOT FOUND` and add back action. |
| `nodes` | `SCANNING` has no timeout | Can appear stuck indefinitely when discovery is unavailable. | Add elapsed scan state and `RETRY DISCOVERY`. |
| `home` / `profile` | `—` for node, uptime, reputation, member date | The value is technically honest but not actionable. | Use `NOT READY`, `NO SIGNAL`, or `NOT RECORDED` depending on the field. |

## Keep (valid product states)

- `intents` empty states: useful because they explain what the user can do next.
- `vault` masked total: required privacy behavior, not a loading placeholder.
- `connecting` handshake log: useful because it streams real bootstrap progress.
- `WAITING` settlement status: required for offline relay and must not look settled.
- `DISCOVERY UNAVAILABLE` and `NO NODES NEARBY`: distinct, actionable states.

## Copy consistency

- Use an ellipsis only for an active operation: `LOADING…`, `SCANNING…`,
  `FETCHING PROOF…`.
- Use `NOT READY`/`NOT RECORDED` for missing data, not a bare em dash.
- Every terminal error state needs one recovery action or a clear back action.
- Do not add fabricated sample rows to make an empty state look populated.

## Visual comparison notes

The supplied demo uses dense instrumentation panels, an Oracle hero/emblem,
and a five-destination bottom bar. Those are now present in the desktop-mobile
build. The remaining mismatch is primarily behavior of loading/error states,
not missing brand assets.
