#!/bin/bash
# TYPE, from "The TYPE command" stage on.
source "$(dirname "$0")/helpers.sh"

echo "== stage test"
check '+OK\r\n' SET some_key foo
check '+string\r\n' TYPE some_key
check '+none\r\n' TYPE missing_key

echo "== lists, emptied lists, expired keys"
check ':2\r\n' RPUSH list_key a b
check '+list\r\n' TYPE list_key
check "$(array a b)" LPOP list_key 2
check '+none\r\n' TYPE list_key
check '+OK\r\n' SET temp v PX 100
sleep 0.2
check '+none\r\n' TYPE temp

echo "== arity, lowercase"
check "-ERR wrong number of arguments for 'type' command\\r\\n" TYPE
check "-ERR wrong number of arguments for 'type' command\\r\\n" TYPE some_key extra
check '+string\r\n' type some_key

echo "== regressions"
check '+PONG\r\n' PING
check '$3\r\nfoo\r\n' GET some_key

finish
