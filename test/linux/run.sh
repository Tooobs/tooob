#!/bin/sh
# The Host's cargo tests on Linux (musl), in a podman container, before a Release is cut: the
# release workflow is otherwise the first place they run on Linux. Arguments go to cargo test,
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
