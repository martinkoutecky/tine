# 0023. SIGKILL WebKitGTK's helper processes at quit (Linux)

- **Status:** Accepted
- **Date:** 2026-07-07

## Context

On Linux, closing Tine leaves a `WebKitWebProcess` coredump (SIGABRT) behind
([#28](https://github.com/martinkoutecky/tine/issues/28)). The app itself closes
cleanly — the crash is in WebKitGTK's *renderer subprocess*, and it fires during
that process's normal exit teardown:

```
exit → __run_exit_handlers → _int_free_* → malloc_printerr → abort
```

The loaded modules (`libGLESv2`, `libEGL_mesa`, `libgbm`, `dri_gbm.so`,
`libgallium`, `libnvidia-egl-*`) place the double-free in the GL/EGL/GBM driver's
static destructors. WebKit's auxiliary processes terminate by *returning from
`main()`*, so they run the full C runtime teardown (`atexit`/`__cxa_atexit`)
rather than `_exit()` — which is what gives those buggy driver destructors a
chance to run. This is a long-standing class of Mesa/driver-teardown bug (Mesa has
fixed instances of it before; native WebKitGTK apps like Epiphany/Evolution hit the
same signature) and it reproduces on plain Intel iGPUs, not just hybrid-NVIDIA.

Alternatives considered and rejected:

- **`WEBKIT_DISABLE_DMABUF_RENDERER=1` / `WEBKIT_DISABLE_COMPOSITING_MODE=1`**
  (Tine's existing `TINE_GPU=0` opt-out). These *do* stop the crash — but only
  because they take the GL/GBM path out of the web process, i.e. they disable the
  GPU compositing that is the entire point of Tine. Unacceptable as a default.
- **`RunEvent::Exit` + `std::process::exit(0)`** (the common Tauri lore). Verified
  against wry/WebKit source that this does *not* stop the subprocess dump: it
  hard-exits the *main* process, but the web process is a separate PID that, on
  losing its IPC socket, still shuts *itself* down gracefully and still runs the
  same exit handlers. WebKit's sandbox (which would `die-with-parent` SIGKILL the
  child) is off for wry, so there's no parent-death shortcut.
- **Suppressing the coredump only** (`RLIMIT_CORE=0`): hides the dump, doesn't
  prevent the abort, and is unreliable against systemd's piped `core_pattern`.
- **Fixing it upstream in Mesa**: correct, but not shippable by the app; tracked as
  follow-up.

## Decision

At quit, on Linux, Tine **SIGKILLs its own WebKitGTK helper subprocesses
(`WebKit*`, matched by `comm` prefix and `ppid == our pid`) before they are asked
to shut down gracefully**, then exits through Tauri's normal path.

Mechanics: the JS close handler's final step calls a `tine_quit` backend command
(instead of `window.destroy()`). `tine_quit` runs `platform::kill_webkit_children()`
— a `/proc` walk that SIGKILLs each `WebKit*` process whose parent is us — then
`app.exit(0)`. SIGKILL is uncatchable and runs no exit handlers, so the driver
teardown never executes and no core is produced. Killing *before* `destroy()` (not
in the later `WindowEvent::Destroyed` handler) is deliberate: it preempts the
graceful shutdown rather than racing it.

**Data-safety invariant (load-bearing):** `tine_quit` may only be called *after*
pending edits are persisted. The JS close handler already awaits
`flushAll()`/`flushSession()` (with a confirm-on-unsaved guard) before calling it,
and the web process owns no persistent app data (Tine's graph is written by the
Rust main process, which exits normally). Any future change to the close path must
preserve "flush, then quit."

## Consequences

- The exit coredump is *prevented*, not hidden, and GPU compositing stays on for
  the whole session. `TINE_GPU=0` remains a pure opt-out, no longer needed for #28.
- We now depend on the `flush-before-quit` ordering in `App.tsx`; it is called out
  in code comments and here. A regression that quits before flushing would lose the
  last edits — but that was already true of the previous `destroy()` path.
- `libc` is a new (Linux-only) direct dependency, used solely for `kill(2)`.
- The `/proc/<pid>/stat` parse handles `comm` containing spaces and `)` and the
  kernel's 15-byte `comm` truncation ("WebKitWebProcess" → "WebKitWebProces"); it is
  unit-tested (`platform::tests`). The PPID filter ensures we never touch another
  application's WebKit processes.
- macOS/Windows are unaffected (`tine_quit` there is just `app.exit(0)`).
- The underlying double-free should still be filed upstream at Mesa (and, on hybrid
  boxes, the NVIDIA-EGL/libglvnd teardown ordering) — that's where it is truly
  fixed. Tracked in `docs/BACKLOG.md`.
