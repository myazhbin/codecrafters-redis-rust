#!/bin/bash
# Geospatial commands, from the "Respond to GEOADD" stage on.
source "$(dirname "$0")/helpers.sh"

echo "== stage test: GEOADD replies with how many locations it added"
check ':1\r\n' GEOADD places 11.5030378 48.164271 Munich
check ':0\r\n' GEOADD places 11.5030378 48.164271 Munich
check ':2\r\n' GEOADD places 2.2944692 48.8584625 Paris -0.0884948 51.506479 London
check '+zset\r\n' TYPE places

echo "== stage test: coordinates out of range are refused"
check '-ERR invalid longitude,latitude pair 200.000000,100.000000\r\n' GEOADD location_key 200 100 foo
check '-ERR invalid longitude,latitude pair 180.000000,90.000000\r\n' GEOADD location_key 180 90 test1
check '-ERR invalid longitude,latitude pair 181.000000,0.300000\r\n' GEOADD location_key 181 0.3 test2
check '-ERR invalid longitude,latitude pair -180.100000,0.000000\r\n' GEOADD location_key -180.1 0 test3
check '-ERR invalid longitude,latitude pair 0.000000,-85.051129\r\n' GEOADD location_key 0 -85.0511288 test4
check '+none\r\n' TYPE location_key

echo "== the limits themselves are fine"
check ':4\r\n' GEOADD edges 180 85.05112878 a -180 -85.05112878 b 0 0 c -0.5 85 d

echo "== one bad location stops the whole GEOADD"
check '-ERR invalid longitude,latitude pair 0.000000,90.000000\r\n' GEOADD edges 1 1 e 0 90 f
check ':4\r\n' ZCARD edges

echo "== stage test: a location is stored in a sorted set"
check ':1\r\n' GEOADD stored 2.2944692 48.8584625 Paris
check "$(array Paris)" ZRANGE stored 0 -1
check ':1\r\n' ZCARD stored

echo "== locations and ZADD members mix in the same sorted set"
check ':1\r\n' ZADD stored 1 plain
check "$(array plain Paris)" ZRANGE stored 0 -1
check ':1\r\n' ZREM stored Paris
check "$(array plain)" ZRANGE stored 0 -1

echo "== stage test: the score is the location's geohash"
check ':1\r\n' GEOADD scored 2.2944692 48.8584625 Paris
check ':1\r\n' GEOADD scored -0.127758 51.507351 London
check '$16\r\n3663832614298053\r\n' ZSCORE scored Paris
check '$16\r\n2163557714754256\r\n' ZSCORE scored London

echo "== more places, from the reference implementation"
check ':4\r\n' GEOADD scored 11.5030378 48.164271 Munich 100.5252 13.722 Bangkok 151.2093 -33.8688 Sydney -74.006 40.7128 NewYork
check '$16\r\n3672376881541190\r\n' ZSCORE scored Munich
check '$16\r\n3962257306574459\r\n' ZSCORE scored Bangkok
check '$16\r\n3252046221964352\r\n' ZSCORE scored Sydney
check '$16\r\n1791873974549446\r\n' ZSCORE scored NewYork
check "$(array NewYork London Sydney Paris Munich Bangkok)" ZRANGE scored 0 -1

echo "== stage test: GEOPOS gives a position for each location, or a null array"
check ':1\r\n' GEOADD location_key -0.0884948 51.506479 London
check ':1\r\n' GEOADD location_key 11.5030378 48.164271 Munich
check "*2\\r\\n$(array -0.08849412202835083 51.50647814139934)$(array 11.503036916255951 48.16427086232977)" \
  GEOPOS location_key London Munich
check '*1\r\n*-1\r\n' GEOPOS location_key missing_location
check '*2\r\n*-1\r\n*-1\r\n' GEOPOS missing_key London Munich
check "*3\\r\\n*-1\\r\\n$(array -0.08849412202835083 51.50647814139934)*-1\\r\\n" \
  GEOPOS location_key nowhere London nowhere
check '*0\r\n' GEOPOS location_key
check '+OK\r\n' SET a-string v
check "$wrongtype" GEOPOS a-string London
check "-ERR wrong number of arguments for 'geopos' command\\r\\n" GEOPOS

echo "== arity and syntax"
check "-ERR wrong number of arguments for 'geoadd' command\\r\\n" GEOADD places 1 2
check '-ERR syntax error. Try GEOADD key [x1] [y1] [name1] [x2] [y2] [name2] ... \r\n' GEOADD places 1 2 a 3
check '-ERR value is not a valid float\r\n' GEOADD places abc 2 a
check '+OK\r\n' SET text v
check "$wrongtype" GEOADD text 1 2 a
check ':1\r\n' geoadd lower 1 2 a

finish
