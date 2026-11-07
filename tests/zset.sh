#!/bin/bash
# Sorted sets, from the "Create a sorted set" stage on.
source "$(dirname "$0")/helpers.sh"

zadd_arity="-ERR wrong number of arguments for 'zadd' command\\r\\n"

echo "== stage test: ZADD creates a sorted set"
check ':1\r\n' ZADD zset_key 10.0 zset_member
check '+zset\r\n' TYPE zset_key

echo "== several members at once, and scores in any form"
check ':3\r\n' ZADD multi 1 a 2 b 3 c
check ':2\r\n' ZADD forms inf top -inf bottom
check ':2\r\n' ZADD forms 1e3 sci -2.5 negative

echo "== errors, which leave nothing behind"
check '-ERR value is not a valid float\r\n' ZADD bad abc member
check '-ERR value is not a valid float\r\n' ZADD bad nan member
check '-ERR value is not a valid float\r\n' ZADD bad 1 ok x notok
check '+none\r\n' TYPE bad
check '-ERR syntax error\r\n' ZADD bad 1 a 2
check "$zadd_arity" ZADD bad 1
check "$zadd_arity" ZADD bad
check '+OK\r\n' SET str v
check "$wrongtype" ZADD str 1 a
check "$wrongtype" GET zset_key
check ':1\r\n' zadd lower 1 a

finish
