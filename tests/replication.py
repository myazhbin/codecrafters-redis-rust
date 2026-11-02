#!/usr/bin/env python3
"""Replication from start to finish, with a real master and two real replicas."""

import socket
import time

from helpers import (OK, OTHER_PORT, PORT, QUEUED, Client, bulk, check, command, encode, entry, expect,
                     finish, run, running_server)

AS_REPLICA = ("--replicaof", f"localhost {PORT}")
REPLICA_PORTS = (OTHER_PORT, OTHER_PORT + 100)


def client_on(port):
    return Client(socket.create_connection(("127.0.0.1", port)))


def on_replicas(want, *args):
    """Runs a command on each replica and checks the reply."""
    for port in REPLICA_PORTS:
        run(client_on(port), want, *args)


def offset_on(port):
    """The master_repl_offset line of the server on `port`."""
    client = client_on(port)
    client.send("INFO", "replication")
    return client.read(200, 0.1).split(b"\r\n")[3]


with running_server(), running_server(*AS_REPLICA, port=REPLICA_PORTS[0]), running_server(*AS_REPLICA, port=REPLICA_PORTS[1]):
    print("== both replicas get through the handshake")
    command(b":2\r\n", "WAIT", "2", "2000")

    print("== writes of every kind go to the master")
    command(OK, "SET", "foo", "1")
    command(b":2\r\n", "RPUSH", "list", "a", "b")
    command(b":3\r\n", "LPUSH", "list", "c")
    command(bulk("c"), "LPOP", "list")
    command(encode("list", "a"), "BLPOP", "list", "0")
    blocked = Client()
    blocked.send("BLPOP", "waiting", "0")
    time.sleep(0.1)
    command(b":1\r\n", "RPUSH", "waiting", "x")
    expect(blocked, encode("waiting", "x"), "the blocked client gets x")
    command(b":1\r\n", "INCR", "n")
    transaction = Client()
    run(transaction, OK, "MULTI")
    run(transaction, QUEUED, "SET", "t", "1")
    run(transaction, QUEUED, "INCR", "t")
    run(transaction, b"*2\r\n+OK\r\n:2\r\n", "EXEC")
    client = Client()
    client.send("XADD", "stream", "*", "f", "v")
    generated = client.read(64, 0.2).split(b"\r\n")[1].decode()

    print("== WAIT says both replicas have acknowledged them")
    command(b":2\r\n", "WAIT", "2", "2000")

    print("== and they have them all")
    on_replicas(bulk("1"), "GET", "foo")
    on_replicas(encode("b"), "LRANGE", "list", "0", "-1")
    on_replicas(b":0\r\n", "LLEN", "waiting")
    on_replicas(bulk("1"), "GET", "n")
    on_replicas(bulk("2"), "GET", "t")
    on_replicas(b"*1\r\n" + entry(generated, "f", "v"), "XRANGE", "stream", "-", "+")

    print("== they carry on from there, after a GETACK in the stream")
    command(OK, "SET", "foo", "2")
    command(b":2\r\n", "WAIT", "2", "2000")
    on_replicas(bulk("2"), "GET", "foo")

    print("== with no new writes, WAIT needs nothing more from them")
    start = time.monotonic()
    command(b":2\r\n", "WAIT", "2", "2000")
    elapsed = time.monotonic() - start
    check(f"it replied at once ({elapsed:.2f}s)", elapsed < 0.4)

    print("== the master and replicas agree on how far the stream has got")
    time.sleep(0.1)
    master_offset = offset_on(PORT)
    check(f"the master's offset has moved on ({master_offset.decode()})", master_offset != b"master_repl_offset:0")
    for port in REPLICA_PORTS:
        got = offset_on(port)
        check(f"a replica has the same offset ({got.decode()})", got == master_offset)

finish()
