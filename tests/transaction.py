#!/usr/bin/env python3
"""MULTI, EXEC and DISCARD, from the "Transactions" stages on, using several clients."""

import threading
import time

from helpers import (OK, QUEUED, Client, check, command, encode, entry, expect, finish,
                     run, running_server, streams)

with running_server():
    print("== stage test")
    command(OK, "MULTI")

    print("== commands are queued, then run by EXEC")
    c = Client()
    run(c, OK, "MULTI")
    run(c, QUEUED, "SET", "foo", "41")
    run(c, QUEUED, "INCR", "foo")
    command(b"$-1\r\n", "GET", "foo")
    run(c, b"*2\r\n+OK\r\n:42\r\n", "EXEC")
    command(b"$2\r\n42\r\n", "GET", "foo")
    run(c, b"-ERR EXEC without MULTI\r\n", "EXEC")

    print("== empty transaction")
    run(c, OK, "MULTI")
    run(c, b"*0\r\n", "EXEC")

    print("== EXEC and DISCARD without MULTI, nested MULTI")
    command(b"-ERR EXEC without MULTI\r\n", "EXEC")
    command(b"-ERR DISCARD without MULTI\r\n", "DISCARD")
    run(c, OK, "MULTI")
    run(c, b"-ERR MULTI calls can not be nested\r\n", "MULTI")
    run(c, QUEUED, "PING")
    run(c, b"*1\r\n+PONG\r\n", "EXEC")

    print("== DISCARD")
    run(c, OK, "MULTI")
    run(c, QUEUED, "SET", "foo", "100")
    run(c, OK, "DISCARD")
    command(b"$2\r\n42\r\n", "GET", "foo")
    run(c, b"-ERR DISCARD without MULTI\r\n", "DISCARD")
    run(c, b"-ERR EXEC without MULTI\r\n", "EXEC")

    print("== a failing command doesn't stop the others")
    command(OK, "SET", "bad", "abc")
    command(OK, "SET", "good", "7")
    run(c, OK, "MULTI")
    run(c, QUEUED, "INCR", "bad")
    run(c, QUEUED, "INCR", "good")
    run(c, b"*2\r\n-ERR value is not an integer or out of range\r\n:8\r\n", "EXEC")

    print("== each connection has its own transaction")
    c1, c2, c3 = Client(), Client(), Client()
    run(c1, OK, "MULTI")
    run(c2, OK, "MULTI")
    run(c1, QUEUED, "INCR", "counter")
    run(c2, QUEUED, "INCR", "counter")
    run(c3, b":1\r\n", "INCR", "counter")
    run(c1, b"*1\r\n:2\r\n", "EXEC")
    run(c2, b"*1\r\n:3\r\n", "EXEC")

    print("== a whole transaction in one write")
    piped = Client()
    piped.sock.sendall(encode("MULTI") + encode("INCR", "piped") + encode("INCR", "piped")
                       + encode("EXEC"))
    expect(piped, b"+OK\r\n+QUEUED\r\n+QUEUED\r\n*2\r\n:1\r\n:2\r\n",
           "MULTI, INCR piped, INCR piped, EXEC")

    print("== arity, lowercase, different kinds of reply inside EXEC")
    command(b"-ERR wrong number of arguments for 'multi' command\r\n", "MULTI", "x")
    command(b"-ERR wrong number of arguments for 'exec' command\r\n", "EXEC", "x")
    command(b"-ERR wrong number of arguments for 'discard' command\r\n", "DISCARD", "x")
    run(c, OK, "multi")
    run(c, QUEUED, "rpush", "list", "a", "b")
    run(c, QUEUED, "lrange", "list", "0", "-1")
    run(c, QUEUED, "get", "missing")
    run(c, b"*3\r\n:2\r\n" + encode("a", "b") + b"$-1\r\n", "exec")

    print("== a client that hangs up mid-transaction")
    gone = Client()
    run(gone, OK, "MULTI")
    run(gone, QUEUED, "SET", "never", "1")
    gone.close()
    time.sleep(0.1)
    command(b"$-1\r\n", "GET", "never")
    command(b"+PONG\r\n", "PING")

    print("== EXEC runs all at once: another client's commands can't land in between")
    stop = threading.Event()

    def keep_setting_shared_to_2():
        writer = Client()
        batch = encode("SET", "shared", "2") * 50
        while not stop.is_set():
            writer.sock.sendall(batch)
            writer.read(len(OK) * 50, timeout=5.0)
        writer.close()

    writer = threading.Thread(target=keep_setting_shared_to_2)
    writer.start()
    time.sleep(0.2)
    gets = 2000
    header = b"*%d\r\n+OK\r\n" % (gets + 1)
    want = header + b"$1\r\n1\r\n" * gets
    atomic = Client()
    complete, saw_2 = True, 0
    for _ in range(3):
        atomic.sock.sendall(encode("MULTI") + encode("SET", "shared", "1")
                            + encode("GET", "shared") * gets + encode("EXEC"))
        atomic.read(len(OK) + len(QUEUED) * (gets + 1), timeout=5.0)
        reply = atomic.read(len(want), timeout=5.0)
        complete = complete and len(reply) == len(want) and reply.startswith(header)
        saw_2 += reply.count(b"$1\r\n2\r\n")
    stop.set()
    writer.join()
    check(f"3 transactions of SET shared 1 and {gets} GETs, while another client keeps "
          f"setting it to 2: no GET sees 2 ({saw_2} did)", complete and saw_2 == 0)

    print("== BLPOP and XREAD BLOCK inside a transaction don't wait, as in Redis")
    t = Client()
    run(t, OK, "MULTI")
    run(t, QUEUED, "BLPOP", "nothing", "0")
    run(t, QUEUED, "XREAD", "BLOCK", "0", "STREAMS", "nostream", "$")
    start = time.monotonic()
    run(t, b"*2\r\n*-1\r\n*-1\r\n", "EXEC")
    elapsed = time.monotonic() - start
    check(f"EXEC replies without waiting ({elapsed:.2f}s)", elapsed < 0.5)
    command(b":1\r\n", "RPUSH", "ready", "a")
    run(t, OK, "MULTI")
    run(t, QUEUED, "BLPOP", "ready", "0")
    run(t, b"*1\r\n" + encode("ready", "a"), "EXEC")

    print("== a waiting BLPOP is only served once the whole transaction is done")
    waiter = Client()
    waiter.send("BLPOP", "jobs", "0")
    time.sleep(0.1)
    t = Client()
    run(t, OK, "MULTI")
    run(t, QUEUED, "RPUSH", "jobs", "j1")
    run(t, QUEUED, "LLEN", "jobs")
    run(t, b"*2\r\n:1\r\n:1\r\n", "EXEC")
    expect(waiter, encode("jobs", "j1"), "then the waiting BLPOP gets j1")
    command(b":0\r\n", "LLEN", "jobs")

    print("== a waiting XREAD gets all of a transaction's entries together")
    reader = Client()
    reader.send("XREAD", "BLOCK", "0", "STREAMS", "events", "$")
    time.sleep(0.1)
    t = Client()
    run(t, OK, "MULTI")
    run(t, QUEUED, "XADD", "events", "1-1", "n", "1")
    run(t, QUEUED, "XADD", "events", "1-2", "n", "2")
    run(t, b"*2\r\n$3\r\n1-1\r\n$3\r\n1-2\r\n", "EXEC")
    expect(reader, streams(("events", [entry("1-1", "n", "1"), entry("1-2", "n", "2")])),
           "the waiting XREAD gets both entries")

finish()
