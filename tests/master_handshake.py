#!/usr/bin/env python3
"""The master's side of the replication handshake, from the "Receive handshake" stages on,
with the test playing the replica."""

from helpers import (OK, RDB_TRANSFER, Client, command, connect_replica, encode, expect,
                     expect_nothing, finish, fullresync, run, running_server)

ARITY = b"-ERR wrong number of arguments for 'psync' command\r\n"

with running_server():
    print("== stage tests: the whole handshake on one connection, one command at a time")
    replica = connect_replica()
    expect_nothing(replica, "nothing follows the RDB file, not even \\r\\n")

    print("== REPLCONF's arguments and case don't matter")
    command(OK, "REPLCONF", "listening-port", "6380")
    command(OK, "replconf", "capa", "psync2")
    command(OK, "REPLCONF", "capa", "eof", "capa", "psync2")

    print("== a connection that stops at REPLCONF still works for other commands")
    client = Client()
    run(client, OK, "REPLCONF", "listening-port", "6380")
    run(client, OK, "SET", "k", "v")
    run(client, b"$1\r\nv\r\n", "GET", "k")

    print("== PSYNC in any case, whatever it asks for, gets the offset the stream has reached")
    lowercase = Client()
    offset = len(encode("SET", "k", "v"))
    run(lowercase, fullresync(offset), "psync", "8371b4fb1155b71f4a04d3e1bc3e18c4a990aeeb", "100")
    expect(lowercase, RDB_TRANSFER, "the replica receives an empty RDB file")

    print("== PSYNC errors")
    command(ARITY, "PSYNC")
    command(ARITY, "PSYNC", "?")
    transaction = Client()
    run(transaction, OK, "MULTI")
    run(transaction, b"-ERR Command not allowed inside a transaction\r\n", "PSYNC", "?", "-1")
    run(transaction, b"*0\r\n", "EXEC")

    print("== regressions")
    command(b"+PONG\r\n", "PING")
    command(b"$1\r\nv\r\n", "GET", "k")

finish()
