#!/bin/bash
# LLEN, from the "Query list length" stage on.
source "$(dirname "$0")/helpers.sh"

echo "== stage test"
check ':4\r\n' RPUSH list_key a b c d
check ':4\r\n' LLEN list_key
check ':0\r\n' LLEN missing_list_key
check ':37\r\n' RPUSH big $(seq 1 37)
check ':37\r\n' LLEN big

echo "== after LPUSH, lowercase"
check ':5\r\n' LPUSH list_key z
check ':5\r\n' llen list_key

echo "== errors"
check "-ERR wrong number of arguments for 'llen' command\\r\\n" LLEN
check "-ERR wrong number of arguments for 'llen' command\\r\\n" LLEN list_key extra
check '+OK\r\n' SET str val
check "$wrongtype" LLEN str

echo "== expired keys read as missing in LLEN, GET and LRANGE"
check '+OK\r\n' SET temp1 v PX 100
check '+OK\r\n' SET temp2 v PX 100
check '+OK\r\n' SET temp3 v PX 100
sleep 0.2
check ':0\r\n' LLEN temp1
check '$-1\r\n' GET temp2
check "$(array)" LRANGE temp3 0 -1

echo "== GET and LRANGE regressions"
check '$3\r\nval\r\n' GET str
check '$-1\r\n' GET missing
check "$wrongtype" GET list_key
check '+OK\r\n' SET k v EX 100
check '$1\r\nv\r\n' GET k
check "$(array z a b c d)" LRANGE list_key 0 -1
check "$(array a b)" LRANGE list_key 1 2
check "$(array)" LRANGE missing 0 1
check "$wrongtype" LRANGE str 0 1
check '-ERR value is not an integer or out of range\r\n' LRANGE list_key x 1

echo "== other regressions"
check '+PONG\r\n' PING
check '$3\r\nhey\r\n' ECHO hey
check ':6\r\n' RPUSH list_key e
check ':6\r\n' LLEN list_key

finish
