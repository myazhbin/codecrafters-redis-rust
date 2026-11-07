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

    print("== arity, lowercase, and not inside a transaction")
    command(b"-ERR wrong number of arguments for 'subscribe' command\r\n", "SUBSCRIBE")
    command(subscribed("x", 1), "subscribe", "x")
    transaction = Client()
    run(transaction, OK, "MULTI")
    run(transaction, b"-ERR Command not allowed inside a transaction\r\n", "SUBSCRIBE", "x")

finish()
