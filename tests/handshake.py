#!/usr/bin/env python3
"""The replica's side of the replication handshake, from the "Send handshake" stages on,
with the test playing the master."""

import socket
import time

from helpers import PORT, Client, check, command, expect, finish, running_server

# `cargo test` gives each test a port of its own, all well below this one's.
MASTER_PORT = PORT + 100
AS_REPLICA = ("--replicaof", f"localhost {MASTER_PORT}")


def stays_quiet(connection, wait=0.3):
    """Whether the other end neither sends anything nor hangs up for `wait` seconds."""
    connection.sock.settimeout(wait)
    try:
        connection.sock.recv(1)
    except socket.timeout:
        return True
    return False


print("== with no master to connect to, the replica still serves clients")
with running_server(*AS_REPLICA):
    command(b"+PONG\r\n", "PING")
    command(b"+OK\r\n", "SET", "k", "v")
    command(b"$1\r\nv\r\n", "GET", "k")

with socket.create_server(("127.0.0.1", MASTER_PORT)) as listener, running_server(*AS_REPLICA):
    print("== stage test: the replica connects to the master and sends PING")
    listener.settimeout(2.0)
    master = Client(listener.accept()[0])
    expect(master, b"*1\r\n$4\r\nPING\r\n", "the master receives PING as a RESP array")
    check("the replica then stays connected, sending nothing more", stays_quiet(master))

    print("== it serves clients while connected to the master")
    command(b"+PONG\r\n", "PING")
    command(b"+OK\r\n", "SET", "k", "v")
    command(b"$1\r\nv\r\n", "GET", "k")

    print("== the master's reply doesn't disturb it")
    master.sock.sendall(b"+PONG\r\n")
    check("the replica stays connected, sending nothing more", stays_quiet(master))
    command(b"+PONG\r\n", "PING")

    print("== it keeps serving clients after the master hangs up")
    master.close()
    time.sleep(0.1)
    command(b"+PONG\r\n", "PING")
    command(b"$1\r\nv\r\n", "GET", "k")

finish()
