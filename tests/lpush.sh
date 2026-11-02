#!/bin/bash
# LPUSH, from the "Prepend elements" stage on.
source "$(dirname "$0")/helpers.sh"

echo "== stage test"
check ':1\r\n' LPUSH list_key c
check ':3\r\n' LPUSH list_key b a
check "$(array a b c)" LRANGE list_key 0 -1

echo "== stage description example"
check ':3\r\n' LPUSH other a b c
check "$(array c b a)" LRANGE other 0 -1

echo "== mixed with RPUSH, lowercase"
check ':4\r\n' RPUSH list_key d
check ':5\r\n' LPUSH list_key z
check "$(array z a b c d)" LRANGE list_key 0 -1
check ':1\r\n' lpush lower x

echo "== errors"
check "-ERR wrong number of arguments for 'lpush' command\\r\\n" LPUSH list_key
check "-ERR wrong number of arguments for 'lpush' command\\r\\n" LPUSH
check '+OK\r\n' SET str val
check "$wrongtype" LPUSH str x

echo "== an expired string key is replaced by a new list"
check '+OK\r\n' SET temp v PX 100
sleep 0.2
check ':1\r\n' LPUSH temp x
check "$(array x)" LRANGE temp 0 -1

echo "== RPUSH regressions"
check ':1\r\n' RPUSH rlist a
check ':3\r\n' RPUSH rlist b c
check "$(array a b c)" LRANGE rlist 0 -1
check "-ERR wrong number of arguments for 'rpush' command\\r\\n" RPUSH rlist
check "$wrongtype" RPUSH str x
check '+OK\r\n' SET temp2 v PX 100
sleep 0.2
check ':1\r\n' RPUSH temp2 x

echo "== other regressions"
check '+PONG\r\n' PING
check '$3\r\nhey\r\n' ECHO hey
check '$3\r\nval\r\n' GET str

finish
