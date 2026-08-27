#!/bin/sh
# Build and run the C smoke test against the static library.
#
# Static rather than shared so there is no install step and no LD_LIBRARY_PATH
# to get wrong: the point is to prove the header and the library agree, not to
# exercise the loader.
set -e
root=$(cd "$(dirname "$0")/../.." && pwd)
out=${TMPDIR:-/tmp}/ade-bridge-smoke

cargo build --release -p ade-bridge --manifest-path "$root/Cargo.toml"
cc -Wall -Wextra -Werror -o "$out" "$root/bridge/tests/smoke.c" \
   "$root/target/release/libade.a" -lpthread -ldl -lm
exec "$out" "$@"
