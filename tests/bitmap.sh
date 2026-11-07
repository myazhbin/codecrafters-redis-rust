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

echo "== stage test: GETBIT reads a bit back"
check ':0\r\n' SETBIT bit_key 2 1
check ':1\r\n' GETBIT bit_key 2
check ':0\r\n' GETBIT bit_key 3
check ':0\r\n' GETBIT new_key 2

echo "== bits past the end of the string are 0"
check ':0\r\n' GETBIT bit_key 8
check ':0\r\n' GETBIT bit_key 4294967295
check ':1\r\n' SETBIT bit_key 2 0
check ':0\r\n' GETBIT bit_key 2

echo "== stage test: GETBIT reads a string set with SET, A being 01000001"
check '+OK\r\n' SET mykey A
check ':1\r\n' GETBIT mykey 1
check ':1\r\n' GETBIT mykey 7
check ':0\r\n' GETBIT mykey 0
check ':0\r\n' GETBIT mykey 2

echo "== stage test: GET reads a bitmap as a string, 01000000 being @"
check ':0\r\n' SETBIT at_key 1 1
check '$1\r\n@\r\n' GET at_key

echo "== spelling out \"Hi\" (01001000 01101001) a bit at a time"
for offset in 1 4 9 10 12 15; do
  check ':0\r\n' SETBIT hi "$offset" 1
done
check '$2\r\nHi\r\n' GET hi

echo "== SETBIT changes a string set with SET, keeping its expiry"
check '+OK\r\n' SET ttl A PX 300
check ':0\r\n' SETBIT ttl 6 1
check '$1\r\nC\r\n' GET ttl
sleep 0.4
check '$-1\r\n' GET ttl

echo "== errors"
check "$offset_error" GETBIT bit_key -1
check "$offset_error" GETBIT bit_key 4294967296
check "-ERR wrong number of arguments for 'getbit' command\\r\\n" GETBIT bit_key
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
