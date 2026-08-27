# Build

> **No longer a stub.** Every command below is real. The Rust workspace has built, tested and linted since 2026-08-21; the C-ABI bridge and the Qt6 GUI joined it on 2026-08-27. Toolchain settled by **D-001** and **D-002** — see [DECISIONS.md](DECISIONS.md).
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

## The GUI

Qt6 6.4 or later, and CMake 3.19 or later. CMake invokes `cargo` for the
bridge itself, so this is the only command needed:

```bash
cmake -S gui -B gui/build -DCMAKE_BUILD_TYPE=Release
cmake --build gui/build
./gui/build/ade-gui [image...]
```

Its tests need no X server — `Qt6::Test` under the offscreen platform — and
generate their own image with `mkfixture`, since D-010 commits no binaries:

```bash
ctest --test-dir gui/build --output-on-failure
```

The C ABI has its own check, a real C program compiled `-Wall -Wextra -Werror`
against the hand-written header and run under the sanitizers. It is the only
thing that can catch `bridge/include/ade.h` disagreeing with the library:

```bash
bridge/tests/run.sh
```

Still to come:

```bash
cargo fuzz run <target>          # F-001 acceptance, with the first parser
```

## Lints as invariants

The workspace lint set in the root `Cargo.toml` is load-bearing, not
cosmetic. Relaxing any of it is a decision rather than a convenience:

- `unsafe_code = "forbid"` — nothing in the core needs it. The C-ABI bridge
  does, and its exemption is scoped rather than granted: `forbid` cannot be
  lifted by an inner `allow`, so `ade-bridge` opts out of the workspace lint
  set and restates every other lint verbatim (D-001, D-006).
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
