#!/usr/bin/env bash
set -euo pipefail

cargo build --profile release-prof --example prof_solve
valgrind --tool=massif --stacks=yes target/release-prof/examples/prof_solve "${1:-500}" ${@:2}
ms_print "$(ls -t massif.out.* | head -n1)" | less
