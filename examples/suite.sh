#!/usr/bin/env bash
cd "$(dirname "$0")/.."
export CARGO_BUILD_JOBS=3 CARGO_TARGET_DIR=/tmp/mb1138-target
: > /tmp/mb1138-suite.txt
echo "== lib" >> /tmp/mb1138-suite.txt
nice cargo test --lib 2>&1 | grep -E "^test result|FAILED|panicked" >> /tmp/mb1138-suite.txt
for f in tests/*.rs; do
  n=$(basename "$f" .rs)
  echo "== $n" >> /tmp/mb1138-suite.txt
  nice cargo test --test "$n" 2>&1 | grep -E "^test result|FAILED|panicked|^error" >> /tmp/mb1138-suite.txt
done
echo "== done" >> /tmp/mb1138-suite.txt
