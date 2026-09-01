#!/usr/bin/env bash
set -euo pipefail

cargo build --profile release-prof --bin prof-solve --features bin
valgrind --tool=massif --stacks=yes target/release-prof/prof-solve "${1:-500}" ${@:2}
ms_print "$(ls -t massif.out.* | head -n1)" | less
