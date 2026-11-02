#!/usr/bin/env python3
"""WATCH and UNWATCH, from the "Optimistic locking" stages on, using several clients."""

import time

from helpers import (OK, QUEUED, WRONGTYPE, Client, command, encode, expect, finish, run,
                     running_server)

ABORTED = b"*-1\r\n"

with running_server():
    print("== stage test")
    command(OK, "WATCH", "key")

    print("== WATCH isn't allowed inside a transaction, and needs a key")
    a, b = Client(), Client()
    run(a, OK, "MULTI")
    run(a, b"-ERR WATCH inside MULTI is not allowed\r\n", "WATCH", "key")
    run(a, b"*0\r\n", "EXEC")
    command(b"-ERR wrong number of arguments for 'watch' command\r\n", "WATCH")

    print("== the stage description's example: another client's change calls EXEC off")
    command(OK, "SET", "counter", "10")
    run(a, OK, "WATCH", "counter")
    run(a, b"$2\r\n10\r\n", "GET", "counter")
    run(b, OK, "SET", "counter", "20")
    run(a, OK, "MULTI")
    run(a, QUEUED, "SET", "counter", "11")
    run(a, ABORTED, "EXEC")
    command(b"$2\r\n20\r\n", "GET", "counter")

    print("== EXEC goes ahead when the watched key hasn't changed")
    run(a, OK, "WATCH", "counter")
    run(b, b"$2\r\n20\r\n", "GET", "counter")
    run(a, OK, "MULTI")
    run(a, QUEUED, "INCR", "counter")
    run(a, b"*1\r\n:21\r\n", "EXEC")

    print("== EXEC stops watching, whether or not it went ahead")
    run(b, OK, "SET", "counter", "0")
    run(a, OK, "MULTI")
    run(a, QUEUED, "INCR", "counter")
    run(a, b"*1\r\n:1\r\n", "EXEC")

    print("== several keys: a change to any of them calls EXEC off")
    run(a, OK, "WATCH", "k1", "k2", "k3")
    run(b, b":1\r\n", "INCR", "k2")
    run(a, OK, "MULTI")
    run(a, QUEUED, "SET", "k1", "x")
    run(a, ABORTED, "EXEC")
    command(b"$-1\r\n", "GET", "k1")
    run(a, OK, "WATCH", "k1")
    run(a, OK, "WATCH", "k2")
    run(b, OK, "SET", "k1", "y")
    run(a, OK, "MULTI")
    run(a, ABORTED, "EXEC")

    print("== watching keys that don't exist yet")
    run(a, OK, "WATCH", "newkey")
    run(b, OK, "SET", "newkey", "v")
    run(a, OK, "MULTI")
    run(a, QUEUED, "GET", "newkey")
    run(a, ABORTED, "EXEC")
    run(a, OK, "WATCH", "stillmissing")
    run(a, OK, "MULTI")
    run(a, QUEUED, "SET", "stillmissing", "1")
    run(a, b"*1\r\n+OK\r\n", "EXEC")

    print("== UNWATCH")
    run(a, OK, "WATCH", "u")
    run(a, OK, "UNWATCH")
    run(b, OK, "SET", "u", "changed")
    run(a, OK, "MULTI")
    run(a, QUEUED, "GET", "u")
    run(a, b"*1\r\n$7\r\nchanged\r\n", "EXEC")
    command(OK, "UNWATCH")
    command(b"-ERR wrong number of arguments for 'unwatch' command\r\n", "UNWATCH", "x")
    run(a, OK, "WATCH", "u")
    run(a, OK, "MULTI")
    run(a, QUEUED, "UNWATCH")
    run(a, b"*1\r\n+OK\r\n", "EXEC")

    print("== DISCARD stops watching too")
    run(a, OK, "WATCH", "d")
    run(a, OK, "MULTI")
    run(a, OK, "DISCARD")
    run(b, OK, "SET", "d", "1")
    run(a, OK, "MULTI")
    run(a, b"*0\r\n", "EXEC")

    print("== every kind of write is a change")
    command(b":1\r\n", "RPUSH", "w-list", "a")
    command(OK, "SET", "w-count", "1")
    for write, reply in [
        (["LPUSH", "w-list", "b"], b":2\r\n"),
        (["RPUSH", "w-list", "c"], b":3\r\n"),
        (["LPOP", "w-list"], b"$1\r\nb\r\n"),
        (["LPOP", "w-list", "2"], encode("a", "c")),
        (["INCR", "w-count"], b":2\r\n"),
        (["SET", "w-count", "x"], OK),
        (["XADD", "w-stream", "1-1", "f", "v"], b"$3\r\n1-1\r\n"),
    ]:
        run(a, OK, "WATCH", write[1])
        run(b, reply, *write)
        run(a, OK, "MULTI")
        run(a, ABORTED, "EXEC")

    print("== so is the watching client's own write, and a push a waiting BLPOP takes")
    run(a, OK, "WATCH", "own")
    run(a, OK, "SET", "own", "1")
    run(a, OK, "MULTI")
    run(a, ABORTED, "EXEC")
    popper = Client()
    popper.send("BLPOP", "queue", "0")
    time.sleep(0.1)
    run(a, OK, "WATCH", "queue")
    run(b, b":1\r\n", "RPUSH", "queue", "job")
    expect(popper, encode("queue", "job"), "the waiting BLPOP gets the job")
    run(a, OK, "MULTI")
    run(a, ABORTED, "EXEC")

    print("== a key expiring is a change, unless it had already expired when watched")
    command(OK, "SET", "fleeting", "v", "PX", "100")
    run(a, OK, "WATCH", "fleeting")
    time.sleep(0.2)
    run(a, OK, "MULTI")
    run(a, ABORTED, "EXEC")
    command(OK, "SET", "gone", "v", "PX", "100")
    time.sleep(0.2)
    run(a, OK, "WATCH", "gone")
    run(a, OK, "MULTI")
    run(a, b"*0\r\n", "EXEC")

    print("== reads, failed writes and LPOP of 0 elements aren't changes")
    command(OK, "SET", "s", "abc")
    run(a, OK, "WATCH", "s")
    run(b, b"$3\r\nabc\r\n", "GET", "s")
    run(b, b"-ERR value is not an integer or out of range\r\n", "INCR", "s")
    run(b, WRONGTYPE, "RPUSH", "s", "x")
    run(a, OK, "MULTI")
    run(a, b"*0\r\n", "EXEC")
    command(b":1\r\n", "RPUSH", "l0", "a")
    run(a, OK, "WATCH", "l0")
    run(b, b"*0\r\n", "LPOP", "l0", "0")
    run(a, OK, "MULTI")
    run(a, b"*0\r\n", "EXEC")

finish()
