#!/bin/bash
# Builds the server the way ./your_program.sh does, then runs the end-to-end tests in
# this directory against it: all of them, after the unit tests (`cargo test`), or just
# the ones named. For example:
#   tests/run.sh
#   tests/run.sh incr.sh blpop.py
set -u
cd "$(dirname "$0")/.."
cargo build --release --quiet --target-dir=/tmp/codecrafters-build-redis-rust || exit 1
bin=/tmp/codecrafters-build-redis-rust/release/codecrafters-redis

failed=""
if [ $# -eq 0 ]; then
  echo "##### unit tests (cargo test)"
  cargo test --quiet || failed=" cargo-test"
  echo
  for file in tests/*.sh tests/*.py; do
    case ${file##*/} in
      run.sh | helpers.*) ;;
      *) set -- "$@" "${file##*/}" ;;
    esac
  done
fi

for test in "$@"; do
  echo "##### $test"
  case $test in
    *.py) python3 "tests/$test" "$bin" ;;
    *) bash "tests/$test" "$bin" ;;
  esac || failed="$failed $test"
  echo
done

if [ -n "$failed" ]; then
  echo "Failed:$failed"
  exit 1
fi
echo "All tests passed."
