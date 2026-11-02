#!/bin/bash
# XREAD without BLOCK, from the "Query single stream using XREAD" stage on.
# (XREAD BLOCK is in xread_block.py.)
source "$(dirname "$0")/helpers.sh"

invalid='-ERR Invalid stream ID specified as stream command argument\r\n'
arity="-ERR wrong number of arguments for 'xread' command\\r\\n"

echo "== stage test"
check '$3\r\n0-1\r\n' XADD stream_key 0-1 temperature 96
check "*1\\r\\n$(stream stream_key "$(entry 0-1 temperature 96)")" XREAD STREAMS stream_key 0-0

echo "== stage description example: only entries after the ID"
check '$15\r\n1526985054069-0\r\n' XADD some_key 1526985054069-0 temperature 36 humidity 95
check '$15\r\n1526985054079-0\r\n' XADD some_key 1526985054079-0 temperature 37 humidity 94
first=$(entry 1526985054069-0 temperature 36 humidity 95)
second=$(entry 1526985054079-0 temperature 37 humidity 94)
check "*1\\r\\n$(stream some_key "$second")" XREAD STREAMS some_key 1526985054069-0
check "*1\\r\\n$(stream some_key "$second")" XREAD STREAMS some_key 1526985054069
check "*1\\r\\n$(stream some_key "$first" "$second")" XREAD STREAMS some_key 0

echo "== several streams, lowercase"
check "*2\\r\\n$(stream stream_key "$(entry 0-1 temperature 96)")$(stream some_key "$second")" \
  XREAD STREAMS stream_key some_key 0-0 1526985054069-0
check "*1\\r\\n$(stream some_key "$second")" \
  XREAD STREAMS stream_key missing some_key 0-1 0-0 1526985054069-0
check "*1\\r\\n$(stream stream_key "$(entry 0-1 temperature 96)")" XREAD streams stream_key 0-0

echo "== nothing to read gives a null array"
check '*-1\r\n' XREAD STREAMS stream_key 0-1
check '*-1\r\n' XREAD STREAMS missing 0-0
check '*-1\r\n' XREAD STREAMS stream_key '$'
check '*-1\r\n' XREAD STREAMS missing '$'

echo "== errors"
check "$invalid" XREAD STREAMS stream_key abc
check "$invalid" XREAD STREAMS stream_key -
check "$invalid" XREAD STREAMS stream_key +
check "-ERR Unbalanced 'xread' list of streams: for each stream key an ID or '\$' must be specified.\\r\\n" \
  XREAD STREAMS a b 0
check "$arity" XREAD STREAMS stream_key
check "$arity" XREAD
check '-ERR syntax error\r\n' XREAD COUNT 1 STREAMS stream_key 0
check '+OK\r\n' SET str val
check "$wrongtype" XREAD STREAMS str 0

echo "== regressions"
check "*1\\r\\n$(entry 0-1 temperature 96)" XRANGE stream_key - +
check '+PONG\r\n' PING

finish
