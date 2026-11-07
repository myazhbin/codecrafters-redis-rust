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

echo "== arity and syntax"
check "-ERR wrong number of arguments for 'geoadd' command\\r\\n" GEOADD places 1 2
check '-ERR syntax error. Try GEOADD key [x1] [y1] [name1] [x2] [y2] [name2] ... \r\n' GEOADD places 1 2 a 3
check '-ERR value is not a valid float\r\n' GEOADD places abc 2 a
check '+OK\r\n' SET text v
check "$wrongtype" GEOADD text 1 2 a
check ':1\r\n' geoadd lower 1 2 a

finish
