#!/usr/bin/env python3
"""Pub/Sub, from the "Subscribe to a channel" stage on, using several clients."""

from helpers import OK, Client, bulk, command, finish, run, running_server


def subscribed(channel, count):
    """The reply to subscribing to `channel`, leaving the client subscribed to `count`."""
    return b"*3\r\n" + bulk("subscribe") + bulk(channel) + b":%d\r\n" % count


with running_server():
    print("== stage test: SUBSCRIBE replies with the channel and how many there are")
    client = Client()
    run(client, subscribed("foo", 1), "SUBSCRIBE", "foo")

    print("== several channels in one SUBSCRIBE get a reply each, and repeats don't count")
    other = Client()
    run(other, subscribed("a", 1) + subscribed("b", 2) + subscribed("a", 2), "SUBSCRIBE", "a", "b", "a")

    print("== stage test: SUBSCRIBE again, to a new channel and to the same one")
    first, second = Client(), Client()
    run(first, subscribed("foo", 1), "SUBSCRIBE", "foo")
    run(first, subscribed("bar", 2), "SUBSCRIBE", "bar")
    run(first, subscribed("bar", 2), "SUBSCRIBE", "bar")

    print("== each client's count is its own")
    run(second, subscribed("bar", 1), "SUBSCRIBE", "bar")
    run(second, subscribed("baz", 2), "SUBSCRIBE", "baz")
    run(first, subscribed("baz", 3), "SUBSCRIBE", "baz")
    run(Client(), subscribed("foo", 1), "SUBSCRIBE", "foo")

    print("== stage test: in subscribed mode, other commands are refused")
    for args in [("SET", "key", "value"), ("GET", "key"), ("ECHO", "hey"), ("set", "k", "v"),
                 ("MULTI",), ("RPUSH", "list", "a"), ("PUBLISH", "foo", "msg")]:
        name = args[0].lower().encode()
        run(first, b"-ERR Can't execute '%s': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT "
                   b"/ RESET are allowed in this context\r\n" % name, *args)
    run(first, subscribed("qux", 4), "SUBSCRIBE", "qux")
    command(b"$-1\r\n", "GET", "key")

    print("== clients that aren't subscribed can run anything")
    command(OK, "SET", "key", "value")
    command(b"$5\r\nvalue\r\n", "GET", "key")

    print("== stage test: PING in subscribed mode replies with an array")
    run(first, b"*2\r\n" + bulk("pong") + bulk(""), "PING")
    run(first, b"*2\r\n" + bulk("pong") + bulk("hello"), "PING", "hello")
    run(first, b"-ERR wrong number of arguments for 'ping' command\r\n", "PING", "a", "b")

    print("== and as usual otherwise")
    command(b"+PONG\r\n", "PING")
    command(bulk("hello"), "PING", "hello")
    command(b"-ERR wrong number of arguments for 'ping' command\r\n", "PING", "a", "b")

    print("== arity, lowercase, and not inside a transaction")
    command(b"-ERR wrong number of arguments for 'subscribe' command\r\n", "SUBSCRIBE")
    command(subscribed("x", 1), "subscribe", "x")
    transaction = Client()
    run(transaction, OK, "MULTI")
    run(transaction, b"-ERR Command not allowed inside a transaction\r\n", "SUBSCRIBE", "x")

finish()
