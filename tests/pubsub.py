#!/usr/bin/env python3
"""Pub/Sub, from the "Subscribe to a channel" stage on, using several clients."""

import time

from helpers import (OK, Client, bulk, command, expect, expect_nothing, finish, run,
                     running_server)


def subscribed(channel, count):
    """The reply to subscribing to `channel`, leaving the client subscribed to `count`."""
    return b"*3\r\n" + bulk("subscribe") + bulk(channel) + b":%d\r\n" % count


def unsubscribed(channel, count):
    """The reply to unsubscribing from `channel`, leaving the client subscribed to `count`."""
    return b"*3\r\n" + bulk("unsubscribe") + bulk(channel) + b":%d\r\n" % count


def message(channel, text):
    """What a subscriber receives when `text` is published to `channel`."""
    return b"*3\r\n" + bulk("message") + bulk(channel) + bulk(text)


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

    print("== stage test: PUBLISH replies with how many clients are subscribed")
    news, sport1, sport2 = Client(), Client(), Client()
    run(news, subscribed("news", 1), "SUBSCRIBE", "news")
    run(sport1, subscribed("sport", 1), "SUBSCRIBE", "sport")
    run(sport2, subscribed("sport", 1), "SUBSCRIBE", "sport")
    command(b":2\r\n", "PUBLISH", "sport", "msg")
    command(b":1\r\n", "PUBLISH", "news", "msg")
    command(b":0\r\n", "PUBLISH", "nobody-listens", "msg")

    print("== a client that hangs up stops counting")
    sport2.close()
    time.sleep(0.2)
    command(b":1\r\n", "PUBLISH", "sport", "msg")

    print("== stage test: subscribers get each message published to their channels")
    a1, a2, b1, idle = Client(), Client(), Client(), Client()
    run(a1, subscribed("chan-a", 1), "SUBSCRIBE", "chan-a")
    run(a2, subscribed("chan-a", 1), "SUBSCRIBE", "chan-a")
    run(b1, subscribed("chan-b", 1), "SUBSCRIBE", "chan-b")
    command(b":2\r\n", "PUBLISH", "chan-a", "hello")
    command(b":1\r\n", "PUBLISH", "chan-b", "world")
    expect(a1, message("chan-a", "hello"), "the first chan-a subscriber gets hello")
    expect(a2, message("chan-a", "hello"), "so does the second")
    expect(b1, message("chan-b", "world"), "the chan-b subscriber gets world")
    expect_nothing(a1, "chan-a's subscribers don't get chan-b's message")
    expect_nothing(idle, "a client that isn't subscribed gets nothing")

    print("== messages arrive in order, from any channel the client is subscribed to")
    run(a1, subscribed("chan-b", 2), "SUBSCRIBE", "chan-b")
    command(b":2\r\n", "PUBLISH", "chan-a", "one")
    command(b":2\r\n", "PUBLISH", "chan-b", "two")
    command(b":2\r\n", "PUBLISH", "chan-a", "three")
    expect(a1, message("chan-a", "one") + message("chan-b", "two") + message("chan-a", "three"),
           "the client subscribed to both channels gets all three")
    expect(a2, message("chan-a", "one") + message("chan-a", "three"), "chan-a's other subscriber gets two")
    expect(b1, message("chan-b", "two"), "chan-b's other subscriber gets one")

    print("== a subscriber can still PING between messages")
    run(a2, b"*2\r\n" + bulk("pong") + bulk(""), "PING")
    command(b":2\r\n", "PUBLISH", "chan-a", "after ping")
    expect(a2, message("chan-a", "after ping"), "the message after the PING")

    print("== stage test: UNSUBSCRIBE, with messages before and after")
    one, two = Client(), Client()
    run(one, subscribed("u-foo", 1) + subscribed("u-baz", 2), "SUBSCRIBE", "u-foo", "u-baz")
    run(two, subscribed("u-foo", 1) + subscribed("u-bar", 2), "SUBSCRIBE", "u-foo", "u-bar")
    command(b":2\r\n", "PUBLISH", "u-foo", "before-unsubscribe")
    expect(one, message("u-foo", "before-unsubscribe"), "client one gets the first message")
    expect(two, message("u-foo", "before-unsubscribe"), "so does client two")
    run(one, unsubscribed("u-foo", 1), "UNSUBSCRIBE", "u-foo")
    command(b":1\r\n", "PUBLISH", "u-foo", "after-unsubscribe")
    expect(two, message("u-foo", "after-unsubscribe"), "client two gets the second message")
    expect_nothing(one, "client one doesn't")
    command(b":1\r\n", "PUBLISH", "u-baz", "still")
    expect(one, message("u-baz", "still"), "but still gets its other channel's")

    print("== a channel the client isn't subscribed to leaves the count as it was")
    run(one, unsubscribed("u-nope", 1), "UNSUBSCRIBE", "u-nope")
    run(Client(), unsubscribed("u-nope", 0), "UNSUBSCRIBE", "u-nope")

    print("== several channels, and no channels meaning all of them")
    run(two, subscribed("u-a", 3) + subscribed("u-b", 4), "SUBSCRIBE", "u-a", "u-b")
    run(two, unsubscribed("u-a", 3) + unsubscribed("u-bar", 2), "UNSUBSCRIBE", "u-a", "u-bar")
    run(two, unsubscribed("u-foo", 1) + unsubscribed("u-b", 0), "UNSUBSCRIBE")
    run(two, b"*3\r\n" + bulk("unsubscribe") + b"$-1\r\n:0\r\n", "UNSUBSCRIBE")

    print("== with no channels left, the client is out of subscribed mode")
    run(two, b"+PONG\r\n", "PING")
    run(two, OK, "SET", "after", "pubsub")
    command(b":0\r\n", "PUBLISH", "u-b", "nobody")
    run(one, unsubscribed("u-baz", 0), "UNSUBSCRIBE", "u-baz")
    run(one, bulk("pubsub"), "GET", "after")

    print("== a message published before UNSUBSCRIBE still comes before its reply")
    run(one, subscribed("u-race", 1), "SUBSCRIBE", "u-race")
    racer = Client()
    run(racer, b":1\r\n", "PUBLISH", "u-race", "first")
    run(one, message("u-race", "first") + unsubscribed("u-race", 0), "UNSUBSCRIBE", "u-race")

    print("== PUBLISH's arity, and inside a transaction")
    command(b"-ERR wrong number of arguments for 'publish' command\r\n", "PUBLISH", "news")
    publisher = Client()
    run(publisher, OK, "MULTI")
    run(publisher, b"+QUEUED\r\n", "PUBLISH", "news", "msg")
    run(publisher, b"*1\r\n:1\r\n", "EXEC")

    print("== arity, lowercase, and not inside a transaction")
    command(b"-ERR wrong number of arguments for 'subscribe' command\r\n", "SUBSCRIBE")
    command(subscribed("x", 1), "subscribe", "x")
    transaction = Client()
    run(transaction, OK, "MULTI")
    run(transaction, b"-ERR Command not allowed inside a transaction\r\n", "SUBSCRIBE", "x")

finish()
