#!/usr/bin/env python3
"""WAIT, from the "WAIT with no replicas" stage on, with the test playing the replicas."""

import time

from helpers import (OK, QUEUED, Client, check, command, connect_replica, encode, expect,
                     expect_nothing, finish, run, running_server)

GETACK = encode("REPLCONF", "GETACK", "*")
SET_LENGTH = len(encode("SET", "foo", "123"))


def at_once(want, *args):
    """Runs a command that shouldn't need its timeout of 500ms or more."""
    start = time.monotonic()
    command(want, *args)
    elapsed = time.monotonic() - start
    check(f"it replied at once ({elapsed:.2f}s)", elapsed < 0.4)


def after(seconds, want, *args):
    """Runs a command that should take `seconds`."""
    start = time.monotonic()
    command(want, *args)
    elapsed = time.monotonic() - start
    check(f"it waited about {seconds}s ({elapsed:.2f}s)", seconds * 0.9 <= elapsed < seconds + 0.5)


def write_needing_ack(replicas, waiter, write, *wait_args):
    """Sends a write, then a WAIT that should ask for ACKs, and checks the replicas get both."""
    command(OK, *write)
    waiter.send("WAIT", *wait_args)
    for replica in replicas:
        expect(replica, encode(*write) + GETACK, "the replica receives the write, then GETACK")


with running_server():
    print("== stage test: with no replicas, WAIT 0 replies at once")
    at_once(b":0\r\n", "WAIT", "0", "60000")

    print("== wanting replicas that aren't there, it waits out the timeout")
    after(0.1, b":0\r\n", "WAIT", "1", "100")

    print("== errors")
    command(b"-ERR wrong number of arguments for 'wait' command\r\n", "WAIT")
    command(b"-ERR wrong number of arguments for 'wait' command\r\n", "WAIT", "1")
    command(b"-ERR value is not an integer or out of range\r\n", "WAIT", "x", "100")
    command(b"-ERR timeout is not an integer or out of range\r\n", "WAIT", "1", "x")
    command(b"-ERR timeout is negative\r\n", "WAIT", "1", "-1")

    print("== a replica that connects counts straight away")
    waiter = Client()
    waiter.send("WAIT", "1", "0")
    expect_nothing(waiter, "WAIT 1 0 waits, with no replicas")
    replicas = [connect_replica()]
    expect(waiter, b":1\r\n", "WAIT 1 0 replies 1 once one connects")

    print("== stage test: with no writes, WAIT replies with how many replicas there are")
    replicas += [connect_replica() for _ in range(2)]
    at_once(b":3\r\n", "WAIT", "1", "500")
    at_once(b":3\r\n", "WAIT", "3", "500")
    after(0.2, b":3\r\n", "WAIT", "5", "200")
    for replica in replicas:
        expect_nothing(replica, "with no writes, a replica isn't asked for an ACK", wait=0.1)

    print("== stage test: after a write, WAIT asks for ACKs and replies when it has enough")
    write_needing_ack(replicas, waiter, ("SET", "foo", "123"), "1", "2000")
    expect_nothing(waiter, "WAIT holds its reply until there's an ACK")
    replicas[0].send("REPLCONF", "ACK", str(SET_LENGTH))
    expect(waiter, b":1\r\n", "one ACK is enough for WAIT 1")
    offset = SET_LENGTH + len(GETACK)

    print("== without enough ACKs, it replies at the timeout with how many it has")
    start = time.monotonic()
    waiter.send("WAIT", "2", "300")
    for replica in replicas:
        expect(replica, GETACK, "the replica is asked for an ACK")
    offset += len(GETACK)
    replicas[1].send("REPLCONF", "ACK", str(SET_LENGTH - 1))
    expect(waiter, b":1\r\n", "an ACK that's short of the write doesn't count")
    elapsed = time.monotonic() - start
    check(f"it waited about 0.3s ({elapsed:.2f}s)", 0.27 <= elapsed < 0.8)

    print("== once they've all caught up, no more ACKs are needed")
    for replica in replicas[1:]:
        replica.send("REPLCONF", "ACK", str(SET_LENGTH))
    time.sleep(0.1)
    at_once(b":3\r\n", "WAIT", "3", "500")
    after(0.2, b":3\r\n", "WAIT", "4", "200")
    for replica in replicas:
        expect_nothing(replica, "a replica isn't asked for an ACK", wait=0.1)

    print("== stage test: the next write needs acknowledging again")
    write_needing_ack(replicas, waiter, ("SET", "bar", "456"), "3", "2000")
    offset += SET_LENGTH
    for replica in replicas:
        replica.send("REPLCONF", "ACK", str(offset - 1))
    expect_nothing(waiter, "ACKs from before the write don't count")
    for replica in replicas:
        replica.send("REPLCONF", "ACK", str(offset))
    expect(waiter, b":3\r\n", "all three have acknowledged it")
    offset += len(GETACK)

    print("== a replica that hangs up doesn't count")
    replicas.pop().close()
    time.sleep(0.1)
    at_once(b":2\r\n", "WAIT", "2", "500")
    after(0.2, b":2\r\n", "WAIT", "3", "200")

    print("== a timeout of 0 waits as long as it takes")
    write_needing_ack(replicas, waiter, ("SET", "baz", "789"), "1", "0")
    offset += SET_LENGTH
    expect_nothing(waiter, "WAIT holds its reply until there's an ACK")
    replicas[0].send("REPLCONF", "ACK", str(offset))
    expect(waiter, b":1\r\n", "the ACK gets it a reply")

    print("== inside a transaction, it replies at once with the count so far")
    transaction = Client()
    run(transaction, OK, "MULTI")
    run(transaction, QUEUED, "WAIT", "5", "0")
    run(transaction, b"*1\r\n:1\r\n", "EXEC")

    print("== a client that hangs up stops waiting")
    gone = Client()
    gone.send("WAIT", "9", "0")
    time.sleep(0.1)
    gone.close()
    time.sleep(0.1)

    print("== regressions")
    command(b"+PONG\r\n", "PING")
    command(b"$3\r\n789\r\n", "GET", "baz")

finish()
