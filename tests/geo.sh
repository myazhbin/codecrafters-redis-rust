#!/bin/bash
# Geospatial commands, from the "Respond to GEOADD" stage on.
source "$(dirname "$0")/helpers.sh"

echo "== stage test: GEOADD replies with how many locations it added"
check ':1\r\n' GEOADD places 11.5030378 48.164271 Munich
check ':0\r\n' GEOADD places 11.5030378 48.164271 Munich
check ':2\r\n' GEOADD places 2.2944692 48.8584625 Paris -0.0884948 51.506479 London
check '+zset\r\n' TYPE places

echo "== arity and syntax"
check "-ERR wrong number of arguments for 'geoadd' command\\r\\n" GEOADD places 1 2
check '-ERR syntax error. Try GEOADD key [x1] [y1] [name1] [x2] [y2] [name2] ... \r\n' GEOADD places 1 2 a 3
check '-ERR value is not a valid float\r\n' GEOADD places abc 2 a
check '+OK\r\n' SET text v
check "$wrongtype" GEOADD text 1 2 a
check ':1\r\n' geoadd lower 1 2 a

finish
