#!/usr/bin/env python3
"""Propagating writes as a master, from the "Single-replica propagation" stage on, with the
test playing the replicas."""

import time

from helpers import (OK, QUEUED, WRONGTYPE, Client, bulk, command, connect_replica, encode, expect,
                     expect_nothing, finish, run, running_server)

NOT_AN_INT = b"-ERR value is not an integer or out of range\r\n"

# The bytes propagated so far, which is the master's replication offset.
propagated = 0


def expect_propagated(replicas, *commands):
    """Checks that each of `replicas` receives these commands next, as RESP arrays."""
    global propagated
    want = b"".join(encode(*args) for args in commands)
    label = ", ".join(" ".join(args) for args in commands)
    for replica in replicas:
        expect(replica, want, f"a replica receives {label}")
    propagated += len(want)


with running_server():
    print("== a write with no replicas connected doesn't count towards the offset")
    command(OK, "SET", "early", "1")
    replica = connect_replica(offset=0)
    expect_nothing(replica, "a new replica isn't sent writes from before it connected")

    print("== stage test: writes reach a replica in order, on the handshake's connection")
    writes = [("SET", "foo", "1"), ("SET", "bar", "2"), ("SET", "baz", "3")]
    for write in writes:
        command(OK, *write)
    expect_propagated([replica], *writes)

    print("== stage test: writes reach every replica")
    replicas = [replica] + [connect_replica(offset=propagated) for _ in range(2)]
    writes = [("SET", "foo", "4"), ("SET", "bar", "5"), ("SET", "baz", "6")]
    for write in writes:
        command(OK, *write)
    expect_propagated(replicas, *writes)

    print("== commands that only read aren't propagated")
    command(b"+PONG\r\n", "PING")
    command(bulk("hey"), "ECHO", "hey")
    command(bulk("4"), "GET", "foo")
    command(b"+string\r\n", "TYPE", "foo")
    command(OK, "REPLCONF", "capa", "psync2")
    expect_nothing(replica, "the replica receives nothing")

    print("== nor are writes that fail or change nothing")
    command(OK, "SET", "s", "abc")
    command(b":1\r\n", "RPUSH", "one", "a")
    expect_propagated(replicas, ("SET", "s", "abc"), ("RPUSH", "one", "a"))
    command(NOT_AN_INT, "INCR", "s")
    command(WRONGTYPE, "RPUSH", "s", "x")
    command(b"-ERR wrong number of arguments for 'set' command\r\n", "SET", "only_a_key")
    command(b"$-1\r\n", "LPOP", "missing")
    command(b"*-1\r\n", "LPOP", "missing", "2")
    command(b"*0\r\n", "LPOP", "one", "0")
    expect_nothing(replica, "the replica receives nothing")

    print("== every other write goes out as it came, in any case")
    command(b":2\r\n", "RPUSH", "list", "a", "b")
    command(b":3\r\n", "LPUSH", "list", "c")
    command(bulk("c"), "LPOP", "list")
    command(encode("a"), "LPOP", "list", "1")
    command(b":1\r\n", "INCR", "n")
    command(OK, "set", "lower", "case")
    command(OK, "SET", "temp", "v", "PX", "100000")
    expect_propagated(
        replicas,
        ("RPUSH", "list", "a", "b"),
        ("LPUSH", "list", "c"),
        ("LPOP", "list"),
        ("LPOP", "list", "1"),
        ("INCR", "n"),
        ("set", "lower", "case"),
        ("SET", "temp", "v", "PX", "100000"),
    )

    print("== XADD goes out with the ID it used, even one it generated")
    command(bulk("1-1"), "XADD", "stream", "1-1", "f", "v")
    client = Client()
    client.send("XADD", "stream", "*", "f", "v")
    generated = client.read(64, 0.2).split(b"\r\n")[1].decode()
    expect_propagated(
        replicas,
        ("XADD", "stream", "1-1", "f", "v"),
        ("XADD", "stream", generated, "f", "v"),
    )

    print("== BLPOP goes out as LPOP, so that a replica never waits")
    command(b":1\r\n", "RPUSH", "ready", "a")
    command(encode("ready", "a"), "BLPOP", "ready", "0")
    expect_propagated(replicas, ("RPUSH", "ready", "a"), ("LPOP", "ready"))
    blocked = Client()
    blocked.send("BLPOP", "waiting", "0")
    expect_nothing(replica, "a BLPOP that's waiting hasn't popped anything")
    command(b":1\r\n", "RPUSH", "waiting", "x")
    expect(blocked, encode("waiting", "x"), "the blocked client gets x")
    expect_propagated(replicas, ("RPUSH", "waiting", "x"), ("LPOP", "waiting"))
    command(b"*-1\r\n", "BLPOP", "nothing", "0.1")
    expect_nothing(replica, "a BLPOP that times out isn't propagated")

    print("== a transaction's writes go out when EXEC runs it, in order")
    transaction = Client()
    run(transaction, OK, "MULTI")
    run(transaction, QUEUED, "SET", "t", "1")
    run(transaction, QUEUED, "INCR", "t")
    run(transaction, QUEUED, "GET", "t")
    expect_nothing(replica, "nothing goes out while they're queued")
    run(transaction, b"*3\r\n+OK\r\n:2\r\n$1\r\n2\r\n", "EXEC")
    expect_propagated(replicas, ("SET", "t", "1"), ("INCR", "t"))
    run(transaction, OK, "MULTI")
    run(transaction, QUEUED, "SET", "t", "9")
    run(transaction, OK, "DISCARD")
    expect_nothing(replica, "nothing goes out from a discarded transaction")
    command(b":1\r\n", "RPUSH", "queue", "a")
    run(transaction, OK, "MULTI")
    run(transaction, QUEUED, "BLPOP", "queue", "0")
    run(transaction, b"*1\r\n" + encode("queue", "a"), "EXEC")
    expect_propagated(replicas, ("RPUSH", "queue", "a"), ("LPOP", "queue"))

    print("== a client that isn't a replica doesn't receive writes")
    other = Client()
    command(OK, "SET", "only", "replicas")
    expect_propagated(replicas, ("SET", "only", "replicas"))
    expect_nothing(other, "the client receives nothing")

    print("== a replica that hangs up is forgotten, and the others carry on")
    replicas.pop().close()
    time.sleep(0.1)
    command(OK, "SET", "after", "1")
    command(OK, "SET", "after", "2")
    expect_propagated(replicas, ("SET", "after", "1"), ("SET", "after", "2"))

    print("== a replica that connects later only receives what comes after")
    late = connect_replica(offset=propagated)
    expect_nothing(late, "it isn't sent the writes from before it connected")
    command(OK, "SET", "late", "1")
    expect_propagated([*replicas, late], ("SET", "late", "1"))

finish()
