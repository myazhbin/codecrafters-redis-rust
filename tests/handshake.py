#!/usr/bin/env python3
"""The replica's side of the replication handshake, from the "Send handshake" stages on,
with the test playing the master."""

import socket
import time
from contextlib import contextmanager

from helpers import PORT, Client, check, command, expect, finish, running_server

# `cargo test` gives each test a port of its own, all well below this one's.
MASTER_PORT = PORT + 100
AS_REPLICA = ("--replicaof", f"localhost {MASTER_PORT}")

PING = b"*1\r\n$4\r\nPING\r\n"
LISTENING_PORT = b"*3\r\n$8\r\nREPLCONF\r\n$14\r\nlistening-port\r\n$%d\r\n%d\r\n" % (len(str(PORT)), PORT)
CAPA = b"*3\r\n$8\r\nREPLCONF\r\n$4\r\ncapa\r\n$6\r\npsync2\r\n"
PSYNC = b"*3\r\n$5\r\nPSYNC\r\n$1\r\n?\r\n$2\r\n-1\r\n"


@contextmanager
def connected_replica():
    """Runs a replica, and gives the test the master's end of the connection it makes."""
    with socket.create_server(("127.0.0.1", MASTER_PORT)) as listener:
        listener.settimeout(2.0)
        with running_server(*AS_REPLICA):
            yield Client(listener.accept()[0])


def stays_quiet(connection, wait=0.3):
    """Whether the other end neither sends anything nor hangs up for `wait` seconds."""
    connection.sock.settimeout(wait)
    try:
        connection.sock.recv(1)
    except socket.timeout:
        return True
    return False


def hangs_up(connection, wait=1.0):
    """Whether the other end hangs up within `wait` seconds, having sent nothing more."""
    connection.sock.settimeout(wait)
    try:
        return connection.sock.recv(1) == b""
    except socket.timeout:
        return False


print("== with no master to connect to, the replica still serves clients")
with running_server(*AS_REPLICA):
    command(b"+PONG\r\n", "PING")
    command(b"+OK\r\n", "SET", "k", "v")
    command(b"$1\r\nv\r\n", "GET", "k")

with connected_replica() as master:
    print("== stage test: each handshake command waits for the reply to the one before")
    expect(master, PING, "the master receives PING as a RESP array")
    check("the replica waits for the reply to PING", stays_quiet(master))
    master.sock.sendall(b"+PONG\r\n")
    expect(master, LISTENING_PORT, f"then REPLCONF listening-port {PORT}")
    check("the replica waits for the reply to that", stays_quiet(master))
    master.sock.sendall(b"+OK\r\n")
    expect(master, CAPA, "then REPLCONF capa psync2")
    check("the replica waits for the reply to that too", stays_quiet(master))
    master.sock.sendall(b"+OK\r\n")
    expect(master, PSYNC, "then PSYNC ? -1")
    master.sock.sendall(b"+FULLRESYNC 8371b4fb1155b71f4a04d3e1bc3e18c4a990aeeb 0\r\n")
    check("the replica then stays connected, sending nothing more", stays_quiet(master))

    print("== it serves clients while connected to the master")
    command(b"+PONG\r\n", "PING")
    command(b"+OK\r\n", "SET", "k", "v")
    command(b"$1\r\nv\r\n", "GET", "k")

    print("== what the master sends after the handshake doesn't disturb it")
    master.sock.sendall(b"$5\r\nREDIS")
    check("the replica stays connected, sending nothing more", stays_quiet(master))
    command(b"+PONG\r\n", "PING")

    print("== it keeps serving clients after the master hangs up")
    master.close()
    time.sleep(0.1)
    command(b"+PONG\r\n", "PING")
    command(b"$1\r\nv\r\n", "GET", "k")

with connected_replica() as master:
    print("== a reply that arrives in pieces is waited for")
    expect(master, PING, "the master receives PING")
    master.sock.sendall(b"+PO")
    check("the replica doesn't go on after part of a reply", stays_quiet(master))
    master.sock.sendall(b"NG\r\n")
    expect(master, LISTENING_PORT, "it goes on once the rest of the reply arrives")

with connected_replica() as master:
    print("== an error reply ends the handshake")
    expect(master, PING, "the master receives PING")
    master.sock.sendall(b"+PONG\r\n")
    expect(master, LISTENING_PORT, "then REPLCONF listening-port")
    master.sock.sendall(b"-ERR no thanks\r\n")
    check("the replica hangs up, without sending REPLCONF capa", hangs_up(master))
    command(b"+PONG\r\n", "PING")

with connected_replica() as master:
    print("== a master that hangs up during the handshake just ends it")
    expect(master, PING, "the master receives PING")
    master.close()
    time.sleep(0.1)
    command(b"+PONG\r\n", "PING")

finish()
