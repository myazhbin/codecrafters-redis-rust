#!/bin/bash
# Bitmaps, from the "Create a bitmap" stage on.
source "$(dirname "$0")/helpers.sh"

offset_error='-ERR bit offset is not an integer or out of range\r\n'

echo "== stage test: SETBIT creates a bitmap, replying with the bit's old value"
check ':0\r\n' SETBIT bitmap_key 3 1
check '+string\r\n' TYPE bitmap_key
check ':1\r\n' SETBIT bitmap_key 3 1
check ':1\r\n' SETBIT bitmap_key 3 0
check ':0\r\n' SETBIT bitmap_key 3 0

echo "== errors"
check "$offset_error" SETBIT bad -1 1
check "$offset_error" SETBIT bad x 1
check "$offset_error" SETBIT bad 4294967296 1
check '-ERR bit is not an integer or out of range\r\n' SETBIT bad 0 2
check '-ERR bit is not an integer or out of range\r\n' SETBIT bad 0 x
check '+none\r\n' TYPE bad
check ':1\r\n' RPUSH list a
check "$wrongtype" SETBIT list 0 1
check "-ERR wrong number of arguments for 'setbit' command\\r\\n" SETBIT bad 0
check ':0\r\n' setbit lower 0 1

finish
