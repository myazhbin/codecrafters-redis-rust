#!/bin/bash
# LRANGE, from the "List elements" stages on.
source "$(dirname "$0")/helpers.sh"

echo "== stage examples"
check ':5\r\n' RPUSH list_key a b c d e
check "$(array a b)" LRANGE list_key 0 1
check "$(array c d e)" LRANGE list_key 2 4
check "$(array a b c)" LRANGE list_key 0 2
check "$(array a)" LRANGE list_key 0 0

echo "== stage edge cases"
check "$(array)" LRANGE missing_key 0 1
check "$(array)" LRANGE list_key 5 6
check "$(array)" LRANGE list_key 10 20
check "$(array b c d e)" LRANGE list_key 1 100
check "$(array)" LRANGE list_key 3 1

echo "== negative indexes"
check "$(array d e)" LRANGE list_key -2 -1
check "$(array a b c d e)" LRANGE list_key 0 -1
check "$(array a b)" LRANGE list_key -100 1
check "$(array)" LRANGE list_key -1 -2
check "$(array c)" LRANGE list_key 2 -3
check "$(array)" LRANGE list_key 0 -6

echo "== errors"
check '-ERR value is not an integer or out of range\r\n' LRANGE list_key a 1
check '-ERR value is not an integer or out of range\r\n' LRANGE missing_key 0 x
check "-ERR wrong number of arguments for 'lrange' command\\r\\n" LRANGE list_key 0
check '+OK\r\n' SET str val
check "$wrongtype" LRANGE str 0 1

echo "== an expired string key reads as missing"
check '+OK\r\n' SET temp v PX 100
sleep 0.2
check "$(array)" LRANGE temp 0 1

echo "== regressions"
check ':6\r\n' RPUSH list_key f
check "$(array e f)" LRANGE list_key -2 -1
check '+OK\r\n' SET k v PX 100000
check '$1\r\nv\r\n' GET k
check '-ERR value is not an integer or out of range\r\n' SET k v EX abc
check '-ERR value is not an integer or out of range\r\n' SET k v EX -5
check "-ERR invalid expire time in 'set' command\\r\\n" SET k v PX 0
check '+PONG\r\n' PING
check '$3\r\nhey\r\n' ECHO hey

finish
