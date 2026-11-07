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

echo "== stage test: adding members to a set, and updating one's score"
check ':1\r\n' ZADD added 20.0 zset_member1
check ':1\r\n' ZADD added 30.1 zset_member2
check ':1\r\n' ZADD added 40.2 zset_member3
check ':1\r\n' ZADD added 50.3 zset_member4
check ':0\r\n' ZADD added 100.0 zset_member1
check ':0\r\n' ZADD added 30.1 zset_member2

echo "== new and existing members in one ZADD; only the new ones count"
check ':2\r\n' ZADD added 1 zset_member1 2 new1 3 new2 4 zset_member3

echo "== stage test: ZRANK gives a member's rank, by score and then by name"
check ':1\r\n' ZADD ranked 100.0 foo
check ':1\r\n' ZADD ranked 100.0 bar
check ':1\r\n' ZADD ranked 20.0 baz
check ':1\r\n' ZADD ranked 30.1 caz
check ':1\r\n' ZADD ranked 40.2 paz
check ':1\r\n' ZRANK ranked caz
check ':0\r\n' ZRANK ranked baz
check ':4\r\n' ZRANK ranked foo
check ':3\r\n' ZRANK ranked bar
check ':2\r\n' ZRANK ranked paz
check '$-1\r\n' ZRANK ranked missing_member
check '$-1\r\n' ZRANK missing_key member

echo "== an updated score changes the rank"
check ':0\r\n' ZADD ranked 1 foo
check ':0\r\n' ZRANK ranked foo
check ':1\r\n' ZRANK ranked baz

echo "== ZRANK WITHSCORE, errors"
check '*2\r\n:1\r\n$2\r\n20\r\n' ZRANK ranked baz WITHSCORE
check '*2\r\n:3\r\n$4\r\n40.2\r\n' ZRANK ranked paz withscore
check '*-1\r\n' ZRANK ranked missing WITHSCORE
check '-ERR syntax error\r\n' ZRANK ranked baz BOGUS
check '+OK\r\n' SET a-string v
check "$wrongtype" ZRANK a-string member
check "-ERR wrong number of arguments for 'zrank' command\\r\\n" ZRANK ranked

echo "== errors, which leave nothing behind"
check '-ERR value is not a valid float\r\n' ZADD added 1 new3 x new4
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
