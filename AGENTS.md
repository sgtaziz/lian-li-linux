# AGENTS.md

This file defines how coding agents should work in this repository. It applies to the entire repository unless a more specific `AGENTS.md` exists below the file being changed.

## Project priorities

Lian Li Linux is a long-running hardware-control daemon with a Tauri desktop client. Correct device behavior, low background resource use, safe shutdown, and compatibility with existing user configuration take priority over convenience or large refactors.

Treat regressions that can wedge hardware, leave fans or pumps uncontrolled, blank displays, saturate USB, consume a CPU core, grow memory without bound, or prevent clean shutdown as release blockers.

If a requested change overlooks a serious correctness, safety, compatibility, or resource issue, point it out clearly. Give concrete evidence and explain the user-visible consequence. Do not silently implement a narrowly requested change when a closely related glaring issue would make the result unsafe or incomplete.

## Non-negotiable rules

### Comments

Prefer clear names, small functions, and straightforward control flow over comments.

Do not add comments unless they are necessary to explain code that remains complex or difficult to understand after reasonable simplification. When a comment is necessary:

- Keep it concise.
- Explain why the code exists or which invariant it preserves.
- Do not narrate what the next line does.
- Use plain text without decorative symbols, diagrams, banners, issue markers, or changelog-style history.
- Place it next to the code it explains so it moves with that code.

When editing a region, remove stale, redundant, overly long, or detached comments you encounter. Preserve comments that document protocol facts, unsafe invariants, hardware timing requirements, or behavior that cannot be made clear through naming alone. Do not perform an unrelated repository-wide comment cleanup unless asked.

Public API documentation should follow the same standard. Add it only when callers need information that the type or function signature cannot express.

### Git and GitHub

Never commit code unless the user explicitly asks for a commit. Never push code unless the user explicitly asks for a push. Never create or update a pull request unless the user explicitly asks for that pull request action.

Do not treat a request to implement, fix, test, review, or prepare changes as permission to commit, push, or create a pull request.

When a commit is explicitly requested, use Conventional Commits with a concise scope naming the affected device or feature:

```text
fix(h2): restore brightness after daemon restart
fix(wireless): bound failed discovery retries
feat(rgb): add per-zone direction control
fix(tl-lcd): validate frame payload size
```

Use an imperative, compact subject. Keep it specific and omit generic subjects such as `fix bug`, `updates`, or `misc changes`. Common scopes include `h2`, `hydroshift`, `tl`, `tl-lcd`, `wireless`, `rgb`, `lcd`, `media`, `daemon`, `gui`, `ipc`, `evdi`, `packaging`, and `docs`.

### Tests

Do not add tests merely to increase coverage or mirror implementation details.

Tests must validate meaningful input and output behavior, including protocol bytes, parsing, serialization, state transitions, boundary conditions, error results, or regressions observable by a caller. A regression test should fail for the reported bug and pass after the fix.

Hardware-independent logic should be separated from device I/O when that produces a useful deterministic test. Do not build elaborate mocks for behavior whose only meaningful validation requires physical hardware. Record required manual hardware checks in the handoff instead.

## Repository map

The active Rust workspace members are:

| Path | Responsibility |
| --- | --- |
| `crates/lianli-shared` | Configuration schema, IPC types, device identifiers, sensor types, RGB and fan models, screen capabilities, and shared templates |
| `crates/lianli-transport` | HID and USB transport wrappers, timeouts, retry behavior, and transport errors |
| `crates/lianli-devices` | Device detection, protocol implementations, device traits, wireless control, and per-family drivers |
| `crates/lianli-media` | Image, GIF, video, H.264, sensor, and custom-template rendering |
| `crates/lianli-evdi` | Safe wrapper around EVDI for desktop-mode displays |
| `crates/lianli-daemon` | Service lifecycle, polling, controllers, media streaming, IPC server, persistence, and shutdown |
| `crates/lianli-gui/src-tauri` | Tauri backend for the desktop client |

The active frontend is in `crates/lianli-gui/src` and uses Vue 3, TypeScript, Pinia, and Naive UI.

`crates/lianli-gui-old` is excluded from the workspace. Do not update it unless the task explicitly targets the old Slint client.

Treat `vendor/`, `target/`, `tmp/`, generated frontend output, and packaged build trees as generated or third-party content. Do not edit them unless the task specifically requires it. Make source changes in the owning crate.

## Architecture boundaries

Keep shared data contracts in `lianli-shared`. Device-independent configuration and IPC types should not depend on daemon, GUI, transport, or hardware implementations.

Keep raw HID and USB mechanics in `lianli-transport`. Keep device opcodes, packet layout, initialization sequences, and family-specific behavior in `lianli-devices`.

Keep orchestration in `lianli-daemon`. The daemon may coordinate devices, media, controllers, and IPC, but it should not duplicate protocol construction owned by a device driver.

Keep media decoding and rendering in `lianli-media`. Avoid coupling renderers to daemon state or USB handles.

The GUI communicates with the daemon through IPC. Do not bypass IPC by teaching the GUI to access hardware or daemon configuration files directly.

When adding a device or capability, check all relevant sources of truth:

- `crates/lianli-shared/src/device_id.rs` for IDs, families, and capabilities
- `crates/lianli-shared/src/screen.rs` for screen behavior
- `crates/lianli-devices/src/registry.rs` and `detect/` for opening and enumeration
- `crates/lianli-devices/src/traits.rs` for cross-family interfaces
- `crates/lianli-daemon/src/service/` for lifecycle integration
- `crates/lianli-daemon/src/service/sync.rs` for GUI-visible device state
- `README.md` and packaging rules when user-facing support or installed files change

## Long-running daemon requirements

Resource usage is a core correctness requirement. This daemon commonly runs for an entire gaming session, where background CPU, memory, process, and USB activity directly affect the user.

For every recurring task, worker, renderer, or polling loop:

- Avoid busy loops. Every idle or retry path must block, wait, or use an intentional bounded cadence.
- Avoid sending unchanged state or frames. Preserve deduplication and event-driven wakeups.
- Bound queues and define overload behavior. Do not allow producers to create unbounded memory growth.
- Bound retries and add backoff for unavailable or unresponsive hardware.
- Avoid repeated device enumeration, file reads, media decoding, allocation, and process creation in hot paths.
- Cache immutable or expensive results when invalidation is clear.
- Reuse buffers in frame and packet loops where practical.
- Clamp user-controlled polling intervals and frame rates to safe device limits.
- Do not hold a mutex during sleep, external process work, blocking IPC, long media work, or avoidable USB waits.
- Ensure worker threads and child processes have a stop path and are joined or intentionally detached with a documented reason.
- Consider steady-state CPU, wakeup frequency, memory, USB bandwidth, and log volume.

Prefer event-driven work over frequent polling. When polling is required by hardware, preserve established timing and explain any cadence change in the handoff.

Do not add informational logs inside frame loops or high-frequency polling paths. Use `tracing` at a level appropriate to frequency. Repeated failures should be rate-limited, deduplicated, or logged on state transitions.

Changes to media rendering must account for both JPEG and H.264 paths, static and dynamic sources, frame pacing, source frame-rate limits, orientation, payload limits, and renderer shutdown.

## Hardware and protocol safety

USB and HID behavior is stateful. A daemon restart does not guarantee that a device returns to its power-on state. Assume brightness, mode, stream state, binding state, frame rate, and firmware state can persist across process restarts.

For initialization, shutdown, suspend, resume, hotplug, recovery, and mode-switch changes:

- Trace the complete lifecycle, including a fresh process attaching to hardware left by the previous process.
- Keep shutdown ordered and bounded. Stop producers before releasing consumers or transports.
- Do not terminate while a transfer or protocol transaction is intentionally left incomplete.
- Ensure commands needed during teardown are delivered before closing their worker or transport.
- Restore persistent hardware state on startup or give configuration fields explicit backward-compatible defaults.
- Treat device disappearance and timeouts as expected runtime conditions, not panics.
- Avoid global flags that accidentally re-enable unrelated I/O during shutdown.
- Preserve serialization requirements for devices that cannot handle concurrent commands.
- Do not change opcodes, packet sizes, byte order, encryption, timing, endpoints, or response handling without protocol evidence.

Never infer that two device families accept the same command because their visible behavior is similar. Follow the driver dispatch and verify each affected backend.

Do not run the daemon, device utilities, USB resets, binding commands, firmware operations, or direct hardware tests against attached devices unless the user asked for hardware interaction. Builds and unit tests are safe by default.

When hardware validation is needed, state the exact devices, connection mode, media mode, and lifecycle sequence to test. Include cold start, daemon restart, clean shutdown, and failure recovery when relevant.

## Configuration and IPC compatibility

User configuration survives upgrades. New fields must deserialize old files safely. Decide explicitly whether a missing field means disabled, device-managed, inherited, or a concrete default.

Do not show a default in the GUI while leaving the daemon to interpret the missing value differently. Defaults must agree across:

- Rust configuration types and migration logic
- daemon application behavior
- IPC payloads and responses
- TypeScript types and frontend controls

Prefer a single Rust helper or deserialization default for effective values. Preserve unknown or optional behavior when `None` has a distinct meaning.

IPC changes must remain compatible where practical. Add `serde` defaults for new request or response fields when older peers may omit them. Keep request handling bounded and avoid waiting on the main service loop for work that can exceed the IPC timeout.

Configuration writes must use the existing persistence path and notification flow. Do not add direct frontend writes to `config.json`.

## Concurrency and lifecycle

Before changing shared state or worker ownership, identify:

- Which thread owns the value
- Which locks guard it
- Whether a USB call or channel send can block
- Which stop flag ends the worker
- Who joins the worker
- What happens when a channel is full or disconnected
- What happens during shutdown, config reload, suspend, resume, and device removal

Use bounded lock attempts in the service loop when another worker may hold a device across a long operation. Avoid lock-order inversions between service state, device handles, controller state, and IPC state.

Channel success must mean the operation was accepted at the level promised to the caller. Do not report success after silently dropping a control command. For commands whose delivery matters, use acknowledgement or an ordered shutdown path.

## Error handling and logging

Use `anyhow::Result` with context at application boundaries. Use structured error types where callers need to distinguish failure categories.

Include enough context to identify the device, operation, and relevant target without logging high-frequency noise or sensitive local data. Use `tracing` rather than `println` in daemon and library code. CLI commands may print user-facing results.

Do not swallow an error unless failure is intentionally best-effort. When ignoring a result, make the fallback or invariant clear through structure or a concise necessary comment.

## Workflow

Before editing:

1. Read the nearest applicable `AGENTS.md` completely.
2. Inspect the working tree and preserve unrelated user changes.
3. Trace the relevant path across config, IPC, daemon orchestration, device drivers, and GUI where applicable.
4. Check recent history when behavior or intent is unclear.

While editing:

1. Keep the change scoped to the requested behavior.
2. Prefer the smallest design that fixes the full lifecycle.
3. Maintain backward compatibility unless the task explicitly requires a breaking change.
4. Avoid opportunistic formatting or unrelated cleanup.
5. Update documentation and packaging only when behavior, dependencies, installed assets, commands, or supported devices change.

After editing:

1. Review the diff for unintended changes and stale comments.
2. Run formatting, Clippy, and focused checks for the touched code.
3. Run broader checks only when the change crosses crates or shared contracts.
4. Report what changed, why, what was tested, and any hardware validation still required.
5. Do not commit, push, or create a pull request unless explicitly requested.

## Validation commands

Choose checks proportional to the change. Start focused and expand when shared contracts or multiple crates are involved.

```bash
cargo fmt --all --check
cargo clippy -p <affected-crate> --all-targets -- -D warnings
cargo check -p lianli-shared
cargo check -p lianli-transport
cargo check -p lianli-devices
cargo check -p lianli-media
cargo check -p lianli-daemon
cargo check -p lianli-gui
cargo test -p <affected-crate>
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Building the Tauri GUI through Cargo runs the required npm frontend build automatically. Use `cargo check -p lianli-gui` for a focused GUI check and the workspace commands above for cross-project validation. Do not run a separate npm build unless the task specifically requires frontend-only diagnosis.

The full workspace and GUI require system libraries for libusb, FFmpeg, EVDI, WebKitGTK, and related graphics dependencies. If a broad build fails because one of those libraries is unavailable, run the strongest focused checks possible and report the missing dependency precisely.

Tests that depend on socket buffer sizes, timing, USB access, attached hardware, systemd, EVDI, or a compositor can be environment-sensitive. Investigate failures before classifying them as regressions. Never dismiss a failure solely because it is intermittent.

## Review standard

When reviewing code, prioritize findings in this order:

1. Hardware safety and persistent device state
2. Fan and pump control correctness
3. Shutdown, restart, suspend, resume, and hotplug behavior
4. Unbounded CPU, memory, thread, process, log, or USB usage
5. Deadlocks, races, dropped commands, and misleading success responses
6. Configuration and IPC compatibility
7. Media correctness and frame pacing
8. Packaging and user-visible behavior

Each finding should identify the trigger, the resulting behavior, and the relevant code location. Distinguish verified defects from risks that still need hardware confirmation. Do not bury a serious issue among style notes.
