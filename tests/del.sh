#!/bin/bash
# DEL, added with the "Write multiple commands" AOF stage, which lists it among the writes.
source "$(dirname "$0")/helpers.sh"

arity="-ERR wrong number of arguments for 'del' command\\r\\n"

echo "== DEL removes keys and replies with how many it removed"
check '+OK\r\n' SET a 1
check ':1\r\n' DEL a
check '$-1\r\n' GET a
check ':0\r\n' DEL a
check ':0\r\n' DEL missing

echo "== several keys at once, including missing and repeated ones"
check '+OK\r\n' SET b 2
check '+OK\r\n' SET c 3
check ':2\r\n' DEL b missing c b
check '*0\r\n' KEYS '*'

echo "== any type of value"
check ':1\r\n' RPUSH list x
check '$3\r\n1-1\r\n' XADD stream 1-1 f v
check ':2\r\n' DEL list stream
check '+none\r\n' TYPE list
check '+none\r\n' TYPE stream

echo "== a key that has expired counts as gone"
check '+OK\r\n' SET old v PX 100
sleep 0.2
check ':0\r\n' DEL old

echo "== arity, lowercase"
check "$arity" DEL
check '+OK\r\n' SET d 4
check ':1\r\n' del d

finish
