#!/usr/bin/env python3
"""AOF persistence, from the "Default AOF options" stage on: the append-only file's
settings, the files kept for it, logging writes to it, and replaying it at startup."""

import os

from helpers import command, encode, finish, running_server

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

finish()
