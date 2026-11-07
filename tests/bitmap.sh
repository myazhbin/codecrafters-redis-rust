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

echo "== stage test: SETBIT past the end grows the string, as STRLEN shows"
check ':0\r\n' SETBIT grow_key 1 1
check ':1\r\n' STRLEN grow_key
check ':0\r\n' SETBIT grow_key 10 1
check ':1\r\n' GETBIT grow_key 10
check ':2\r\n' STRLEN grow_key
check '$2\r\n@ \r\n' GET grow_key
check ':0\r\n' SETBIT grow_key 87 0
check ':11\r\n' STRLEN grow_key
check ':1\r\n' SETBIT grow_key 10 0
check ':11\r\n' STRLEN grow_key

echo "== STRLEN of other keys"
check ':0\r\n' STRLEN missing
check '+OK\r\n' SET text hello
check ':5\r\n' STRLEN text
check ':1\r\n' RPUSH a-list x
check "$wrongtype" STRLEN a-list
check "-ERR wrong number of arguments for 'strlen' command\\r\\n" STRLEN

echo "== stage test: BITCOUNT counts the set bits, in all or some of the bytes"
check ':0\r\n' SETBIT count_key 1 1
check ':0\r\n' SETBIT count_key 10 1
check ':2\r\n' BITCOUNT count_key
check ':2\r\n' BITCOUNT count_key 0 1
check ':1\r\n' BITCOUNT count_key 0 0
check ':1\r\n' BITCOUNT count_key 1 1
check ':0\r\n' BITCOUNT count_key 2 5
check ':1\r\n' BITCOUNT count_key 1 100
check ':0\r\n' BITCOUNT count_key 1 0
check ':0\r\n' BITCOUNT missing
check ':0\r\n' BITCOUNT missing 0 10

echo "== Redis's own examples, negative indexes, and BYTE and BIT"
check '+OK\r\n' SET mykey foobar
check ':26\r\n' BITCOUNT mykey
check ':4\r\n' BITCOUNT mykey 0 0
check ':6\r\n' BITCOUNT mykey 1 1
check ':6\r\n' BITCOUNT mykey 1 1 BYTE
check ':17\r\n' BITCOUNT mykey 5 30 BIT
check ':26\r\n' BITCOUNT mykey 0 -1
check ':4\r\n' BITCOUNT mykey -1 -1

echo "== BITCOUNT errors"
check '-ERR syntax error\r\n' BITCOUNT mykey 0
check '-ERR syntax error\r\n' BITCOUNT mykey 0 1 WORDS
check '-ERR syntax error\r\n' BITCOUNT mykey 0 1 BIT extra
check '-ERR value is not an integer or out of range\r\n' BITCOUNT mykey a 1
check ':1\r\n' RPUSH count-list x
check "$wrongtype" BITCOUNT count-list
check "-ERR wrong number of arguments for 'bitcount' command\\r\\n" BITCOUNT

echo "== stage test: BITOP AND keeps the bits set in every source"
check ':0\r\n' SETBIT key1 0 1
check ':0\r\n' SETBIT key1 4 1
check ':0\r\n' SETBIT key2 0 1
check ':0\r\n' SETBIT key2 6 1
check ':1\r\n' BITOP AND dest key1 key2
check ':1\r\n' GETBIT dest 0
check ':0\r\n' GETBIT dest 4
check ':0\r\n' GETBIT dest 6
check ':1\r\n' BITCOUNT dest

echo "== stage test: shorter sources count as padded with zeros"
check ':0\r\n' SETBIT long1 1 1
check ':0\r\n' SETBIT long1 10 1
check ':0\r\n' SETBIT short2 1 1
check ':2\r\n' BITOP AND dest2 long1 short2
check ':1\r\n' GETBIT dest2 1
check ':0\r\n' GETBIT dest2 10
check ':2\r\n' STRLEN dest2
check ':2\r\n' BITOP AND dest3 short2 long1
check ':1\r\n' BITCOUNT dest3
check ':2\r\n' BITOP AND dest4 long1 missing
check ':0\r\n' BITCOUNT dest4

echo "== the result replaces what was at the destination, expiry and all"
check '+OK\r\n' SET old-dest something PX 200
check ':1\r\n' BITOP and old-dest key1 key2
sleep 0.3
check ':1\r\n' GETBIT old-dest 0
check ':1\r\n' RPUSH list-dest x
check ':1\r\n' BITOP AND list-dest key1
check '+string\r\n' TYPE list-dest

echo "== missing sources, and an empty result deleting the destination"
check ':1\r\n' BITOP AND with-missing key1 missing
check ':0\r\n' BITCOUNT with-missing
check ':0\r\n' BITOP AND nothing missing also-missing
check '+none\r\n' TYPE nothing
check '+OK\r\n' SET doomed x
check ':0\r\n' BITOP AND doomed missing
check '+none\r\n' TYPE doomed

echo "== stage test: BITOP OR keeps the bits set in any source"
check ':1\r\n' BITOP OR or-dest key1 key2
check ':1\r\n' GETBIT or-dest 0
check ':1\r\n' GETBIT or-dest 4
check ':1\r\n' GETBIT or-dest 6
check ':3\r\n' BITCOUNT or-dest
check ':0\r\n' SETBIT key3 1 1
check ':0\r\n' SETBIT key3 10 1
check ':0\r\n' SETBIT key4 1 1
check ':2\r\n' BITOP OR dest5 key3 key4
check ':1\r\n' GETBIT dest5 1
check ':1\r\n' GETBIT dest5 10
check ':2\r\n' BITCOUNT dest5

echo "== XOR and NOT"
check ':1\r\n' BITOP XOR xor-dest key1 key2
check ':0\r\n' GETBIT xor-dest 0
check ':1\r\n' GETBIT xor-dest 4
check ':1\r\n' GETBIT xor-dest 6
check ':1\r\n' BITOP NOT not-dest key1
check ':6\r\n' BITCOUNT not-dest
check ':0\r\n' GETBIT not-dest 0
check '-ERR BITOP NOT must be called with a single source key.\r\n' BITOP NOT not-dest key1 key2

echo "== BITOP errors"
check '-ERR syntax error\r\n' BITOP NAND dest key1 key2
check ':1\r\n' RPUSH bitop-list x
check "$wrongtype" BITOP AND dest key1 bitop-list
check "-ERR wrong number of arguments for 'bitop' command\\r\\n" BITOP AND dest

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
