#!/usr/bin/env python3
"""BLPOP, from the "Blocking retrieval" stages on, using several client connections."""

import time

from helpers import (WRONGTYPE, Client, check, command, encode, expect, expect_nothing,
                     finish, running_server)

with running_server():
    print("== stage test: BLPOP with timeout 0 is served by a later RPUSH")
    a = Client()
    a.send("BLPOP", "list_key", "0")
    expect_nothing(a, "BLPOP list_key 0 blocks")
    command(b":1\r\n", "RPUSH", "list_key", "foo")
    expect(a, encode("list_key", "foo"), "the blocked client receives [list_key, foo]")
    command(b":0\r\n", "LLEN", "list_key")

    print("== several blocked clients are served longest-waiting first")
    c1, c2, c3 = Client(), Client(), Client()
    for client in (c1, c2, c3):
        client.send("BLPOP", "another_list_key", "0")
        time.sleep(0.1)
    command(b":1\r\n", "RPUSH", "another_list_key", "foo")
    expect(c1, encode("another_list_key", "foo"), "the first client gets foo")
    expect_nothing(c2, "the second client is still blocked")
    expect_nothing(c3, "the third client is still blocked", wait=0.1)
    command(b":2\r\n", "RPUSH", "another_list_key", "bar", "baz")
    expect(c2, encode("another_list_key", "bar"), "the second client gets bar")
    expect(c3, encode("another_list_key", "baz"), "the third client gets baz")
    command(b":0\r\n", "LLEN", "another_list_key")

    print("== LPUSH serves a blocked client from the head of the list")
    lp = Client()
    lp.send("BLPOP", "lp", "0")
    time.sleep(0.1)
    command(b":2\r\n", "LPUSH", "lp", "a", "b")
    expect(lp, encode("lp", "b"), "the blocked client gets b")
    command(encode("a"), "LRANGE", "lp", "0", "-1")

    print("== an element that is already there is popped at once")
    command(b":2\r\n", "RPUSH", "ready", "a", "b")
    command(encode("ready", "a"), "BLPOP", "ready", "0")
    command(encode("b"), "LRANGE", "ready", "0", "-1")

    print("== timeouts")
    t = Client()
    start = time.monotonic()
    t.send("BLPOP", "slow", "0.5")
    expect(t, b"*-1\r\n", "BLPOP slow 0.5 times out with a null array", timeout=2.0)
    elapsed = time.monotonic() - start
    check(f"it waited about 0.5s ({elapsed:.2f}s)", 0.45 <= elapsed < 1.0)
    command(b":1\r\n", "RPUSH", "slow", "x")
    command(encode("x"), "LRANGE", "slow", "0", "-1")
    t2 = Client()
    t2.send("BLPOP", "slow2", "2")
    time.sleep(0.2)
    command(b":1\r\n", "RPUSH", "slow2", "y")
    expect(t2, encode("slow2", "y"), "BLPOP with time left is served normally")

    print("== a client that hangs up stops waiting")
    gone = Client()
    gone.send("BLPOP", "gone", "0")
    time.sleep(0.1)
    gone.close()
    time.sleep(0.1)
    command(b":1\r\n", "RPUSH", "gone", "z")
    command(encode("z"), "LRANGE", "gone", "0", "-1")

    print("== commands pipelined behind BLPOP wait their turn")
    pipe = Client()
    pipe.sock.sendall(encode("BLPOP", "pipe", "0") + encode("PING"))
    expect_nothing(pipe, "nothing is answered while BLPOP blocks")
    command(b":1\r\n", "RPUSH", "pipe", "v")
    expect(pipe, encode("pipe", "v") + b"+PONG\r\n", "the BLPOP reply, then PONG")

    print("== errors")
    command(b"+OK\r\n", "SET", "str", "val")
    command(WRONGTYPE, "BLPOP", "str", "0")
    command(b"-ERR timeout is negative\r\n", "BLPOP", "k", "-1")
    command(b"-ERR timeout is not a float or out of range\r\n", "BLPOP", "k", "abc")
    command(b"-ERR wrong number of arguments for 'blpop' command\r\n", "BLPOP", "k")

    print("== regressions")
    command(b"+PONG\r\n", "PING")
    command(b"$3\r\nval\r\n", "GET", "str")
    command(b":3\r\n", "RPUSH", "r", "a", "b", "c")
    command(b"$1\r\na\r\n", "LPOP", "r")
    command(encode("b", "c"), "LPOP", "r", "5")
    command(b"$-1\r\n", "GET", "r")

finish()
