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
DEFAULT_BINARY = "/tmp/codecrafters-build-redis-rust/release/codecrafters-redis"
OK = b"+OK\r\n"
QUEUED = b"+QUEUED\r\n"
WRONGTYPE = b"-WRONGTYPE Operation against a key holding the wrong kind of value\r\n"

failures = 0


def bulk(text):
    """A RESP bulk string."""
    data = text.encode()
    return b"$%d\r\n%s\r\n" % (len(data), data)


def encode(*args):
    """A RESP array of bulk strings: how commands are sent, and how some replies look."""
    return b"*%d\r\n" % len(args) + b"".join(bulk(arg) for arg in args)


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


def port_open():
    try:
        socket.create_connection(ADDRESS).close()
        return True
    except OSError:
        return False


@contextmanager
def running_server(*extra_args):
    """Runs the server, with any extra arguments, for the duration of the `with` block."""
    binary = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_BINARY
    if not os.access(binary, os.X_OK):
        sys.exit(f"No server binary at {binary}; build it with ./your_program.sh, or use cargo test.")
    if port_open():
        sys.exit(f"Something is already listening on port {PORT}; stop it before running the tests.")
    # On the default port, the server starts without arguments, so the default gets tested.
    args = [binary] if PORT == 6379 else [binary, "--port", str(PORT)]
    server = subprocess.Popen([*args, *extra_args], stdout=subprocess.DEVNULL)
    try:
        for _ in range(100):
            if port_open():
                break
            time.sleep(0.05)
        yield
    finally:
        server.terminate()
        server.wait()


def finish():
    """Prints the summary and exits, failing if any check failed."""
    print(f"{failures} failure(s)")
    sys.exit(1 if failures else 0)
