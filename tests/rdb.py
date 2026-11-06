#!/usr/bin/env python3
"""RDB persistence, from the "RDB file config" stage on: CONFIG GET for where the RDB file
is, and loading the keys in it at startup."""

import os
import tempfile

from helpers import command, encode, finish, running_server

ARITY = b"-ERR wrong number of arguments for '%s' command\r\n"

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

print("== without the flags: dump.rdb in the working directory")
with running_server():
    command(encode("dir", os.getcwd()), "CONFIG", "GET", "dir")
    command(encode("dbfilename", "dump.rdb"), "CONFIG", "GET", "dbfilename")

finish()
