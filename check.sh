#!/bin/sh
# The fast checks, run before every push (.githooks/pre-push) and, once it exists, by release.sh
# (#59): formatting, clippy, the Host's tests, the Bootstrap's build and its Bundle's manifest
# tests. Stops at the first that fails and says which. The browser suite isn't here: it runs at the
# end of each issue and before a Release.
set -u
cd "$(dirname "$0")" || exit 1

# step NAME DIR COMMAND...: run COMMAND in DIR, or stop, saying NAME failed.
step() {
  name=$1
  dir=$2
  shift 2
  echo "check.sh: $name"
  if ! (cd "$dir" && "$@"); then
    echo "check.sh: $name failed" >&2
    exit 1
  fi
}

step "cargo fmt (Host)" host cargo fmt --check
step "cargo fmt (Bootstrap)" bootstrap cargo fmt --check
step "cargo clippy (Host)" host cargo clippy --all-targets --quiet -- -D warnings
step "cargo test (Host)" host cargo test --quiet
step "bootstrap/build.sh" bootstrap ./build.sh
step "manifest tests" bootstrap/check node --test manifest.test.mjs
echo "check.sh: all passed"
