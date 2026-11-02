#!/bin/bash
# INCR, from the "The INCR command" stages on.
source "$(dirname "$0")/helpers.sh"

notint='-ERR value is not an integer or out of range\r\n'
arity="-ERR wrong number of arguments for 'incr' command\\r\\n"

echo "== stage test"
check '+OK\r\n' SET foo 41
check ':42\r\n' INCR foo
check ':43\r\n' INCR foo
check '$2\r\n43\r\n' GET foo

echo "== missing keys start from 0"
check ':1\r\n' INCR missing
check ':2\r\n' INCR missing
check '$1\r\n2\r\n' GET missing
check '+OK\r\n' SET old 100 PX 100
sleep 0.2
check ':1\r\n' INCR old

echo "== values that aren't integers, overflow"
check '+OK\r\n' SET s abc
check "$notint" INCR s
check '$3\r\nabc\r\n' GET s
check '+OK\r\n' SET float 1.5
check "$notint" INCR float
check '+OK\r\n' SET big 9223372036854775807
check '-ERR increment or decrement would overflow\r\n' INCR big
check '$19\r\n9223372036854775807\r\n' GET big
check '+OK\r\n' SET neg -5
check ':-4\r\n' INCR neg

echo "== an expiry is kept"
check '+OK\r\n' SET ttl 10 PX 1000
check ':11\r\n' INCR ttl
sleep 1.2
check '$-1\r\n' GET ttl

echo "== wrong types, arity, lowercase"
check ':1\r\n' RPUSH list a
check "$wrongtype" INCR list
check '$3\r\n1-1\r\n' XADD stream 1-1 a b
check "$wrongtype" INCR stream
check "$arity" INCR
check "$arity" INCR foo bar
check ':44\r\n' incr foo

echo "== regressions"
check '+PONG\r\n' PING
check '+OK\r\n' SET k v
check '$1\r\nv\r\n' GET k

finish
