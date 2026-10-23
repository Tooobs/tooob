#!/bin/sh
# Build the host binary and the browser WASM bundle into www/__tooob/pkg.
set -e
cd "$(dirname "$0")"
LLVM=/opt/homebrew/opt/llvm/bin  # Apple clang can't target wasm; ring needs a real one
CC_wasm32_unknown_unknown=$LLVM/clang AR_wasm32_unknown_unknown=$LLVM/llvm-ar \
  cargo build -p web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir www/__tooob/pkg target/wasm32-unknown-unknown/release/web.wasm
cargo build -p host --release
ls -l www/__tooob/pkg/web_bg.wasm | awk '{printf "wasm: %.1f MB (gzip: ", $5/1048576}'
gzip -9c www/__tooob/pkg/web_bg.wasm | wc -c | awk '{printf "%.1f MB)\n", $1/1048576}'
