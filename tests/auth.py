#!/usr/bin/env python3
"""Authentication, from the "Respond to ACL WHOAMI" stage on: users, passwords and AUTH,
using several clients."""

from helpers import OK, Client, bulk, command, encode, finish, run, running_server

# The SHA-256 hashes of "mypassword", "second" and "secret", as Redis stores passwords.
MYPASSWORD = "89e01536ac207279409d4de1e5253e01f4a1769e696db0d6062ca9b8f56767c8"
SECOND = "16367aacb67a4a017c8da8ab95682ccb390863780f7114dda0a0e0c55644c7c4"
SECRET = "2bb80d537b1da3e38bd30361aa855686bde0eacd7162fef6a25fe97bf527a25b"
WRONGPASS = b"-WRONGPASS invalid username-password pair or user is disabled.\r\n"


def getuser(flags, passwords):
    """ACL GETUSER's reply for a user with `flags` and the password hashes `passwords`."""
    return b"*4\r\n" + bulk("flags") + encode(*flags) + bulk("passwords") + encode(*passwords)

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
    command(b"-ERR wrong number of arguments for 'acl|setuser' command\r\n", "ACL", "SETUSER")

# Setting a password for the default user changes things for new connections, so each test
# that does gets a server of its own, and talks to it on a connection opened beforehand.
with running_server():
    print("== stage test: ACL SETUSER >password sets a password, which replaces nopass")
    client = Client()
    run(client, getuser(["nopass"], []), "ACL", "GETUSER", "default")
    run(client, OK, "ACL", "SETUSER", "default", ">mypassword")
    run(client, getuser([], [MYPASSWORD]), "ACL", "GETUSER", "default")

    print("== more passwords, removing them, nopass and resetpass")
    run(client, OK, "ACL", "SETUSER", "default", ">second", ">mypassword")
    run(client, getuser([], [MYPASSWORD, SECOND]), "ACL", "GETUSER", "default")
    run(client, OK, "ACL", "SETUSER", "default", "<mypassword")
    run(client, getuser([], [SECOND]), "ACL", "GETUSER", "default")
    run(client, OK, "ACL", "SETUSER", "default", "#" + MYPASSWORD)
    run(client, getuser([], [SECOND, MYPASSWORD]), "ACL", "GETUSER", "default")
    run(client, OK, "ACL", "SETUSER", "default", "!" + SECOND)
    run(client, getuser([], [MYPASSWORD]), "ACL", "GETUSER", "default")
    run(client, OK, "ACL", "SETUSER", "default", "nopass")
    run(client, getuser(["nopass"], []), "ACL", "GETUSER", "default")
    run(client, OK, "ACL", "SETUSER", "default", "resetpass")
    run(client, getuser([], []), "ACL", "GETUSER", "default")

    print("== a rule that can't be applied stops them all")
    run(client, b"-ERR Error in ACL SETUSER modifier 'bogus': Syntax error\r\n",
        "ACL", "SETUSER", "default", ">x", "bogus")
    run(client, b"-ERR Error in ACL SETUSER modifier '<nope': The password you are trying to remove "
                b"from the user does not exist\r\n", "ACL", "SETUSER", "default", "<nope")
    run(client, getuser([], []), "ACL", "GETUSER", "default")

    print("== SETUSER creates a user that isn't there")
    run(client, OK, "ACL", "SETUSER", "alice", ">secret")
    run(client, getuser([], [SECRET]), "ACL", "GETUSER", "alice")

with running_server():
    print("== with nopass, any password will do, but not one on its own")
    command(OK, "AUTH", "default", "anything")
    command(b"-ERR AUTH <password> called without any password configured for the default user. "
            b"Are you sure your configuration is correct?\r\n", "AUTH", "anything")

    print("== stage test: AUTH checks the password")
    client = Client()
    run(client, OK, "ACL", "SETUSER", "default", ">mypassword")
    run(client, WRONGPASS, "AUTH", "default", "wrongpassword")
    run(client, OK, "AUTH", "default", "mypassword")

    print("== the password alone means the default user; other users; errors")
    run(client, OK, "AUTH", "mypassword")
    run(client, WRONGPASS, "AUTH", "wrongpassword")
    run(client, WRONGPASS, "AUTH", "nobody", "mypassword")
    run(client, OK, "ACL", "SETUSER", "alice", ">secret")
    run(client, WRONGPASS, "AUTH", "alice", "mypassword")
    run(client, OK, "AUTH", "alice", "secret")
    run(client, b"-ERR wrong number of arguments for 'auth' command\r\n", "AUTH")
    run(client, b"-ERR syntax error\r\n", "AUTH", "a", "b", "c")

finish()
