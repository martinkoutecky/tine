# UI state lifetimes

Every UI value belongs to one lifetime. A change that persists or resets a
value must name its owner and lifetime before wiring storage. This prevents a
runtime handle from accidentally becoming durable state and prevents a graph
transition from saving state under the wrong graph.

## Lifetimes

- `device-preference`: owned by the installation, independent of any graph;
  reset by an explicit preference change or application-data reset.
- `graph-configuration`: derived from graph files/configuration and reloaded on
  graph bind or configuration refresh; it is not session state.
- `graph-session`: user view state belonging to one graph or named workspace;
  reset or restored at graph/workspace transitions and persisted through the
  audited session boundary.
- `transient-runtime`: generations, native handles, in-flight work, focus and
  navigation intents; reminted or cleared at the owning runtime boundary and
  never serialized.

## Route-owned graph-session state

Pane snapshots and their route histories are the durable authority for visible
content. `src/session.ts` parses and serializes each route explicitly instead of
spreading runtime objects into storage. A PDF is therefore an ordinary `pdf`
route in a tab, not a second top-level pane state:

| State | Owner | Lifetime | Reset trigger | Persisted representation |
|---|---|---|---|---|
| PDF tab | pane router and session serializer | `graph-session` | tab close, graph switch, or workspace switch | `kind`, stable `viewId`, `filename`, `label`, optional page and scale |

The former top-level `pdfTarget` is accepted only as legacy input. Desktop
restore migrates it into a companion pane in the layout tree; mobile restore
appends it to the active pane history. New sessions never write `pdfTarget` or
a global PDF-pane width.

PDF graph ownership and its generation, pending page/highlight navigation
intents, viewer/native handles, render tasks, and sidecar view state are
`transient-runtime`. They are scoped to the route's `viewId`, reminted or
cancelled at their owning boundary, and never serialized. On restore, Tine uses
the stable resource identity and mints ownership from the current graph bind.

The typed registry in `src/uiStateRegistry.ts` remains the authority for
standalone graph-session signals. It is intentionally empty now that PDF state
lives in pane routes. Adding another standalone signal requires an explicit row
and typed registry decision in the same change.
