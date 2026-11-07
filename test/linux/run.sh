#!/bin/sh
# The Host's cargo tests on Linux (musl, arm64 on a Mac), in a podman container: release.sh runs
# it before it tags (#108), so a Linux failure stops the Release instead of spending its tag. CI's
# linux-tests workflow runs them on every push to main too. Arguments go to cargo test,
#   test/linux/run.sh                   # all of them
#   test/linux/run.sh --test terminal   # one file
# Runs as a user, not root: root ignores the read-only files some tests set up.
set -eu
cd "$(dirname "$0")/../.."
podman image exists localhost/tooob-linux-test ||
  podman build -q -t localhost/tooob-linux-test test/linux >/dev/null
exec podman run --rm --user tester -v "$PWD:/src:ro" \
  -v tooob-linux-cargo:/usr/local/cargo/registry -v tooob-linux-target:/target \
  -e CARGO_TARGET_DIR=/target -e RUSTUP_TOOLCHAIN=1.97.1 -w /src/host \
  localhost/tooob-linux-test cargo test --locked "$@"
