#!/usr/bin/env python3
"""RDB persistence, from the "RDB file config" stage on: CONFIG GET for where the RDB file
is, and loading the keys in it at startup."""

import os
import tempfile
from contextlib import contextmanager

from helpers import OK, Client, bulk, command, encode, check, finish, running_server

ARITY = b"-ERR wrong number of arguments for '%s' command\r\n"
NULL = b"$-1\r\n"


class Raw:
    """A value already in an RDB string encoding, such as an integer one."""

    def __init__(self, data):
        self.data = data


def length(n):
    """A length-encoded number, as RDB files store sizes."""
    if n < 1 << 6:
        return bytes([n])
    if n < 1 << 14:
        return bytes([0x40 | n >> 8, n & 0xFF])
    return b"\x80" + n.to_bytes(4, "big")


def string(text):
    """A string-encoded value: its length, then the bytes."""
    if isinstance(text, Raw):
        return text.data
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

    print("== stage test: GET reads the value from the RDB file")
    with server_with_rdb(tmp, rdb_file(("foo", "bar"))):
        command(bulk("bar"), "GET", "foo")
        command(NULL, "GET", "missing")

    print("== values stored as integers, a long value, and keys that are integers")
    long_value = "v" * 700
    with server_with_rdb(tmp, rdb_file(
        ("int8", Raw(b"\xc0\x7b")),
        ("int16", Raw(b"\xc1\x39\x30")),
        ("int32", Raw(b"\xc2\x87\xd6\x12\x00")),
        ("negative", Raw(b"\xc0\xff")),
        ("long", long_value),
        (Raw(b"\xc0\x2a"), "the key is 42"),
    )):
        command(bulk("123"), "GET", "int8")
        command(bulk("12345"), "GET", "int16")
        command(bulk("1234567"), "GET", "int32")
        command(bulk("-1"), "GET", "negative")
        command(bulk(long_value), "GET", "long")
        command(bulk("the key is 42"), "GET", "42")

        print("== loaded values are ordinary strings")
        command(b":124\r\n", "INCR", "int8")
        command(b"+string\r\n", "TYPE", "long")
        command(OK, "SET", "int16", "replaced")
        command(bulk("replaced"), "GET", "int16")

    print("== stage test: KEYS * lists every key in the RDB file")
    fruits = [("banana", "1"), ("mango", "2"), ("grape", "3"), ("apple", "4")]
    with server_with_rdb(tmp, rdb_file(*fruits)):
        unordered([key for key, _ in fruits], "KEYS", "*")
        unordered(["mango", "grape"], "KEYS", "*g*")

        print("== stage test: GET reads each key's value")
        for key, value in fruits:
            command(bulk(value), "GET", key)
        command(NULL, "GET", "cherry")

    print("== only keys in database 0 are loaded, as there's no SELECT")
    two_dbs = rdb_file(("in-db-0", "a")).replace(
        b"\xff" + b"\x00" * 8, b"\xfe\x01\x00" + string("in-db-1") + string("b") + b"\xff" + b"\x00" * 8)
    with server_with_rdb(tmp, two_dbs):
        command(encode("in-db-0"), "KEYS", "*")

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
