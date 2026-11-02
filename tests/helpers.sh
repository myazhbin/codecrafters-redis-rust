# Helpers shared by the tests/*.sh end-to-end tests, which each source this file.
#
# Sourcing it starts the server (the binary given as the test's first argument, by
# default the one ./your_program.sh builds) on port 6379 and stops it when the test
# exits. Each `check` sends one command over a new connection with nc and compares the
# raw reply, with \r\n spelled out, against the expected one.

set -u
bin=${1:-/tmp/codecrafters-build-redis-rust/release/codecrafters-redis}
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

# Sends standard input to the server over one connection and prints what comes back,
# with \r\n spelled out.
send() {
  nc -w 1 127.0.0.1 6379 | perl -pe 's/\r/\\r/g; s/\n/\\n/g'
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
if nc -z 127.0.0.1 6379 2>/dev/null; then
  echo "Something is already listening on port 6379; stop it before running the tests." >&2
  exit 1
fi
"$bin" >/dev/null &
server=$!
trap 'kill $server 2>/dev/null' EXIT
for _ in $(seq 50); do
  nc -z 127.0.0.1 6379 2>/dev/null && break
  sleep 0.1
done
