# Build

> **No longer a stub.** The Rust workspace builds, tests, and lints as of 2026-08-21, so the commands below are real rather than intended. The Qt6 GUI does not exist yet (Phase 5), so its half remains prospective. Toolchain settled by **D-001** and **D-002** — see [DECISIONS.md](DECISIONS.md).
>
> **The toolchain is pinned.** `rust-toolchain.toml` names an exact version, not `stable`, and rustup installs it automatically. Verified on Linux x86-64; the workspace pins `edition = "2024"` and a `rust-version` floor of 1.85.
>
> The pin exists because CI denies all warnings and clippy gains lints with every Rust release. On a floating channel the runner installs whatever is current while a developer may be several releases behind, so a build breaks with no code change — which happened on 2026-08-24, when CI at 1.98.0 rejected code that passed locally at 1.94.1. Bumping the pin is a deliberate commit, and a non-blocking `toolchain-drift` CI job lints against latest stable so the bump is scheduled rather than sprung.

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

The workspace builds and tests today, though the engine is still a scaffold:

```bash
cargo build --workspace          # all crates + the `ade` binary
cargo test  --workspace          # unit tests
cargo run   -p ade-cli           # prints version; no commands yet
```

The full check set, which CI runs on every push:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
python3 tools/check-layering.py   # D-003: no cross-layer dependencies
```

Still to come:

```bash
cargo fuzz run <target>          # F-001 acceptance, with the first parser
cmake --build build              # Qt6 GUI (Phase 5)
```

## Lints as invariants

The workspace lint set in the root `Cargo.toml` is load-bearing, not
cosmetic. Relaxing any of it is a decision rather than a convenience:

- `unsafe_code = "forbid"` — nothing in the core needs it. The C-ABI bridge
  will, and gets a scoped exemption when it exists (D-001, D-006).
- `unwrap_used`, `expect_used`, `panic`, `indexing_slicing`,
  `arithmetic_side_effects` — all denied. Each is a route from hostile input
  to a crash, which F-001 forbids.
- `clippy.toml` disallows `u32::from_be_bytes` and its siblings everywhere
  except `ade-endian`, so C-001's "one byte-order module" is enforced by the
  build rather than by review.

`integer_division` is deliberately **not** enabled: its advice is to prefer
floats, which would be a defect in a disk tool, and the panic it guards
against is already covered by `arithmetic_side_effects`.

## Cross-compilation

Not applicable yet (Linux-first).

## Troubleshooting

None recorded yet. Known build failures and fixes will be captured here as they occur.
