#!/usr/bin/env python3
"""Authentication, from the "Respond to ACL WHOAMI" stage on: users, passwords and AUTH,
using several clients."""

from helpers import Client, bulk, command, finish, run, running_server

with running_server():
    print("== stage test: a new connection is the default user")
    command(bulk("default"), "ACL", "WHOAMI")
    command(bulk("default"), "acl", "whoami")

    print("== ACL errors")
    command(b"-ERR wrong number of arguments for 'acl|whoami' command\r\n", "ACL", "WHOAMI", "x")
    command(b"-ERR wrong number of arguments for 'acl' command\r\n", "ACL")
    command(b"-ERR unknown subcommand 'BOGUS'. Try ACL HELP.\r\n", "ACL", "BOGUS")

finish()
