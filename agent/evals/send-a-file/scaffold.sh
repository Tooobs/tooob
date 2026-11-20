#!/bin/sh
# The run must test the `tooob` built from main (#152), first on PATH, not an installed Release:
# the skill from main names what only main's CLI has. Then the file the case sends, in the run's
# working directory.
set -eu
tooob=$(command -v tooob || true)
case "$tooob" in
*/target/debug/tooob) ;;
*)
    echo "the eval needs main's build first on PATH, not '${tooob:-no tooob}':" >&2
    echo '  cargo build --manifest-path host/Cargo.toml' >&2
    echo '  PATH="$PWD/target/debug:$PATH" claude plugin eval agent ... (agent/README.md)' >&2
    exit 1
    ;;
esac
printf '%%PDF-1.4\n%% a stand-in report for the eval\n' >report.pdf
