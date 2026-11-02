#!/bin/bash
# XADD, from the "Create a stream" stage through the auto-generated ID stages.
source "$(dirname "$0")/helpers.sh"

smaller='-ERR The ID specified in XADD is equal or smaller than the target stream top item\r\n'
zero='-ERR The ID specified in XADD must be greater than 0-0\r\n'
invalid='-ERR Invalid stream ID specified as stream command argument\r\n'
arity="-ERR wrong number of arguments for 'xadd' command\\r\\n"

echo "== stage test"
check '$3\r\n0-1\r\n' XADD stream_key 0-1 foo bar
check '+stream\r\n' TYPE stream_key

echo "== stage description example, TYPE of other keys"
check '$15\r\n1526919030474-0\r\n' XADD sensor 1526919030474-0 temperature 36 humidity 95
check '+OK\r\n' SET str val
check '+string\r\n' TYPE str
check '+none\r\n' TYPE missing_key

echo "== IDs must increase"
check '$3\r\n1-1\r\n' XADD v 1-1 a b
check '$3\r\n1-2\r\n' XADD v 1-2 c d
check "$smaller" XADD v 1-2 e f
check "$smaller" XADD v 0-3 e f
check "$zero" XADD v 0-0 e f
check "$zero" XADD fresh 0-0 a b
check '+none\r\n' TYPE fresh

echo "== generated sequence numbers"
check '$3\r\n0-1\r\n' XADD p '0-*' a b
check '$3\r\n5-0\r\n' XADD p '5-*' a b
check '$3\r\n5-1\r\n' XADD p '5-*' a b
check "$smaller" XADD p '3-*' a b
check '$3\r\n7-0\r\n' XADD p 7 a b

echo "== generated IDs"
id=$(resp XADD auto '*' a b | nc -w 1 127.0.0.1 6379 | tr -d '\r' | sed -n 2p)
now=$(($(date +%s) * 1000))
near_now=no
if [[ $id =~ ^[0-9]+-0$ ]] && ((${id%-*} > now - 5000 && ${id%-*} < now + 5000)); then
  near_now=yes
fi
report "XADD auto * a b gives an ID from the current time (got $id)" yes "$near_now"
check '$16\r\n99999999999999-5\r\n' XADD later 99999999999999-5 a b
check '$16\r\n99999999999999-6\r\n' XADD later '*' c d

echo "== bad IDs, arity, wrong types"
check "$invalid" XADD k abc a b
check "$invalid" XADD k 1-x a b
check "$invalid" XADD k 1- a b
check "$arity" XADD k 1-1
check "$arity" XADD k 1-1 a
check "$arity" XADD k
check "$wrongtype" XADD str 1-1 a b
check "$wrongtype" GET stream_key
check "$wrongtype" LLEN stream_key
check "$wrongtype" LPUSH stream_key x
check "$wrongtype" LPOP stream_key
check "$wrongtype" BLPOP stream_key 0

echo "== an expired key is replaced by a new stream"
check '+OK\r\n' SET temp v PX 100
sleep 0.2
check '$3\r\n1-1\r\n' XADD temp 1-1 a b
check '+stream\r\n' TYPE temp

echo "== regressions"
check '+PONG\r\n' PING
check ':2\r\n' RPUSH list a b
check "$(array a b)" LRANGE list 0 -1
check '+list\r\n' TYPE list

finish
