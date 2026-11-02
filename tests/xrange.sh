#!/bin/bash
# XRANGE, from the "Query entries from stream" stages on.
source "$(dirname "$0")/helpers.sh"

invalid='-ERR Invalid stream ID specified as stream command argument\r\n'

echo "== stage test"
check '$3\r\n0-1\r\n' XADD stream_key 0-1 foo bar
check '$3\r\n0-2\r\n' XADD stream_key 0-2 bar baz
check '$3\r\n0-3\r\n' XADD stream_key 0-3 baz foo
check "*2\\r\\n$(entry 0-2 bar baz)$(entry 0-3 baz foo)" XRANGE stream_key 0-2 0-3

echo "== stage description example: IDs without a sequence number"
check '$15\r\n1526985054069-0\r\n' XADD some_key 1526985054069-0 temperature 36 humidity 95
check '$15\r\n1526985054079-0\r\n' XADD some_key 1526985054079-0 temperature 37 humidity 94
check "*2\\r\\n$(entry 1526985054069-0 temperature 36 humidity 95)$(entry 1526985054079-0 temperature 37 humidity 94)" \
  XRANGE some_key 1526985054069 1526985054079
check '$3\r\n5-1\r\n' XADD ms 5-1 a 1
check '$3\r\n5-2\r\n' XADD ms 5-2 b 2
check '$3\r\n6-0\r\n' XADD ms 6-0 c 3
check "*2\\r\\n$(entry 5-1 a 1)$(entry 5-2 b 2)" XRANGE ms 5 5
check "*2\\r\\n$(entry 5-2 b 2)$(entry 6-0 c 3)" XRANGE ms 5-2 6

echo "== - and +"
check "*2\\r\\n$(entry 0-1 foo bar)$(entry 0-2 bar baz)" XRANGE stream_key - 0-2
check "*2\\r\\n$(entry 0-2 bar baz)$(entry 0-3 baz foo)" XRANGE stream_key 0-2 +
check "*3\\r\\n$(entry 0-1 foo bar)$(entry 0-2 bar baz)$(entry 0-3 baz foo)" XRANGE stream_key - +

echo "== empty ranges"
check '*0\r\n' XRANGE stream_key 0-3 0-2
check '*0\r\n' XRANGE stream_key 1-0 +
check '*0\r\n' XRANGE stream_key + -
check '*0\r\n' XRANGE missing - +

echo "== errors"
check "$invalid" XRANGE stream_key abc +
check "$invalid" XRANGE stream_key - 1-x
check "-ERR wrong number of arguments for 'xrange' command\\r\\n" XRANGE stream_key -
check '+OK\r\n' SET str val
check "$wrongtype" XRANGE str - +

echo "== XADD regressions"
check '$3\r\n0-1\r\n' XADD p '0-*' a b
check '$3\r\n5-0\r\n' XADD p '5-*' c d
check '$3\r\n5-1\r\n' XADD p '5-*' e f
check '$3\r\n7-0\r\n' XADD p 7 g h
check "*4\\r\\n$(entry 0-1 a b)$(entry 5-0 c d)$(entry 5-1 e f)$(entry 7-0 g h)" XRANGE p - +
check "$invalid" XADD p abc a b
check "$invalid" XADD p - a b
check "$invalid" XADD p + a b
check "$invalid" XADD p '-*' a b
check '-ERR The ID specified in XADD must be greater than 0-0\r\n' XADD p 0-0 a b
check '-ERR The ID specified in XADD is equal or smaller than the target stream top item\r\n' XADD p '1-*' a b

echo "== other regressions"
check '+PONG\r\n' PING
check '+stream\r\n' TYPE stream_key

finish
