#!/bin/sh
# The file the case sends, in the run's working directory.
set -eu
printf '%%PDF-1.4\n%% a stand-in report for the eval\n' >report.pdf
