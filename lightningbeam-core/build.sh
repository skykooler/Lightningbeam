#!/bin/bash
cd core
wasm-pack build --target web --out-dir ../src/pkg --features wasm
