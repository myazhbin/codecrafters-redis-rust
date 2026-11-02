#!/usr/bin/env python3
"""The replica's side of the replication handshake, from the "Send handshake" stages on,
with the test playing the master."""

import time

from helpers import (OK, OTHER_PORT, PORT, RDB_TRANSFER, check, command, expect, finish, fullresync,
                     hangs_up, replica_of_test, running_server, stays_quiet)

PING = b"*1\r\n$4\r\nPING\r\n"
LISTENING_PORT = b"*3\r\n$8\r\nREPLCONF\r\n$14\r\nlistening-port\r\n$%d\r\n%d\r\n" % (len(str(PORT)), PORT)
CAPA = b"*3\r\n$8\r\nREPLCONF\r\n$4\r\ncapa\r\n$6\r\npsync2\r\n"
PSYNC = b"*3\r\n$5\r\nPSYNC\r\n$1\r\n?\r\n$2\r\n-1\r\n"

print("== with no master to connect to, the replica still serves clients")
with running_server("--replicaof", f"localhost {OTHER_PORT}"):
    command(b"+PONG\r\n", "PING")
    command(b"+OK\r\n", "SET", "k", "v")
    command(b"$1\r\nv\r\n", "GET", "k")

with replica_of_test() as master:
    print("== stage test: each handshake command waits for the reply to the one before")
    expect(master, PING, "the master receives PING as a RESP array")
    check("the replica waits for the reply to PING", stays_quiet(master))
    master.sock.sendall(b"+PONG\r\n")
    expect(master, LISTENING_PORT, f"then REPLCONF listening-port {PORT}")
    check("the replica waits for the reply to that", stays_quiet(master))
    master.sock.sendall(OK)
    expect(master, CAPA, "then REPLCONF capa psync2")
    check("the replica waits for the reply to that too", stays_quiet(master))
    master.sock.sendall(OK)
    expect(master, PSYNC, "then PSYNC ? -1")
    master.sock.sendall(fullresync())
    check("the replica then waits for the RDB file", stays_quiet(master))

    print("== it serves clients while connected to the master")
    command(b"+PONG\r\n", "PING")
    command(b"+OK\r\n", "SET", "k", "v")
    command(b"$1\r\nv\r\n", "GET", "k")

    print("== after the RDB file, it stays connected")
    master.sock.sendall(RDB_TRANSFER)
    check("the replica stays connected, sending nothing more", stays_quiet(master))
    command(b"+PONG\r\n", "PING")

    print("== it keeps serving clients after the master hangs up")
    master.close()
    time.sleep(0.1)
    command(b"+PONG\r\n", "PING")
    command(b"$1\r\nv\r\n", "GET", "k")

with replica_of_test() as master:
    print("== a reply that arrives in pieces is waited for")
    expect(master, PING, "the master receives PING")
    master.sock.sendall(b"+PO")
    check("the replica doesn't go on after part of a reply", stays_quiet(master))
    master.sock.sendall(b"NG\r\n")
    expect(master, LISTENING_PORT, "it goes on once the rest of the reply arrives")

with replica_of_test() as master:
    print("== an error reply ends the handshake")
    expect(master, PING, "the master receives PING")
    master.sock.sendall(b"+PONG\r\n")
    expect(master, LISTENING_PORT, "then REPLCONF listening-port")
    master.sock.sendall(b"-ERR no thanks\r\n")
    check("the replica hangs up, without sending REPLCONF capa", hangs_up(master))
    command(b"+PONG\r\n", "PING")

with replica_of_test() as master:
    print("== so does anything but FULLRESYNC in reply to PSYNC")
    expect(master, PING, "the master receives PING")
    master.sock.sendall(b"+PONG\r\n")
    expect(master, LISTENING_PORT, "then REPLCONF listening-port")
    master.sock.sendall(OK)
    expect(master, CAPA, "then REPLCONF capa psync2")
    master.sock.sendall(OK)
    expect(master, PSYNC, "then PSYNC ? -1")
    master.sock.sendall(OK)
    check("the replica hangs up", hangs_up(master))
    command(b"+PONG\r\n", "PING")

with replica_of_test() as master:
    print("== a master that hangs up during the handshake just ends it")
    expect(master, PING, "the master receives PING")
    master.close()
    time.sleep(0.1)
    command(b"+PONG\r\n", "PING")

finish()
