#!/usr/bin/env python3
"""AOF persistence, from the "Default AOF options" stage on: the append-only file's
settings, the files kept for it, logging writes to it, and replaying it at startup."""

import os
import subprocess
import sys
import tempfile

from helpers import (DEFAULT_BINARY, OTHER_PORT, check, command, encode, finish,
                     running_server)

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

    print("== the server exits with a message if it can't create the directory")
    blocker = os.path.join(tmp, "a-file")
    open(blocker, "w").close()
    result = subprocess.run([BINARY, "--port", str(OTHER_PORT), "--dir", blocker, "--appendonly", "yes"],
                            capture_output=True, text=True, timeout=5)
    check("it exits with status 1", result.returncode == 1, f"status {result.returncode}")
    check("and says why", "couldn't create the append-only directory" in result.stderr,
          f"stderr: {result.stderr!r}")

finish()
