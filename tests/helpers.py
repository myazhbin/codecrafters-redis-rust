"""Helpers shared by the tests/*.py end-to-end tests.

Each test starts the server binary given as its first argument (by default, the one
./your_program.sh builds), talks to it over real client connections, and compares the
raw reply bytes with the expected RESP.
"""

import os
import socket
import subprocess
import sys
import time
from contextlib import contextmanager

# `cargo test` sets TEST_PORT to give each test its own port, so that they can run at once.
PORT = int(os.environ.get("TEST_PORT", "6379"))
ADDRESS = ("127.0.0.1", PORT)
# For a test playing a master, or running a replica. `cargo test`'s ports are all well below it.
OTHER_PORT = PORT + 100
DEFAULT_BINARY = "/tmp/codecrafters-build-redis-rust/release/codecrafters-redis"
OK = b"+OK\r\n"
QUEUED = b"+QUEUED\r\n"
WRONGTYPE = b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"
# The empty RDB file that the "Empty RDB transfer" stage links to, and how a master sends it:
# like a bulk string, but with no \r\n at the end.
EMPTY_RDB = bytes.fromhex(
    "524544495330303131fa0972656469732d76657205372e322e30fa0a72656469732d62697473c040"
    "fa056374696d65c26d08bc65fa08757365642d6d656dc2b0c41000fa08616f662d62617365c000ff"
    "f06e3bfec0ff5aa2"
)
RDB_TRANSFER = b"$%d\r\n%s" % (len(EMPTY_RDB), EMPTY_RDB)

failures = 0


def bulk(text):
    """A RESP bulk string."""
    data = text.encode()
    return b"$%d\r\n%s\r\n" % (len(data), data)


def encode(*args):
    """A RESP array of bulk strings: how commands are sent, and how some replies look."""
    return b"*%d\r\n" % len(args) + b"".join(bulk(arg) for arg in args)


def fullresync(offset=0):
    """A master's reply to PSYNC."""
    return b"+FULLRESYNC 8371b4fb1155b71f4a04d3e1bc3e18c4a990aeeb %d\r\n" % offset


def entry(entry_id, *fields):
    """One stream entry as XRANGE and XREAD reply with it."""
    return b"*2\r\n" + bulk(entry_id) + encode(*fields)


def streams(*items):
    """An XREAD reply, from (key, [entry, ...]) pairs."""
    out = b"*%d\r\n" % len(items)
    for key, entries in items:
        out += b"*2\r\n" + bulk(key) + b"*%d\r\n" % len(entries) + b"".join(entries)
    return out


class Client:
    """One client connection to the server. A test playing a master instead passes in the
    socket it accepted from a replica, and uses this as its end of that connection."""

    def __init__(self, sock=None):
        self.sock = sock or socket.create_connection(ADDRESS)

    def send(self, *args):
        self.sock.sendall(encode(*args))

    def read(self, size, timeout):
        """Reads up to `size` bytes, stopping early at the timeout or end of stream."""
        data = b""
        deadline = time.monotonic() + timeout
        while len(data) < size:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                break
            self.sock.settimeout(remaining)
            try:
                chunk = self.sock.recv(size - len(data))
            except socket.timeout:
                break
            if not chunk:
                break
            data += chunk
        return data

    def close(self):
        self.sock.close()


def check(label, ok, detail=""):
    global failures
    print(("PASS  " if ok else "FAIL  ") + label)
    if not ok:
        print("        " + detail)
        failures += 1


def expect(client, want, label, timeout=1.0):
    """Checks that the next bytes `client` receives are `want`."""
    got = client.read(len(want), timeout)
    check(label, got == want, f"want {want!r}, got {got!r}")


def expect_nothing(client, label, wait=0.3):
    """Checks that `client` receives nothing for `wait` seconds."""
    got = client.read(1, wait)
    check(label, got == b"", f"got unexpected {got!r}")


def run(client, want, *args):
    """Sends a command on `client` and checks the reply."""
    client.send(*args)
    expect(client, want, " ".join(args))


def command(want, *args):
    """Runs one command on a new connection and checks the reply."""
    client = Client()
    run(client, want, *args)
    client.close()


def port_open(port):
    try:
        socket.create_connection(("127.0.0.1", port)).close()
        return True
    except OSError:
        return False


@contextmanager
def running_server(*extra_args, port=PORT):
    """Runs the server, with any extra arguments, for the duration of the `with` block."""
    binary = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_BINARY
    if not os.access(binary, os.X_OK):
        sys.exit(f"No server binary at {binary}; build it with ./your_program.sh, or use cargo test.")
    if port_open(port):
        sys.exit(f"Something is already listening on port {port}; stop it before running the tests.")
    # On the default port, the server starts without arguments, so the default gets tested.
    args = [binary] if port == 6379 else [binary, "--port", str(port)]
    server = subprocess.Popen([*args, *extra_args], stdout=subprocess.DEVNULL)
    try:
        for _ in range(100):
            if port_open(port):
                break
            time.sleep(0.05)
        yield
    finally:
        server.terminate()
        server.wait()


def connect_replica(offset=0):
    """A new connection that has gone through the handshake with the server, as a replica
    would. `offset` is the replication offset the server should say it's at."""
    replica = Client()
    run(replica, b"+PONG\r\n", "PING")
    run(replica, OK, "REPLCONF", "listening-port", "6380")
    run(replica, OK, "REPLCONF", "capa", "psync2")
    run(replica, fullresync(offset), "PSYNC", "?", "-1")
    expect(replica, RDB_TRANSFER, "the replica receives an empty RDB file")
    return replica


@contextmanager
def replica_of_test():
    """Runs the server as a replica with the test as its master, and gives the test the
    master's end of the connection the replica makes."""
    with socket.create_server(("127.0.0.1", OTHER_PORT)) as listener:
        listener.settimeout(2.0)
        with running_server("--replicaof", f"localhost {OTHER_PORT}"):
            yield Client(listener.accept()[0])


def answer_handshake(master):
    """Plays the master's part in the handshake with a replica, up to receiving PSYNC."""
    expect(master, encode("PING"), "the master receives PING")
    master.sock.sendall(b"+PONG\r\n")
    expect(master, encode("REPLCONF", "listening-port", str(PORT)), "then REPLCONF listening-port")
    master.sock.sendall(OK)
    expect(master, encode("REPLCONF", "capa", "psync2"), "then REPLCONF capa psync2")
    master.sock.sendall(OK)
    expect(master, encode("PSYNC", "?", "-1"), "then PSYNC ? -1")


def stays_quiet(connection, wait=0.3):
    """Whether the other end neither sends anything nor hangs up for `wait` seconds."""
    connection.sock.settimeout(wait)
    try:
        connection.sock.recv(1)
    except socket.timeout:
        return True
    return False


def hangs_up(connection, wait=1.0):
    """Whether the other end hangs up within `wait` seconds, having sent nothing more."""
    connection.sock.settimeout(wait)
    try:
        return connection.sock.recv(1) == b""
    except socket.timeout:
        return False


def finish():
    """Prints the summary and exits, failing if any check failed."""
    print(f"{failures} failure(s)")
    sys.exit(1 if failures else 0)
