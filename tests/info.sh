#!/bin/bash
# INFO, from "The INFO command" stage on.
source "$(dirname "$0")/helpers.sh"

role='$11\r\nrole:master\r\n'
nothing='$0\r\n\r\n'

echo "== stage test: the replication section gives the server's role"
check "$role" INFO replication
check "$role" info replication

echo "== any case, no section, the groups of sections, several sections"
check "$role" INFO REPLICATION
check "$role" INFO
check "$role" INFO default
check "$role" INFO all
check "$role" INFO everything
check "$role" INFO server replication

echo "== a section that doesn't exist is left out, not an error"
check "$nothing" INFO server
check "$nothing" INFO bogus

echo "== inside a transaction"
report "MULTI, INFO replication, EXEC" "+OK\\r\\n+QUEUED\\r\\n*1\\r\\n$role" \
  "$({ resp MULTI; resp INFO replication; resp EXEC; } | send)"

echo "== regressions"
check '+PONG\r\n' PING
check '+OK\r\n' SET k v
check '$1\r\nv\r\n' GET k

finish
