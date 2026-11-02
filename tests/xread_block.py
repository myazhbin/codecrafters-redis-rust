#!/usr/bin/env python3
"""XREAD BLOCK, from the "Blocking reads" stages on, using several client connections."""

import time

from helpers import (WRONGTYPE, Client, check, command, encode, entry, expect,
                     expect_nothing, finish, running_server, streams)

with running_server():
    print("== stage test")
    command(b"$3\r\n0-1\r\n", "XADD", "stream_key", "0-1", "temperature", "96")
    reader = Client()
    reader.send("XREAD", "BLOCK", "1000", "streams", "stream_key", "0-1")
    expect_nothing(reader, "XREAD BLOCK 1000 waits for a new entry", wait=0.2)
    command(b"$3\r\n0-2\r\n", "XADD", "stream_key", "0-2", "temperature", "95")
    expect(reader, streams(("stream_key", [entry("0-2", "temperature", "95")])),
           "the waiting XREAD receives the new entry")
    reader = Client()
    start = time.monotonic()
    reader.send("XREAD", "block", "1000", "streams", "stream_key", "0-2")
    expect(reader, b"*-1\r\n", "XREAD block 1000 with nothing new gives a null array",
           timeout=2.0)
    elapsed = time.monotonic() - start
    check(f"after waiting about 1s ({elapsed:.2f}s)", 0.95 <= elapsed < 1.5)

    print("== entries that are already there come back at once")
    reader = Client()
    start = time.monotonic()
    reader.send("XREAD", "BLOCK", "1000", "STREAMS", "stream_key", "0-0")
    expect(reader, streams(("stream_key", [entry("0-1", "temperature", "96"),
                                           entry("0-2", "temperature", "95")])),
           "XREAD BLOCK 1000 STREAMS stream_key 0-0", timeout=0.5)
    elapsed = time.monotonic() - start
    check(f"without waiting ({elapsed:.2f}s)", elapsed < 0.2)

    print("== BLOCK 0 waits forever")
    reader = Client()
    reader.send("XREAD", "BLOCK", "0", "STREAMS", "stream_key", "0-2")
    expect_nothing(reader, "XREAD BLOCK 0 is still waiting after 1.2s", wait=1.2)
    command(b"$3\r\n0-3\r\n", "XADD", "stream_key", "0-3", "temperature", "94")
    expect(reader, streams(("stream_key", [entry("0-3", "temperature", "94")])),
           "it receives the new entry")

    print("== $ means entries added from now on")
    reader = Client()
    reader.send("XREAD", "BLOCK", "0", "STREAMS", "stream_key", "$")
    expect_nothing(reader, "XREAD BLOCK 0 STREAMS stream_key $ waits")
    command(b"$3\r\n0-4\r\n", "XADD", "stream_key", "0-4", "temperature", "93")
    expect(reader, streams(("stream_key", [entry("0-4", "temperature", "93")])),
           "it receives only the new entry")

    print("== every waiting reader gets the new entry")
    readers = [Client(), Client()]
    for reader in readers:
        reader.send("XREAD", "BLOCK", "0", "STREAMS", "stream_key", "$")
    time.sleep(0.2)
    command(b"$3\r\n0-5\r\n", "XADD", "stream_key", "0-5", "temperature", "92")
    for number, reader in enumerate(readers, 1):
        expect(reader, streams(("stream_key", [entry("0-5", "temperature", "92")])),
               f"reader {number} receives it")

    print("== several streams")
    reader = Client()
    reader.send("XREAD", "BLOCK", "0", "STREAMS", "s1", "s2", "$", "$")
    expect_nothing(reader, "XREAD BLOCK 0 on two missing streams waits")
    command(b"$3\r\n1-1\r\n", "XADD", "s2", "1-1", "a", "b")
    expect(reader, streams(("s2", [entry("1-1", "a", "b")])),
           "an entry on the second stream ends the wait")

    print("== entries up to the given ID don't count")
    reader = Client()
    reader.send("XREAD", "BLOCK", "2000", "STREAMS", "later", "100-0")
    time.sleep(0.1)
    command(b"$4\r\n50-0\r\n", "XADD", "later", "50-0", "a", "b")
    expect_nothing(reader, "an entry before 100-0 doesn't end the wait")
    command(b"$5\r\n150-0\r\n", "XADD", "later", "150-0", "c", "d")
    expect(reader, streams(("later", [entry("150-0", "c", "d")])), "an entry after 100-0 does")

    print("== entries on other streams don't count")
    reader = Client()
    start = time.monotonic()
    reader.send("XREAD", "BLOCK", "500", "STREAMS", "lonely", "$")
    time.sleep(0.1)
    command(b"$3\r\n1-1\r\n", "XADD", "other", "1-1", "a", "b")
    expect(reader, b"*-1\r\n", "XREAD BLOCK 500 on lonely still times out", timeout=2.0)
    elapsed = time.monotonic() - start
    check(f"after about 0.5s ({elapsed:.2f}s)", 0.45 <= elapsed < 1.0)

    print("== a waiting reader that hangs up")
    gone = Client()
    gone.send("XREAD", "BLOCK", "0", "STREAMS", "stream_key", "$")
    time.sleep(0.1)
    gone.close()
    time.sleep(0.1)
    command(b"$3\r\n0-6\r\n", "XADD", "stream_key", "0-6", "temperature", "91")
    command(b"+PONG\r\n", "PING")

    print("== errors")
    command(b"-ERR timeout is not an integer or out of range\r\n",
            "XREAD", "BLOCK", "abc", "STREAMS", "stream_key", "0")
    command(b"-ERR timeout is negative\r\n", "XREAD", "BLOCK", "-1", "STREAMS", "stream_key", "0")
    command(b"-ERR syntax error\r\n", "XREAD", "BLOCK", "100", "STREAMS")
    command(b"-ERR syntax error\r\n", "XREAD", "COUNT", "1", "STREAMS", "stream_key", "0")
    command(b"-ERR wrong number of arguments for 'xread' command\r\n", "XREAD", "BLOCK", "100")
    command(b"+OK\r\n", "SET", "str", "val")
    command(WRONGTYPE, "XREAD", "BLOCK", "0", "STREAMS", "str", "0")

    print("== non-blocking XREAD and BLPOP still work")
    command(streams(("stream_key", [entry("0-6", "temperature", "91")])),
            "XREAD", "STREAMS", "stream_key", "0-5")
    command(b"*-1\r\n", "XREAD", "STREAMS", "stream_key", "$")
    popper = Client()
    popper.send("BLPOP", "list", "0")
    time.sleep(0.1)
    command(b":1\r\n", "RPUSH", "list", "x")
    expect(popper, encode("list", "x"), "a waiting BLPOP is served by RPUSH")

finish()
