#!/bin/bash
# --port, from the "Configure listening port" stage on.
port=6380
source "$(dirname "$0")/helpers.sh"

echo "== stage test: the server listens on the port given with --port"
check '+PONG\r\n' PING
check '+OK\r\n' SET k v
check '$1\r\nv\r\n' GET k

echo "== another server can run alongside, on another port"
"$bin" --port 6381 >/dev/null &
second=$!
wait_for_port 6381
report "PING to the server on port 6381" '+PONG\r\n' "$(resp PING | send 6381)"
report "it has its own data: GET k there" '$-1\r\n' "$(resp GET k | send 6381)"
kill $second 2>/dev/null

echo "== bad arguments make the server exit with a message"
refused() { # refused <expected message> <argument...>
  local want=$1 got status
  shift
  # The alarm stops a server that wrongly starts up anyway.
  got=$(perl -e 'alarm 2; exec @ARGV' "$bin" "$@" 2>&1)
  status=$?
  report "the server exits with: $want" "1 $want" "$status $got"
}
refused 'invalid port: abc' --port abc
refused 'invalid port: 70000' --port 70000
refused '--port needs a port number' --port
refused 'unknown argument: --bogus' --bogus

finish
