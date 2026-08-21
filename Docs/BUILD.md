# Build

> **Stub.** Per the project-scaffold standard, BUILD.md is filled in when the first build succeeds. ADE has no code yet; this file records the *intended* environment so the eventual build recipe has a home. **D-001** (language/stack) and **D-002** (reimplement rather than wrap) were Accepted on 2026-08-21, so the toolchain below is now settled rather than conditional — see [DECISIONS.md](DECISIONS.md). Exact versions and commands land here when the first build succeeds.

## Supported platforms

- **Primary:** Linux (x86-64). Development target.
- **Later:** Windows, macOS. Not addressed in v1.

## Build dependencies

Settled by D-001 and D-002:

- **Rust** toolchain — core and CLI. Cargo workspace of `ade-*` crates, one per pipeline layer.
- **C-ABI bridge** — `cdylib` exposing the core to the GUI (D-001, the Pontus pattern).
- **Qt6** — GUI only, from Phase 5. Built via CMake against the bridge.
- **libz** — ADZ/HDZ gzip containers.
- **CAPS** — optional and licence-gated, IPF-read only (C-003).

Deliberately **not** build dependencies:

- **ADFlib** — under D-002 it is a black-box differential-test oracle, invoked as a separate binary by the test harness. It is never linked, is not required to build or run ADE, and its source is not read. Contributors need it only to run the differential suite.
- **xDMS** — Phase 3, and its role is still open (D-009). If that lands on a port, it becomes owned Rust rather than a dependency.

## Test dependencies

- **`cargo-fuzz`** / libFuzzer — the F-001 fuzz corpus is part of the Phase-1 acceptance bar, not an afterthought.
- **ADFlib binary** — differential oracle, as above. The suite must skip rather than fail when it is absent, so that a fresh clone still builds and tests.
- **Fixture corpus** — what may be committed is open under **D-010**; expect a fetch step for anything not freely distributable.

Exact versions and per-platform install commands: TBD until the first build.

## Build commands (placeholder)

```bash
# Not yet runnable — no code. Expected shape:
#   cargo build --release        # core crates + CLI + C-ABI bridge
#   cargo test                   # unit + integration
#   cargo fuzz run <target>      # F-001 acceptance
#   cmake --build build          # Qt6 GUI (Phase 5)
```

## Cross-compilation

Not applicable yet (Linux-first).

## Troubleshooting

None recorded yet. Known build failures and fixes will be captured here as they occur.
