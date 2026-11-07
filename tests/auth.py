#!/usr/bin/env python3
"""Authentication, from the "Respond to ACL WHOAMI" stage on: users, passwords and AUTH,
using several clients."""

from helpers import Client, bulk, command, finish, run, running_server

with running_server():
    print("== stage test: a new connection is the default user")
    command(bulk("default"), "ACL", "WHOAMI")
    command(bulk("default"), "acl", "whoami")

    print("== stage test: ACL GETUSER gives the default user's flags, just nopass, and no passwords")
    command(b"*4\r\n" + bulk("flags") + b"*1\r\n" + bulk("nopass") + bulk("passwords") + b"*0\r\n",
            "ACL", "GETUSER", "default")
    command(b"$-1\r\n", "ACL", "GETUSER", "nobody")
    command(b"-ERR wrong number of arguments for 'acl|getuser' command\r\n", "ACL", "GETUSER")

    print("== ACL errors")
    command(b"-ERR wrong number of arguments for 'acl|whoami' command\r\n", "ACL", "WHOAMI", "x")
    command(b"-ERR wrong number of arguments for 'acl' command\r\n", "ACL")
    command(b"-ERR unknown subcommand 'BOGUS'. Try ACL HELP.\r\n", "ACL", "BOGUS")

finish()
