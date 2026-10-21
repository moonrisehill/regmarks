#!/bin/sh
set -eu
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown
out=target/wasm32-unknown-unknown/release/reg.wasm
cp "$out" ../reg.wasm
echo "built $out ($(wc -c < "$out") bytes) -> reg.wasm"
