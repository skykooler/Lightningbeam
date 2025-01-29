#!/bin/bash
echo "Building native..."
cargo build
echo
echo "Building wasm..."
wasm-pack build --target web --out-dir ../src/pkg --features wasm
