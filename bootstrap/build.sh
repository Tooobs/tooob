#!/bin/sh
# Build the browser's WASM bundle into www/__tooob/pkg, then the Bootstrap Bundle's manifest
# (www/__tooob/version.json). Serve www/ as the Bootstrap.
set -e
cd "$(dirname "$0")"
LLVM=${LLVM:-/opt/homebrew/opt/llvm/bin}  # a clang that targets wasm (Apple's can't; ring needs one)
CC_wasm32_unknown_unknown=$LLVM/clang AR_wasm32_unknown_unknown=$LLVM/llvm-ar \
  cargo build -p web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir www/__tooob/pkg target/wasm32-unknown-unknown/release/web.wasm
node manifest.mjs www
