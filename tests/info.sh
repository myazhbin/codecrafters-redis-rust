#!/bin/bash
# INFO, from the "The INFO command" stages on.
source "$(dirname "$0")/helpers.sh"

# The replication section after the role, which is the same on a master and a replica.
# Every line ends in \r\n, and then the bulk string has a \r\n of its own to end it.
rest='master_replid:8371b4fb1155b71f4a04d3e1bc3e18c4a990aeeb\r\nmaster_repl_offset:0\r\n'
master='$91\r\nrole:master\r\n'$rest'\r\n'
slave='$90\r\nrole:slave\r\n'$rest'\r\n'
nothing='$0\r\n\r\n'

echo "== stage test: without --replicaof, the server is a master, with an ID and offset"
check "$master" INFO replication
check "$master" info replication

echo "== any case, no section, the groups of sections, several sections"
check "$master" INFO REPLICATION
check "$master" INFO
check "$master" INFO default
check "$master" INFO all
check "$master" INFO everything
check "$master" INFO server replication

echo "== a section that doesn't exist is left out, not an error"
check "$nothing" INFO server
check "$nothing" INFO bogus

echo "== inside a transaction"
report "MULTI, INFO replication, EXEC" "+OK\\r\\n+QUEUED\\r\\n*1\\r\\n$master" \
  "$({ resp MULTI; resp INFO replication; resp EXEC; } | send)"

echo "== regressions"
check '+PONG\r\n' PING
check '+OK\r\n' SET k v
check '$1\r\nv\r\n' GET k

echo "== stage test: with --replicaof, the server is a replica"
# From here on, `check` talks to the replica, which names the first server as its master.
master_port=$port
port=6382
"$bin" --port "$port" --replicaof "localhost $master_port" >/dev/null &
replica=$!
wait_for_port "$port"
check "$slave" INFO replication
check "$slave" info replication
check "$slave" INFO
check "$nothing" INFO server
check '+PONG\r\n' PING
kill $replica 2>/dev/null

finish
