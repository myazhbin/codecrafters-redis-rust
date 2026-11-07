#!/usr/bin/env python3
"""AOF persistence, from the "Default AOF options" stage on: the append-only file's
settings, the files kept for it, logging writes to it, and replaying it at startup."""

import os
import subprocess
import sys
import tempfile
import time

from helpers import (DEFAULT_BINARY, OK, OTHER_PORT, QUEUED, WRONGTYPE, Client, check, command,
                     encode, finish, run, running_server)

BINARY = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_BINARY

DEFAULTS = {
    "appendonly": "no",
    "appenddirname": "appendonlydir",
    "appendfilename": "appendonly.aof",
    "appendfsync": "everysec",
}


def config(name, value):
    """Checks CONFIG GET's reply for one parameter."""
    command(encode(name, value), "CONFIG", "GET", name)


def commands_in(data):
    """The commands in an AOF file's contents, each a list of its arguments as strings.
    Arguments mustn't contain \\r\\n, which is enough for these tests."""
    commands, lines = [], data.split(b"\r\n")
    i = 0
    while i < len(lines) and lines[i].startswith(b"*"):
        count = int(lines[i][1:])
        commands.append([line.decode() for line in lines[i + 2:i + 2 + 2 * count:2]])
        i += 1 + 2 * count
    return commands


def read(path):
    with open(path, "rb") as f:
        return f.read()


def manifest_naming(aof_dir, appendfilename, incr_name):
    """Sets up an append-only directory as the tester does: a manifest naming an
    incremental file that isn't the default one, and that file, empty."""
    os.makedirs(aof_dir, exist_ok=True)
    with open(os.path.join(aof_dir, f"{appendfilename}.manifest"), "wb") as f:
        f.write(b"file %s seq 1 type i\n" % incr_name.encode())
    open(os.path.join(aof_dir, incr_name), "wb").close()
    return os.path.join(aof_dir, incr_name)


def file_is(path, want, label):
    """Checks that the file at `path` holds exactly `want`."""
    try:
        with open(path, "rb") as f:
            got = f.read()
    except OSError as e:
        got = e
    check(label, got == want, f"want {want!r}, got {got!r}")


print("== stage test: the AOF settings' defaults")
with running_server():
    config("dir", os.getcwd())
    for name, value in DEFAULTS.items():
        config(name, value)
    command(encode(*[item for pair in DEFAULTS.items() for item in pair]), "CONFIG", "GET", "append*")

with tempfile.TemporaryDirectory() as tmp:
    print("== stage test: flags override the defaults")
    with running_server("--dir", tmp, "--appendonly", "yes", "--appenddirname", "aofdir",
                        "--appendfilename", "log.aof", "--appendfsync", "always"):
        config("dir", tmp)
        config("appendonly", "yes")
        config("appenddirname", "aofdir")
        config("appendfilename", "log.aof")
        config("appendfsync", "always")

    print("== the options without flags keep their defaults")
    with running_server("--dir", tmp, "--appendfsync", "no"):
        config("appendfsync", "no")
        config("appendonly", "no")
        config("appenddirname", "appendonlydir")
        config("appendfilename", "appendonly.aof")

with tempfile.TemporaryDirectory() as tmp:
    aof_dir = os.path.join(tmp, "appendonlydir")
    aof_args = ("--dir", tmp, "--appendonly", "yes", "--appenddirname", "appendonlydir",
                "--appendfilename", "appendonly.aof")

    print("== without --appendonly yes, there's no append-only directory")
    with running_server("--dir", tmp, "--appendonly", "no"):
        command(b"+PONG\r\n", "PING")
    check("no directory is created", not os.path.exists(aof_dir))

    print("== stage test: --appendonly yes creates the directory at startup")
    with running_server(*aof_args):
        check("the directory exists once the server is listening", os.path.isdir(aof_dir))
        command(b"+PONG\r\n", "PING")

    print("== a directory that's already there is fine")
    with running_server(*aof_args):
        command(b"+PONG\r\n", "PING")
    check("it's still there", os.path.isdir(aof_dir))

    print("== stage test: an empty incremental AOF file is created at startup")
    incr = os.path.join(aof_dir, "appendonly.aof.1.incr.aof")
    check("the directory had one already, from the stage before", os.path.isfile(incr))
    os.remove(incr)
    with running_server(*aof_args):
        check("it's created again", os.path.isfile(incr))
        check("and it's empty", os.path.getsize(incr) == 0, f"{os.path.getsize(incr)} bytes")

    print("== the file is named after --appendfilename, in --appenddirname")
    with running_server("--dir", tmp, "--appendonly", "yes", "--appenddirname", "other",
                        "--appendfilename", "log.aof"):
        path = os.path.join(tmp, "other", "log.aof.1.incr.aof")
        check("other/log.aof.1.incr.aof is created", os.path.isfile(path))
        file_is(os.path.join(tmp, "other", "log.aof.manifest"),
                b"file log.aof.1.incr.aof seq 1 type i\n", "so is other/log.aof.manifest")

with tempfile.TemporaryDirectory() as tmp:
    aof_dir = os.path.join(tmp, "appendonlydir")
    aof_args = ("--dir", tmp, "--appendonly", "yes", "--appenddirname", "appendonlydir",
                "--appendfilename", "appendonly.aof")
    manifest = os.path.join(aof_dir, "appendonly.aof.manifest")

    print("== stage test: a manifest listing the incremental file is created at startup")
    with running_server(*aof_args):
        check("the AOF file exists", os.path.isfile(os.path.join(aof_dir, "appendonly.aof.1.incr.aof")))
        file_is(manifest, b"file appendonly.aof.1.incr.aof seq 1 type i\n", "the manifest lists it")

    print("== a manifest that's already there is kept as it is, with the file it lists")
    custom = b"file custom.1.incr.aof seq 1 type i\n"
    with open(manifest, "wb") as f:
        f.write(custom)
    with running_server(*aof_args):
        file_is(manifest, custom, "the manifest is unchanged")
        check("the file it lists is created", os.path.isfile(os.path.join(aof_dir, "custom.1.incr.aof")))

with tempfile.TemporaryDirectory() as tmp:
    aof_dir = os.path.join(tmp, "aofdir")
    incr = manifest_naming(aof_dir, "log.aof", "blueberry.1.incr.aof")
    always = ("--dir", tmp, "--appendonly", "yes", "--appenddirname", "aofdir",
              "--appendfilename", "log.aof", "--appendfsync", "always")

    print("== stage test: a write goes to the file the manifest names, before the reply")
    with running_server(*always):
        command(OK, "SET", "foo", "100")
        file_is(incr, encode("SET", "foo", "100"), "the AOF file holds the SET")
        check("no file with the default name is created",
              not os.path.exists(os.path.join(aof_dir, "log.aof.1.incr.aof")))

        print("== a SET's expiry is logged as the Unix time it expires at")
        before = int(time.time() * 1000)
        command(OK, "SET", "temp", "v", "EX", "10")
        after = int(time.time() * 1000)
        logged = commands_in(read(incr))[-1]
        ok = logged[:4] == ["SET", "temp", "v", "PXAT"] and before + 10000 <= int(logged[4]) <= after + 10000
        check("SET temp v EX 10 is logged with PXAT", ok, f"logged {logged}")
        command(b"$1\r\nv\r\n", "GET", "temp")

    print("== SET understands PXAT and EXAT, as the AOF can hold them")
    with running_server(*always):
        later = int(time.time() * 1000) + 60000
        command(OK, "SET", "a", "1", "PXAT", str(later))
        command(b"$1\r\n1\r\n", "GET", "a")
        command(OK, "SET", "b", "2", "EXAT", str(later // 1000))
        command(b"$1\r\n2\r\n", "GET", "b")
        command(OK, "SET", "gone", "3", "PXAT", "1000")
        command(b"$-1\r\n", "GET", "gone")
        command(b"-ERR invalid expire time in 'set' command\r\n", "SET", "c", "4", "PXAT", "0")

with tempfile.TemporaryDirectory() as tmp:
    incr = manifest_naming(os.path.join(tmp, "aofdir"), "log.aof", "kiwi.1.incr.aof")
    always = ("--dir", tmp, "--appendonly", "yes", "--appenddirname", "aofdir",
              "--appendfilename", "log.aof", "--appendfsync", "always")

    print("== stage test: several writes are appended in the order they came")
    with running_server(*always):
        command(OK, "SET", "foo", "100")
        command(OK, "SET", "bar", "200")
        file_is(incr, encode("SET", "foo", "100") + encode("SET", "bar", "200"), "both SETs, in order")

        print("== every kind of write is logged, as replicas get it")
        command(b":101\r\n", "INCR", "foo")
        command(b":2\r\n", "RPUSH", "list", "a", "b")
        command(b":3\r\n", "LPUSH", "list", "c")
        command(b"$1\r\nc\r\n", "LPOP", "list")
        command(encode("list", "a"), "BLPOP", "list", "0")
        xadd = Client()
        xadd.send("XADD", "stream", "*", "f", "v")
        stream_id = xadd.read(64, 0.3).split(b"\r\n")[1].decode()
        xadd.close()
        command(b":1\r\n", "DEL", "bar")
        transaction = Client()
        for args, reply in [(("MULTI",), OK), (("SET", "t", "1"), QUEUED), (("INCR", "t"), QUEUED),
                            (("EXEC",), b"*2\r\n+OK\r\n:2\r\n")]:
            run(transaction, reply, *args)
        transaction.close()
        want = [
            ["SET", "foo", "100"], ["SET", "bar", "200"], ["INCR", "foo"],
            ["RPUSH", "list", "a", "b"], ["LPUSH", "list", "c"], ["LPOP", "list"],
            # BLPOP as LPOP, and XADD with the ID it generated.
            ["LPOP", "list"], ["XADD", "stream", stream_id, "f", "v"], ["DEL", "bar"],
            ["SET", "t", "1"], ["INCR", "t"],
        ]
        got = commands_in(read(incr))
        check("the AOF holds all the writes, in order", got == want, f"want {want}, got {got}")

with tempfile.TemporaryDirectory() as tmp:
    incr = manifest_naming(os.path.join(tmp, "aofdir"), "log.aof", "lime.1.incr.aof")
    always = ("--dir", tmp, "--appendonly", "yes", "--appenddirname", "aofdir",
              "--appendfilename", "log.aof", "--appendfsync", "always")

    print("== stage test: only writes are logged, not reads")
    with running_server(*always):
        command(OK, "SET", "foo", "1")
        command(b"$1\r\n1\r\n", "GET", "foo")
        command(b"+PONG\r\n", "PING")
        command(b"$5\r\nhello\r\n", "ECHO", "hello")
        command(encode("appendfsync", "always"), "CONFIG", "GET", "appendfsync")
        command(OK, "SET", "bar", "2")
        file_is(incr, encode("SET", "foo", "1") + encode("SET", "bar", "2"), "the AOF holds just the SETs")

        print("== nor other reads, failed writes, or writes that change nothing")
        command(b":1\r\n", "RPUSH", "list", "a")
        command(OK, "SET", "text", "abc")
        logged = read(incr)
        command(encode("list"), "KEYS", "l*")
        command(b"+list\r\n", "TYPE", "list")
        command(b":1\r\n", "LLEN", "list")
        command(encode("a"), "LRANGE", "list", "0", "-1")
        command(b"*0\r\n", "XRANGE", "stream", "-", "+")
        command(b"-ERR value is not an integer or out of range\r\n", "INCR", "text")
        command(WRONGTYPE, "RPUSH", "text", "x")
        command(b":0\r\n", "DEL", "missing")
        command(b"$-1\r\n", "LPOP", "missing")
        command(b"*-1\r\n", "BLPOP", "missing", "0.1")
        command(b"-ERR unknown command 'BOGUS'\r\n", "BOGUS")
        discarded = Client()
        for args, reply in [(("MULTI",), OK), (("SET", "x", "1"), QUEUED), (("DISCARD",), OK)]:
            run(discarded, reply, *args)
        discarded.close()
        file_is(incr, logged, "none of them is in the AOF")

with tempfile.TemporaryDirectory() as tmp:
    aof_args = ("--dir", tmp, "--appendonly", "yes", "--appenddirname", "appendonlydir",
                "--appendfilename", "appendonly.aof")
    manifest = os.path.join(tmp, "appendonlydir", "appendonly.aof.manifest")
    os.makedirs(os.path.dirname(manifest))
    print("== the server exits with a message if the manifest makes no sense")
    with open(manifest, "wb") as f:
        f.write(b"file custom.1.incr.aof seq 1\n")
    result = subprocess.run([BINARY, "--port", str(OTHER_PORT), *aof_args],
                            capture_output=True, text=True, timeout=5)
    check("it exits with status 1", result.returncode == 1, f"status {result.returncode}")
    check("and says why", "couldn't read the manifest" in result.stderr, f"stderr: {result.stderr!r}")

    print("== the server exits with a message if it can't create the directory")
    blocker = os.path.join(tmp, "a-file")
    open(blocker, "w").close()
    result = subprocess.run([BINARY, "--port", str(OTHER_PORT), "--dir", blocker, "--appendonly", "yes"],
                            capture_output=True, text=True, timeout=5)
    check("it exits with status 1", result.returncode == 1, f"status {result.returncode}")
    check("and says why", "couldn't create the append-only directory" in result.stderr,
          f"stderr: {result.stderr!r}")

finish()
