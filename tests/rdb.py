#!/usr/bin/env python3
"""RDB persistence, from the "RDB file config" stage on: CONFIG GET for where the RDB file
is, and loading the keys in it at startup."""

import os
import tempfile
from contextlib import contextmanager

from helpers import OK, Client, command, encode, check, finish, running_server

ARITY = b"-ERR wrong number of arguments for '%s' command\r\n"


def length(n):
    """A length-encoded number, as RDB files store sizes."""
    if n < 1 << 6:
        return bytes([n])
    if n < 1 << 14:
        return bytes([0x40 | n >> 8, n & 0xFF])
    return b"\x80" + n.to_bytes(4, "big")


def string(text):
    """A string-encoded value: its length, then the bytes."""
    data = text.encode() if isinstance(text, str) else text
    return length(len(data)) + data


def rdb_file(*keys):
    """An RDB file holding string keys, each given as (key, value) or (key, value, expiry),
    where the expiry is the opcode and timestamp that come before the key."""
    out = b"REDIS0011"
    out += b"\xfa" + string("redis-ver") + string("7.2.0")
    out += b"\xfa" + string("redis-bits") + b"\xc0\x40"
    with_expiry = sum(1 for key in keys if len(key) > 2)
    out += b"\xfe\x00\xfb" + length(len(keys)) + length(with_expiry)
    for key, value, *expiry in keys:
        out += b"".join(expiry) + b"\x00" + string(key) + string(value)
    # The end of the file, then a checksum, which the server doesn't check.
    return out + b"\xff" + b"\x00" * 8


@contextmanager
def server_with_rdb(directory, data, name="dump.rdb"):
    """Runs the server with an RDB file holding `data` for it to load."""
    with open(os.path.join(directory, name), "wb") as f:
        f.write(data)
    with running_server("--dir", directory, "--dbfilename", name):
        yield


def unordered(want, *args):
    """Runs a command whose reply is an array of bulk strings in no particular order, such
    as KEYS, and checks it holds `want`. The reply's length doesn't depend on the order."""
    client = Client()
    client.send(*args)
    got = client.read(len(encode(*want)), 1.0)
    client.close()
    lines = got.split(b"\r\n")
    items = lines[2:-1:2] if got.startswith(b"*%d\r\n" % len(want)) else None
    ok = items is not None and sorted(items) == sorted(item.encode() for item in want)
    check(" ".join(args), ok, f"want {sorted(want)} in any order, got {got!r}")


with tempfile.TemporaryDirectory() as tmp:
    print("== stage test: CONFIG GET reads --dir and --dbfilename")
    with running_server("--dir", tmp, "--dbfilename", "dump.rdb"):
        command(encode("dir", tmp), "CONFIG", "GET", "dir")
        command(encode("dbfilename", "dump.rdb"), "CONFIG", "GET", "dbfilename")

        print("== any case, several parameters, glob patterns, no match")
        command(encode("dir", tmp), "config", "get", "DIR")
        command(encode("dir", tmp, "dbfilename", "dump.rdb"), "CONFIG", "GET", "dbfilename", "dir")
        command(encode("dbfilename", "dump.rdb"), "CONFIG", "GET", "db*")
        command(encode("dir", tmp), "CONFIG", "GET", "d?r")
        command(encode("dir", tmp), "CONFIG", "GET", "dir", "d*r")
        command(b"*0\r\n", "CONFIG", "GET", "nosuchparameter")

        print("== errors")
        command(ARITY % b"config", "CONFIG")
        command(ARITY % b"config|get", "CONFIG", "GET")
        command(b"-ERR unknown subcommand 'BOGUS'. Try CONFIG HELP.\r\n", "CONFIG", "BOGUS")

    print("== the directory doesn't have to exist")
    missing = os.path.join(tmp, "not", "there")
    with running_server("--dir", missing, "--dbfilename", "x.rdb"):
        command(encode("dir", missing), "CONFIG", "GET", "dir")
        command(encode("dbfilename", "x.rdb"), "CONFIG", "GET", "dbfilename")
        command(b"+PONG\r\n", "PING")
        command(b"*0\r\n", "KEYS", "*")

    print("== stage test: KEYS * lists the key in the RDB file")
    with server_with_rdb(tmp, rdb_file(("foo", "bar"))):
        command(encode("foo"), "KEYS", "*")

        print("== KEYS patterns, arity")
        command(encode("foo"), "keys", "f*")
        command(encode("foo"), "KEYS", "f?o")
        command(b"*0\r\n", "KEYS", "b*")
        command(b"*0\r\n", "KEYS", "FOO")
        command(ARITY % b"keys", "KEYS")
        command(ARITY % b"keys", "KEYS", "a", "b")

        print("== keys set since are listed too")
        command(OK, "SET", "bar", "1")
        unordered(["foo", "bar"], "KEYS", "*")
        command(encode("bar"), "KEYS", "b*")

    print("== an RDB file that doesn't exist means an empty database")
    with running_server("--dir", tmp, "--dbfilename", "nosuchfile.rdb"):
        command(b"*0\r\n", "KEYS", "*")
        command(OK, "SET", "k", "v")
        command(encode("k"), "KEYS", "*")

print("== without the flags: dump.rdb in the working directory")
with running_server():
    command(encode("dir", os.getcwd()), "CONFIG", "GET", "dir")
    command(encode("dbfilename", "dump.rdb"), "CONFIG", "GET", "dbfilename")

finish()
