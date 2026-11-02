#!/usr/bin/env python3
"""Processing the replication stream as a replica, from the "Command processing" and ACK
stages on, with the test playing the master."""

import time

from helpers import (OK, RDB_TRANSFER, answer_handshake, bulk, check, command, encode, expect, finish,
                     fullresync, replica_of_test, stays_quiet)

GETACK = ("REPLCONF", "GETACK", "*")


def ack(offset):
    return encode("REPLCONF", "ACK", str(offset))


def expect_ack(master, offset):
    """Sends GETACK and checks the ACK that comes back."""
    master.send(*GETACK)
    expect(master, ack(offset), f"REPLCONF ACK {offset}")


with replica_of_test() as master:
    answer_handshake(master)
    master.sock.sendall(fullresync() + RDB_TRANSFER)

    print("== stage test: propagated commands are processed, with no replies to the master")
    master.send("SET", "foo", "1")
    master.send("SET", "bar", "2")
    master.send("SET", "baz", "3")
    check("the master receives nothing back", stays_quiet(master))
    command(bulk("1"), "GET", "foo")
    command(bulk("2"), "GET", "bar")
    command(bulk("3"), "GET", "baz")

with replica_of_test() as master:
    answer_handshake(master)
    master.sock.sendall(fullresync() + RDB_TRANSFER)

    print("== stage tests: GETACK gets the offset up to, but not including, itself")
    master.send(*GETACK)
    expect(master, b"*3\r\n$8\r\nREPLCONF\r\n$3\r\nACK\r\n$1\r\n0\r\n", "REPLCONF ACK 0")
    master.send("PING")
    check("PING gets no reply", stays_quiet(master))
    expect_ack(master, 51)
    master.send("SET", "foo", "1")
    master.send("SET", "bar", "2")
    check("SET gets no reply", stays_quiet(master))
    expect_ack(master, 146)
    command(bulk("1"), "GET", "foo")
    command(bulk("2"), "GET", "bar")

    print("== GETACK in any case, as masters send it")
    master.send("replconf", "getack", "*")
    expect(master, ack(183), "REPLCONF ACK 183")

    print("== INFO has the offset too")
    check("no more replies", stays_quiet(master))
    command(
        bulk(
            "role:slave\r\n"
            "master_replid:8371b4fb1155b71f4a04d3e1bc3e18c4a990aeeb\r\n"
            "master_repl_offset:220\r\n"
        ),
        "INFO",
        "replication",
    )

with replica_of_test() as master:
    print("== everything from FULLRESYNC on can arrive in one read")
    answer_handshake(master)
    sets = encode("SET", "a", "1") + encode("SET", "b", "2")
    master.sock.sendall(fullresync() + RDB_TRANSFER + sets + encode(*GETACK))
    expect(master, ack(len(sets)), "the ACK counts both SETs")
    command(bulk("1"), "GET", "a")
    command(bulk("2"), "GET", "b")

with replica_of_test() as master:
    print("== or in pieces")
    answer_handshake(master)
    a_set = encode("SET", "a", "1")
    stream = fullresync() + RDB_TRANSFER + a_set + encode(*GETACK)
    for start in range(0, len(stream), 7):
        master.sock.sendall(stream[start:start + 7])
        time.sleep(0.005)
    expect(master, ack(len(a_set)), "the ACK counts the SET")
    command(bulk("1"), "GET", "a")

with replica_of_test() as master:
    print("== the offset starts from the one in FULLRESYNC")
    answer_handshake(master)
    master.sock.sendall(fullresync(1000) + RDB_TRANSFER)
    expect_ack(master, 1000)
    expect_ack(master, 1037)

with replica_of_test() as master:
    answer_handshake(master)
    master.sock.sendall(fullresync() + RDB_TRANSFER)

    print("== failed commands get no reply either, and still count towards the offset")
    failing = (("SET", "s", "abc"), ("INCR", "s"), ("NOSUCHCOMMAND",))
    for args in failing:
        master.send(*args)
    check("the master receives nothing back", stays_quiet(master))
    offset = sum(len(encode(*args)) for args in failing)
    expect_ack(master, offset)
    offset += len(encode(*GETACK))

    print("== a transaction from the master runs as a client's would, with no replies")
    transaction = (("MULTI",), ("SET", "t", "1"), ("INCR", "t"), ("EXEC",))
    for args in transaction:
        master.send(*args)
    check("the master receives nothing back", stays_quiet(master))
    offset += sum(len(encode(*args)) for args in transaction)
    expect_ack(master, offset)
    command(bulk("2"), "GET", "t")

    print("== a replica can't have replicas of its own")
    command(b"-ERR a replica can't have replicas of its own\r\n", "PSYNC", "?", "-1")

    print("== clients can still use it after the master hangs up")
    master.close()
    time.sleep(0.1)
    command(bulk("2"), "GET", "t")
    command(OK, "SET", "local", "1")

finish()
