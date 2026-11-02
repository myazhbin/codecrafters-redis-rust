#!/bin/bash
# LPOP, from the "Remove an element" stages on.
source "$(dirname "$0")/helpers.sh"

echo "== stage test"
check ':5\r\n' RPUSH list_key one two three four five
check '$3\r\none\r\n' LPOP list_key
check "$(array two three four five)" LRANGE list_key 0 -1
check '$-1\r\n' LPOP missing_key

echo "== with a count"
check "$(array two three)" LPOP list_key 2
check "$(array four five)" LRANGE list_key 0 -1
check "$(array four five)" LPOP list_key 10

echo "== popping the last element deletes the key"
check ':0\r\n' LLEN list_key
check '$-1\r\n' GET list_key
check '$-1\r\n' LPOP list_key
check '*-1\r\n' LPOP list_key 2
check ':1\r\n' RPUSH solo x
check '$1\r\nx\r\n' LPOP solo
check '$-1\r\n' GET solo
check '+OK\r\n' SET solo now-a-string
check '$12\r\nnow-a-string\r\n' GET solo

echo "== count 0, bad counts, arity, wrong type"
check ':1\r\n' RPUSH zero a
check "$(array)" LPOP zero 0
check ':1\r\n' LLEN zero
check '-ERR value is out of range, must be positive\r\n' LPOP zero -1
check '-ERR value is out of range, must be positive\r\n' LPOP zero abc
check "-ERR wrong number of arguments for 'lpop' command\\r\\n" LPOP
check "-ERR wrong number of arguments for 'lpop' command\\r\\n" LPOP zero 1 2
check '+OK\r\n' SET str val
check "$wrongtype" LPOP str
check "$wrongtype" LPOP str 2

echo "== an expired key reads as missing"
check '+OK\r\n' SET temp v PX 100
sleep 0.2
check '$-1\r\n' LPOP temp

echo "== stage description example, LPUSH order, lowercase"
check ':4\r\n' RPUSH ex a b c d
check '$1\r\na\r\n' LPOP ex
check ':3\r\n' LPUSH ord a b c
check '$1\r\nc\r\n' lpop ord
check "$(array b a)" LRANGE ord 0 -1

echo "== other regressions"
check '+PONG\r\n' PING
check '$3\r\nhey\r\n' ECHO hey
check '$3\r\nval\r\n' GET str

finish
