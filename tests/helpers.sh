# Helpers shared by the tests/*.sh end-to-end tests, which each source this file.
#
# Sourcing it starts the server (the binary given as the test's first argument, by
# default the one ./your_program.sh builds) and stops it when the test exits. Each
# `check` sends one command over a new connection with nc and compares the raw reply,
# with \r\n spelled out, against the expected one.

set -u
bin=${1:-/tmp/codecrafters-build-redis-rust/release/codecrafters-redis}
# A test can set `port` before sourcing this file, to start the server with --port.
# Otherwise it starts without arguments, so that the default port gets tested too.
port=${port:-6379}
failures=0

wrongtype='-WRONGTYPE Operation against a key holding the wrong kind of value\r\n'

# A command as clients send it: a RESP array of bulk strings.
resp() {
  printf '*%d\r\n' "$#"
  for arg in "$@"; do printf '$%d\r\n%s\r\n' "${#arg}" "$arg"; done
}

# The same encoding with \r\n spelled out, for writing expected replies.
array() {
  printf '*%d\\r\\n' "$#"
  for item in "$@"; do printf '$%d\\r\\n%s\\r\\n' "${#item}" "$item"; done
}

# One stream entry as XRANGE and XREAD reply with it: entry <id> <field> <value> ...
entry() {
  local id=$1
  shift
  printf '*2\\r\\n$%d\\r\\n%s\\r\\n' "${#id}" "$id"
  array "$@"
}

# One stream in an XREAD reply: stream <key> <entry>... (entries built with `entry`)
stream() {
  local key=$1
  shift
  printf '*2\\r\\n$%d\\r\\n%s\\r\\n*%d\\r\\n' "${#key}" "$key" "$#"
  printf '%s' "$@"
}

# Sends standard input to the server (or to the port given) over one connection and
# prints what comes back, with \r\n spelled out.
send() {
  nc -w 1 127.0.0.1 "${1:-$port}" | perl -pe 's/\r/\\r/g; s/\n/\\n/g'
}

# Waits up to 5 seconds for something to listen on the given port.
wait_for_port() {
  for _ in $(seq 50); do
    nc -z 127.0.0.1 "$1" 2>/dev/null && return
    sleep 0.1
  done
}

# report <label> <expected> <actual>
report() {
  if [ "$3" = "$2" ]; then
    echo "PASS  $1"
  else
    echo "FAIL  $1"
    echo "        want: $2"
    echo "        got:  $3"
    failures=$((failures + 1))
  fi
}

# check <expected reply> <command...>
check() {
  local want=$1
  shift
  report "$*" "$want" "$(resp "$@" | send)"
}

# Prints the summary and exits, failing if any check failed.
finish() {
  echo "$failures failure(s)"
  exit $((failures > 0))
}

if [ ! -x "$bin" ]; then
  echo "No server binary at $bin; tests/run.sh builds it before running the tests." >&2
  exit 1
fi
if nc -z 127.0.0.1 "$port" 2>/dev/null; then
  echo "Something is already listening on port $port; stop it before running the tests." >&2
  exit 1
fi
if [ "$port" = 6379 ]; then
  "$bin" >/dev/null &
else
  "$bin" --port "$port" >/dev/null &
fi
server=$!
trap 'kill $server 2>/dev/null' EXIT
wait_for_port "$port"
