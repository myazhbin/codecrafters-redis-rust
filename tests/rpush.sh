#!/bin/bash
# RPUSH, from the "Create a list" stage on, plus the earlier stages' basics.
source "$(dirname "$0")/helpers.sh"

echo "== stage test: RPUSH list_key element"
check ':1\r\n' RPUSH list_key element

echo "== append, multiple elements, lowercase, no elements"
check ':2\r\n' RPUSH list_key bar
check ':5\r\n' RPUSH list_key a b c
check ':1\r\n' rpush other x
check "-ERR wrong number of arguments for 'rpush' command\\r\\n" RPUSH list_key

echo "== wrong types: RPUSH on a string, GET on a list"
check '+OK\r\n' SET str val
check "$wrongtype" RPUSH str x
check "$wrongtype" GET list_key

echo "== an expired string key is replaced by a new list"
check '+OK\r\n' SET temp hello PX 100
sleep 0.2
check ':1\r\n' RPUSH temp x

echo "== earlier stages: PING, ECHO, SET/GET, a missing key, SET overwriting a list"
check '+PONG\r\n' PING
check '$3\r\nhey\r\n' ECHO hey
check '+OK\r\n' SET k v
check '$1\r\nv\r\n' GET k
check '$-1\r\n' GET missing
check '+OK\r\n' SET list_key overwritten
check '$11\r\noverwritten\r\n' GET list_key

echo "== several commands sent in one go are all answered, in order"
report "PING, ECHO hey, SET k2 v, GET k2, GET missing on one connection" \
  '+PONG\r\n$3\r\nhey\r\n+OK\r\n$1\r\nv\r\n$-1\r\n' \
  "$({ resp PING; resp ECHO hey; resp SET k2 v; resp GET k2; resp GET missing; } | send)"

finish
