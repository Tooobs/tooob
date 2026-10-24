#!/bin/sh
# Package the Bootstrap Bundle for a Release (#59): build it, check it against its manifest, then
# write target/bootstrap/tooob-bootstrap.tar.xz (a tooob-bootstrap/ folder) and its .sha256. dist
# runs this on its Ubuntu runner, which has no setup step for it, so it installs what's missing:
# the wasm target (from rust-toolchain.toml), wasm-bindgen-cli, and a clang that targets wasm.
set -eu
cd "$(dirname "$0")"

bindgen=$(sed -n 's/^wasm-bindgen = "=\(.*\)"$/\1/p' web/Cargo.toml)
if [ "$(wasm-bindgen --version 2>/dev/null)" != "wasm-bindgen $bindgen" ]; then
  cargo install --quiet --locked wasm-bindgen-cli --version "$bindgen"
fi
if [ -z "${LLVM:-}" ] && [ "$(uname)" = Linux ]; then
  # Ubuntu ships versioned LLVMs (/usr/lib/llvm-14/bin); take the newest with both tools.
  for dir in $(ls -d /usr/lib/llvm-*/bin 2>/dev/null | sort -t- -k2 -n -r); do
    if [ -x "$dir/clang" ] && [ -x "$dir/llvm-ar" ]; then LLVM=$dir && break; fi
  done
  export LLVM
fi

./build.sh
node manifest.mjs --check www

out=../target/bootstrap
rm -rf "$out"
mkdir -p "$out/tooob-bootstrap"
cp -R www/. "$out/tooob-bootstrap/"
find "$out/tooob-bootstrap" -name '.*' -type f -delete # .DS_Store and the like: not the Bundle's
export COPYFILE_DISABLE=1 # no macOS ._ metadata files in the archive
(cd "$out" && tar -cJf tooob-bootstrap.tar.xz tooob-bootstrap && shasum -a 256 -b tooob-bootstrap.tar.xz >tooob-bootstrap.tar.xz.sha256)
echo "package.sh: $(cd "$out" && cat tooob-bootstrap.tar.xz.sha256)"
