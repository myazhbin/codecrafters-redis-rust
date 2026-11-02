#![allow(unused_imports)]

mod resp;

use anyhow::{Result, bail};
use bytes::{Buf, BytesMut};
use resp::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, oneshot};

type Db = Arc<Mutex<Store>>;

#[derive(Default)]
struct Store {
    entries: HashMap<Vec<u8>, Entry>,
    /// Clients blocked in BLPOP on each key, longest-waiting first. Each one is sent the
    /// element it pops.
    blocked: HashMap<Vec<u8>, VecDeque<oneshot::Sender<Vec<u8>>>>,
    /// Lists pushed to by the command or transaction being run. As in Redis, clients
    /// blocked on them are served once it's done, so a transaction never sees that happen.
    pushed: Vec<Vec<u8>>,
    /// Notified whenever an entry is added to a stream, so that blocked XREADs look again.
    stream_added: Arc<Notify>,
    /// For WATCH: a counter for each key that has ever been watched, which goes up
    /// whenever the key is written to, deleted or expires.
    versions: HashMap<Vec<u8>, u64>,
    /// Whether the server was started as a replica, with `--replicaof`.
    is_replica: bool,
}

impl Store {
    /// Records a change to `key`, for anyone WATCHing it.
    fn touch(&mut self, key: &[u8]) {
        if let Some(version) = self.versions.get_mut(key) {
            *version += 1;
        }
    }

    /// Starts tracking changes to `key` for WATCH, and returns its current version.
    fn watch(&mut self, key: &[u8]) -> u64 {
        // A key that has already expired goes now, so that it doesn't count as a change.
        remove_if_expired(self, key);
        *self.versions.entry(key.to_vec()).or_insert(0)
    }

    /// Whether `key` has changed since WATCH saw it at `version`.
    fn has_changed(&mut self, key: &[u8], version: u64) -> bool {
        // Expiring counts as a change too, even if nothing has looked at the key since.
        remove_if_expired(self, key);
        self.versions.get(key) != Some(&version)
    }
}

struct Entry {
    value: Data,
    expires_at: Option<Instant>,
}

impl Entry {
    fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|at| at <= Instant::now())
    }
}

enum Data {
    String(Vec<u8>),
    List(VecDeque<Vec<u8>>),
    Stream(Vec<StreamEntry>),
}

struct StreamEntry {
    id: StreamId,
    /// The entry's field-value pairs, flattened: field1, value1, field2, value2, ...
    fields: Vec<Vec<u8>>,
}

impl StreamEntry {
    /// The entry as XRANGE and XREAD reply with it: its ID, then an array of its fields.
    fn to_value(&self) -> Value {
        let fields = self.fields.iter().cloned().map(Value::BulkString);
        Value::Array(vec![
            Value::BulkString(self.id.to_string().into_bytes()),
            Value::Array(fields.collect()),
        ])
    }
}

/// A stream entry ID, written `<ms>-<seq>`: a Unix time in milliseconds, then a sequence
/// number that tells apart entries from the same millisecond. IDs sort in that order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct StreamId {
    ms: u64,
    seq: u64,
}

impl StreamId {
    const MIN: Self = Self { ms: 0, seq: 0 };
    const MAX: Self = Self {
        ms: u64::MAX,
        seq: u64::MAX,
    };
}

impl std::fmt::Display for StreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}-{}", self.ms, self.seq)
    }
}

/// Settings from the command line.
struct Config {
    /// The port to listen on, set with `--port`.
    port: u16,
    /// The host and port of the master to replicate, set with `--replicaof`. A server
    /// without one is a master itself.
    replicaof: Option<(String, u16)>,
}

/// Reads the command-line arguments that follow the program's name:
/// `[--port <port>] [--replicaof "<host> <port>"]`.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Config, String> {
    let mut config = Config {
        port: 6379,
        replicaof: None,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                let port = args.next().ok_or("--port needs a port number")?;
                config.port = port.parse().map_err(|_| format!("invalid port: {port}"))?;
            }
            "--replicaof" => {
                let master = args.next().ok_or("--replicaof needs a host and port")?;
                let invalid = format!("invalid master address: {master}");
                config.replicaof = Some(parse_master(&master).ok_or(invalid)?);
            }
            _ => return Err(format!("unknown argument: {arg}")),
        }
    }
    Ok(config)
}

/// Parses the value of `--replicaof`: a host and a port, with whitespace between them.
fn parse_master(arg: &str) -> Option<(String, u16)> {
    let parts: Vec<&str> = arg.split_whitespace().collect();
    let [host, port] = parts[..] else {
        return None;
    };
    Some((host.to_string(), port.parse().ok()?))
}

#[tokio::main]
async fn main() {
    let config = parse_args(std::env::args().skip(1)).unwrap_or_else(|msg| {
        eprintln!("{msg}");
        std::process::exit(1);
    });
    let listener = TcpListener::bind(("127.0.0.1", config.port)).await.unwrap();
    let db = Db::new(Mutex::new(Store {
        is_replica: config.replicaof.is_some(),
        ..Store::default()
    }));

    if let Some((host, master_port)) = config.replicaof {
        tokio::spawn(async move {
            if let Err(e) = connect_to_master(&host, master_port, config.port).await {
                println!("master connection error: {}", e);
            }
        });
    }

    loop {
        let stream = listener.accept().await;

        match stream {
            Ok((stream, _)) => {
                println!("accepted new connection");

                let db = db.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, db).await {
                        println!("connection error: {}", e);
                    }
                });
            }
            Err(e) => {
                println!("error: {}", e);
            }
        }
    }
}

/// Connects a replica to its master and goes through the handshake, waiting for the
/// reply to each command before sending the next. There's no more to it yet, so the
/// replica then just stays connected until the master hangs up, ignoring what it sends.
async fn connect_to_master(host: &str, port: u16, listening_port: u16) -> Result<()> {
    let mut master = TcpStream::connect((host, port)).await?;
    let mut buf = BytesMut::with_capacity(512);
    let listening_port = listening_port.to_string();
    let handshake = [
        vec!["PING"],
        vec!["REPLCONF", "listening-port", &listening_port],
        vec!["REPLCONF", "capa", "psync2"],
    ];
    for command in handshake {
        let args = command
            .iter()
            .map(|arg| Value::BulkString(arg.as_bytes().to_vec()));
        let request = Value::Array(args.collect()).encode();
        master.write_all(&request).await?;
        if let Value::Error(msg) = read_reply(&mut master, &mut buf).await? {
            bail!("the reply to {} was: {msg}", command.join(" "));
        }
    }
    tokio::io::copy(&mut master, &mut tokio::io::sink()).await?;
    Ok(())
}

/// Reads the master's next reply, however many reads it takes for all of it to arrive.
async fn read_reply(master: &mut TcpStream, buf: &mut BytesMut) -> Result<Value> {
    loop {
        if let Some((reply, len)) = Value::parse(buf)? {
            buf.advance(len);
            return Ok(reply);
        }
        if master.read_buf(buf).await? == 0 {
            bail!("the master hung up during the handshake");
        }
    }
}

/// What the server keeps track of for each client connection.
#[derive(Default)]
struct Session {
    /// The commands queued for EXEC while the client is inside a MULTI transaction.
    transaction: Option<Vec<Vec<Vec<u8>>>>,
    /// The keys the client is WATCHing, each with its version at the time. EXEC calls the
    /// transaction off if any of them has changed since.
    watched: Vec<(Vec<u8>, u64)>,
}

async fn handle_connection(mut stream: TcpStream, db: Db) -> Result<()> {
    let mut buf = BytesMut::with_capacity(512);
    let mut session = Session::default();
    loop {
        // A read can end partway through a command or hold several pipelined ones,
        // so answer every complete command buffered so far, then read more.
        while let Some((request, len)) = Value::parse(&buf)? {
            buf.advance(len);
            let command = handle_request(request, &db, &mut session);
            tokio::pin!(command);
            // BLPOP can wait indefinitely, so keep reading meanwhile. If the client hangs
            // up, the wait is dropped rather than later taking an element nobody receives.
            // `biased` checks the command first, so a ready reply is never skipped.
            let reply = loop {
                tokio::select! {
                    biased;
                    reply = &mut command => break reply,
                    read = stream.read_buf(&mut buf) => {
                        if read? == 0 {
                            return Ok(());
                        }
                    }
                }
            };
            stream.write_all(&reply.encode()).await?;
        }

        if stream.read_buf(&mut buf).await? == 0 {
            return Ok(());
        }
    }
}

/// Handles a client's request. Inside a MULTI transaction, commands other than EXEC,
/// DISCARD, MULTI and WATCH are queued until EXEC runs them all.
async fn handle_request(request: Value, db: &Db, session: &mut Session) -> Value {
    let Some(command) = command_args(request) else {
        return Value::Error("ERR expected an array of bulk strings".into());
    };
    let name = command.first().map(|name| name.to_ascii_uppercase());
    match name.as_deref() {
        Some(b"MULTI") if command.len() == 1 => {
            if session.transaction.is_some() {
                return Value::Error("ERR MULTI calls can not be nested".into());
            }
            session.transaction = Some(Vec::new());
            Value::SimpleString("OK".into())
        }
        Some(b"EXEC") if command.len() == 1 => {
            let Some(queued) = session.transaction.take() else {
                return Value::Error("ERR EXEC without MULTI".into());
            };
            let watched = std::mem::take(&mut session.watched);
            // Holding the lock throughout means no other client's command can run in
            // between: the transaction happens all at once.
            let mut db = db.lock().unwrap();
            // If a WATCHed key has changed since, the transaction is called off.
            if watched
                .iter()
                .any(|(key, version)| db.has_changed(key, *version))
            {
                return Value::NullArray;
            }
            let replies = queued
                .iter()
                .map(|command| execute(&mut db, command))
                .collect();
            serve_blocked(&mut db);
            Value::Array(replies)
        }
        Some(b"DISCARD") if command.len() == 1 => match session.transaction.take() {
            Some(_) => {
                session.watched.clear();
                Value::SimpleString("OK".into())
            }
            None => Value::Error("ERR DISCARD without MULTI".into()),
        },
        Some(b"WATCH") if command.len() > 1 => {
            if session.transaction.is_some() {
                return Value::Error("ERR WATCH inside MULTI is not allowed".into());
            }
            let mut db = db.lock().unwrap();
            for key in &command[1..] {
                session.watched.push((key.clone(), db.watch(key)));
            }
            Value::SimpleString("OK".into())
        }
        // Inside a transaction, UNWATCH is queued like other commands instead.
        Some(b"UNWATCH") if command.len() == 1 && session.transaction.is_none() => {
            session.watched.clear();
            Value::SimpleString("OK".into())
        }
        Some(b"MULTI") => wrong_arity("multi"),
        Some(b"EXEC") => wrong_arity("exec"),
        Some(b"DISCARD") => wrong_arity("discard"),
        Some(b"WATCH") => wrong_arity("watch"),
        _ => match &mut session.transaction {
            Some(queued) => {
                queued.push(command);
                Value::SimpleString("QUEUED".into())
            }
            None => handle_command(&command, db).await,
        },
    }
}

/// Runs a command outside a transaction. BLPOP and XREAD BLOCK can wait here for data to
/// arrive, without holding the lock; everything else runs straight through `execute`.
async fn handle_command(command: &[Vec<u8>], db: &Db) -> Value {
    if let [name, key, timeout] = command
        && name.eq_ignore_ascii_case(b"BLPOP")
        && let Ok(timeout) = parse_timeout(timeout)
    {
        return blocking_pop(db, key, timeout).await;
    }
    if let [name, args @ ..] = command
        && name.eq_ignore_ascii_case(b"XREAD")
        && let Ok(args) = parse_xread_args(args)
        && let Some(timeout) = args.block
    {
        return blocking_read(db, args.keys, args.ids, timeout).await;
    }
    let mut db = db.lock().unwrap();
    let reply = execute(&mut db, command);
    serve_blocked(&mut db);
    reply
}

/// Runs a command against the locked database. EXEC runs a whole transaction through
/// this without letting go of the lock in between.
fn execute(db: &mut Store, command: &[Vec<u8>]) -> Value {
    let Some((name, args)) = command.split_first() else {
        return Value::Error("ERR empty command".into());
    };

    match name.to_ascii_uppercase().as_slice() {
        b"PING" => Value::SimpleString("PONG".into()),
        b"ECHO" => match args {
            [message] => Value::BulkString(message.clone()),
            _ => wrong_arity("echo"),
        },
        b"SET" => match args {
            [key, value, options @ ..] => match parse_expiry(options) {
                Ok(expires_at) => {
                    let entry = Entry {
                        value: Data::String(value.clone()),
                        expires_at,
                    };
                    db.entries.insert(key.clone(), entry);
                    db.touch(key);
                    Value::SimpleString("OK".into())
                }
                Err(msg) => Value::Error(msg.into()),
            },
            _ => wrong_arity("set"),
        },
        b"GET" => match args {
            [key] => match live_value(db, key) {
                Some(Data::String(value)) => Value::BulkString(value.clone()),
                Some(_) => wrong_type(),
                None => Value::NullBulkString,
            },
            _ => wrong_arity("get"),
        },
        b"RPUSH" => match args {
            [key, elements @ ..] if !elements.is_empty() => {
                push_to_list(db, key, elements, VecDeque::push_back)
            }
            _ => wrong_arity("rpush"),
        },
        b"LPUSH" => match args {
            [key, elements @ ..] if !elements.is_empty() => {
                push_to_list(db, key, elements, VecDeque::push_front)
            }
            _ => wrong_arity("lpush"),
        },
        b"LRANGE" => match args {
            [key, start, stop] => {
                let (Some(start), Some(stop)) = (parse_number(start), parse_number(stop)) else {
                    return Value::Error("ERR value is not an integer or out of range".into());
                };
                match live_value(db, key) {
                    Some(Data::List(list)) => Value::Array(list_range(list, start, stop)),
                    Some(_) => wrong_type(),
                    None => Value::Array(vec![]),
                }
            }
            _ => wrong_arity("lrange"),
        },
        b"LLEN" => match args {
            [key] => match live_value(db, key) {
                Some(Data::List(list)) => Value::Integer(list.len() as i64),
                Some(_) => wrong_type(),
                None => Value::Integer(0),
            },
            _ => wrong_arity("llen"),
        },
        b"LPOP" => match args {
            [key] => pop_from_list(db, key, None),
            [key, count] => match parse_number(count) {
                Some(count) => pop_from_list(db, key, Some(count)),
                None => Value::Error("ERR value is out of range, must be positive".into()),
            },
            _ => wrong_arity("lpop"),
        },
        b"BLPOP" => match args {
            // Only inside a transaction does a valid BLPOP get here (`handle_command` runs
            // it otherwise), and there it doesn't wait, as in Redis.
            [key, timeout] => match parse_timeout(timeout) {
                Ok(_) => pop_now(db, key),
                Err(msg) => Value::Error(msg.into()),
            },
            _ => wrong_arity("blpop"),
        },
        b"TYPE" => match args {
            [key] => {
                let kind = match live_value(db, key) {
                    Some(Data::String(_)) => "string",
                    Some(Data::List(_)) => "list",
                    Some(Data::Stream(_)) => "stream",
                    None => "none",
                };
                Value::SimpleString(kind.into())
            }
            _ => wrong_arity("type"),
        },
        b"XADD" => match args {
            [key, id, fields @ ..] if !fields.is_empty() && fields.len().is_multiple_of(2) => {
                add_to_stream(db, key, id, fields)
            }
            _ => wrong_arity("xadd"),
        },
        b"XRANGE" => match args {
            [key, start, end] => {
                let (Some(start), Some(end)) = (
                    parse_range_bound(start, 0),
                    parse_range_bound(end, u64::MAX),
                ) else {
                    return invalid_stream_id();
                };
                match live_value(db, key) {
                    Some(Data::Stream(stream)) => Value::Array(stream_range(stream, start, end)),
                    Some(_) => wrong_type(),
                    None => Value::Array(vec![]),
                }
            }
            _ => wrong_arity("xrange"),
        },
        b"XREAD" => match args {
            // XREAD BLOCK only gets here inside a transaction (`handle_command` runs it
            // otherwise), and there it doesn't wait, as in Redis.
            [_, _, _, ..] => match parse_xread_args(args) {
                Ok(XreadArgs { keys, ids, .. }) => read_now(db, keys, ids),
                Err(msg) => Value::Error(msg.into()),
            },
            _ => wrong_arity("xread"),
        },
        b"INCR" => match args {
            [key] => increment(db, key),
            _ => wrong_arity("incr"),
        },
        // `handle_request` handles UNWATCH outside a transaction. Inside one, it's left
        // with nothing to do: EXEC has already stopped watching by the time it runs.
        b"UNWATCH" => match args {
            [] => Value::SimpleString("OK".into()),
            _ => wrong_arity("unwatch"),
        },
        b"INFO" => info(db, args),
        _ => Value::Error(format!(
            "ERR unknown command '{}'",
            String::from_utf8_lossy(name)
        )),
    }
}

fn wrong_arity(command: &str) -> Value {
    Value::Error(format!(
        "ERR wrong number of arguments for '{command}' command"
    ))
}

fn wrong_type() -> Value {
    Value::Error("WRONGTYPE Operation against a key holding the wrong kind of value".into())
}

fn invalid_stream_id() -> Value {
    Value::Error("ERR Invalid stream ID specified as stream command argument".into())
}

/// Parses the options after `SET key value`. So far only an `EX seconds` or
/// `PX milliseconds` expiry is supported.
fn parse_expiry(options: &[Vec<u8>]) -> Result<Option<Instant>, &'static str> {
    let (unit, amount) = match options {
        [] => return Ok(None),
        [unit, amount] => (unit.to_ascii_uppercase(), amount),
        _ => return Err("ERR syntax error"),
    };
    let to_duration: fn(u64) -> Duration = match unit.as_slice() {
        b"EX" => Duration::from_secs,
        b"PX" => Duration::from_millis,
        _ => return Err("ERR syntax error"),
    };
    let Some(amount) = parse_number::<u64>(amount) else {
        return Err("ERR value is not an integer or out of range");
    };
    // checked_add so that an absurdly long expiry is an error rather than a panic.
    match Instant::now().checked_add(to_duration(amount)) {
        Some(expires_at) if amount > 0 => Ok(Some(expires_at)),
        _ => Err("ERR invalid expire time in 'set' command"),
    }
}

/// Parses a numeric argument, such as an expiry time, a list index or a timeout.
fn parse_number<T: std::str::FromStr>(arg: &[u8]) -> Option<T> {
    std::str::from_utf8(arg).ok()?.parse().ok()
}

/// Parses BLPOP's timeout: a number of seconds, where 0 means wait forever.
fn parse_timeout(arg: &[u8]) -> Result<Option<Duration>, &'static str> {
    let Some(secs) = parse_number::<f64>(arg) else {
        return Err("ERR timeout is not a float or out of range");
    };
    if secs < 0.0 {
        return Err("ERR timeout is negative");
    }
    if secs == 0.0 {
        return Ok(None);
    }
    match Duration::try_from_secs_f64(secs) {
        Ok(timeout) => Ok(Some(timeout)),
        Err(_) => Err("ERR timeout is out of range"),
    }
}

/// Removes `key` if it has expired. Expired keys are only removed once something tries
/// to read them, so this runs before every lookup.
fn remove_if_expired(db: &mut Store, key: &[u8]) {
    if db.entries.get(key).is_some_and(Entry::is_expired) {
        db.entries.remove(key);
        db.touch(key);
    }
}

/// Looks up the value at `key`, treating an expired key as missing.
fn live_value<'a>(db: &'a mut Store, key: &[u8]) -> Option<&'a mut Data> {
    remove_if_expired(db, key);
    db.entries.get_mut(key).map(|entry| &mut entry.value)
}

/// Like `live_value`, but first stores `new` at `key` if the key is missing or expired.
fn live_value_or_insert<'a>(db: &'a mut Store, key: &[u8], new: Data) -> &'a mut Data {
    remove_if_expired(db, key);
    let entry = db.entries.entry(key.to_vec()).or_insert(Entry {
        value: new,
        expires_at: None,
    });
    &mut entry.value
}

/// Pushes `elements` onto the list at `key` one at a time (so LPUSH a b c gives c b a),
/// creating the list if needed. Replies with the list's new length.
fn push_to_list(
    db: &mut Store,
    key: &[u8],
    elements: &[Vec<u8>],
    push: fn(&mut VecDeque<Vec<u8>>, Vec<u8>),
) -> Value {
    let Data::List(list) = live_value_or_insert(db, key, Data::List(VecDeque::new())) else {
        return wrong_type();
    };
    for element in elements {
        push(list, element.clone());
    }
    let len = list.len();
    db.touch(key);
    // Clients blocked on the list get their elements once the command is done.
    db.pushed.push(key.to_vec());
    Value::Integer(len as i64)
}

/// Removes elements from the front of the list at `key`: just the first one as a bulk
/// string, or up to `count` of them as an array.
fn pop_from_list(db: &mut Store, key: &[u8], count: Option<usize>) -> Value {
    let list = match live_value(db, key) {
        Some(Data::List(list)) => list,
        Some(_) => return wrong_type(),
        None if count.is_some() => return Value::NullArray,
        None => return Value::NullBulkString,
    };
    let reply = match count {
        None => list
            .pop_front()
            .map_or(Value::NullBulkString, Value::BulkString),
        Some(count) => {
            let count = count.min(list.len());
            Value::Array(list.drain(..count).map(Value::BulkString).collect())
        }
    };
    // Redis deletes a list along with its last element, so the key reads as missing again.
    if list.is_empty() {
        db.entries.remove(key);
    }
    // Stored lists are never empty, so anything but a count of 0 popped something.
    if count != Some(0) {
        db.touch(key);
    }
    reply
}

/// Hands elements from the lists pushed to by the command or transaction just run over
/// to the clients blocked on them, longest-waiting first.
fn serve_blocked(db: &mut Store) {
    for key in std::mem::take(&mut db.pushed) {
        let Some(waiters) = db.blocked.get_mut(&key) else {
            continue;
        };
        // Forget clients that have stopped waiting: they timed out or hung up.
        waiters.retain(|waiter| !waiter.is_closed());
        let mut served = false;
        if let Some(Data::List(list)) = db.entries.get_mut(&key).map(|entry| &mut entry.value) {
            while !list.is_empty() {
                let Some(waiter) = waiters.pop_front() else {
                    break;
                };
                let element = list.pop_front().unwrap();
                match waiter.send(element) {
                    Ok(()) => served = true,
                    // A client can still stop waiting after the retain above, in which
                    // case its element goes back to the list for the next one.
                    Err(element) => list.push_front(element),
                }
            }
            if list.is_empty() {
                db.entries.remove(&key);
            }
        }
        if waiters.is_empty() {
            db.blocked.remove(&key);
        }
        if served {
            db.touch(&key);
        }
    }
}

/// Pops the first element of the list at `key`, waiting for one to be pushed if there
/// isn't one: for up to `timeout`, or forever if there's no timeout.
async fn blocking_pop(db: &Db, key: &[u8], timeout: Option<Duration>) -> Value {
    let mut receiver = {
        let mut db = db.lock().unwrap();
        match pop_now(&mut db, key) {
            // Nothing to pop yet, so wait in line for a push.
            Value::NullArray => {}
            reply => return reply,
        }
        let (sender, receiver) = oneshot::channel();
        let waiters = db.blocked.entry(key.to_vec()).or_default();
        // Forget clients that have stopped waiting while we're here.
        waiters.retain(|waiter| !waiter.is_closed());
        waiters.push_back(sender);
        receiver
    };
    let element = match timeout {
        None => receiver.await.ok(),
        Some(timeout) => match tokio::time::timeout(timeout, &mut receiver).await {
            Ok(received) => received.ok(),
            // An element may have been handed over just as time ran out.
            Err(_) => {
                receiver.close();
                receiver.try_recv().ok()
            }
        },
    };
    element.map_or(Value::NullArray, |element| blpop_reply(key, element))
}

/// BLPOP without the waiting: pops the first element of the list at `key`, or replies
/// with a null array if there's no list there.
fn pop_now(db: &mut Store, key: &[u8]) -> Value {
    let list = match live_value(db, key) {
        Some(Data::List(list)) => list,
        Some(_) => return wrong_type(),
        None => return Value::NullArray,
    };
    let Some(element) = list.pop_front() else {
        return Value::NullArray;
    };
    if list.is_empty() {
        db.entries.remove(key);
    }
    db.touch(key);
    blpop_reply(key, element)
}

/// BLPOP's reply for `element`, popped from the list at `key`.
fn blpop_reply(key: &[u8], element: Vec<u8>) -> Value {
    Value::Array(vec![
        Value::BulkString(key.to_vec()),
        Value::BulkString(element),
    ])
}

/// Returns the elements from `start` to `stop` inclusive. Negative indexes count back
/// from the end, and any part of the range that falls outside the list is ignored.
fn list_range(list: &VecDeque<Vec<u8>>, start: i64, stop: i64) -> Vec<Value> {
    let len = list.len() as i64;
    let resolve = |index: i64| if index < 0 { len + index } else { index };
    let (start, stop) = (resolve(start).max(0), resolve(stop).min(len - 1));
    if start > stop {
        return Vec::new();
    }
    list.range(start as usize..=stop as usize)
        .map(|element| Value::BulkString(element.clone()))
        .collect()
}

/// Appends an entry to the stream at `key`, creating the stream if needed, and replies
/// with the new entry's ID.
fn add_to_stream(db: &mut Store, key: &[u8], id: &[u8], fields: &[Vec<u8>]) -> Value {
    let Some((ms, seq)) = parse_xadd_id(id) else {
        return invalid_stream_id();
    };
    // Rejected before the stream is created, so that this can't leave an empty one behind.
    if (ms, seq) == (Some(0), Some(0)) {
        return Value::Error("ERR The ID specified in XADD must be greater than 0-0".into());
    }
    let Data::Stream(stream) = live_value_or_insert(db, key, Data::Stream(Vec::new())) else {
        return wrong_type();
    };
    // A new stream counts as ending at 0-0, below every ID it can be given.
    let last = stream.last().map_or(StreamId::MIN, |entry| entry.id);
    let ms = ms.unwrap_or_else(|| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        // If the clock has gone backwards, carry on from the last entry's time.
        (now.as_millis() as u64).max(last.ms)
    });
    let seq = match seq {
        Some(seq) => seq,
        // Within the same millisecond, carry on from the last entry's sequence number.
        None if ms == last.ms => last.seq.saturating_add(1),
        None => 0,
    };
    let id = StreamId { ms, seq };
    if id <= last {
        return Value::Error(
            "ERR The ID specified in XADD is equal or smaller than the target stream top item"
                .into(),
        );
    }
    stream.push(StreamEntry {
        id,
        fields: fields.to_vec(),
    });
    db.touch(key);
    db.stream_added.notify_waiters();
    Value::BulkString(id.to_string().into_bytes())
}

/// Parses XADD's ID argument into its time and sequence number, with `None` for a part
/// to generate: `<ms>-<seq>`, `<ms>` (sequence 0), `<ms>-*` or `*`.
fn parse_xadd_id(arg: &[u8]) -> Option<(Option<u64>, Option<u64>)> {
    if arg == b"*" {
        return Some((None, None));
    }
    if let Some(ms) = arg.strip_suffix(b"-*") {
        return Some((Some(parse_number(ms)?), None));
    }
    let id = parse_stream_id(arg, 0)?;
    Some((Some(id.ms), Some(id.seq)))
}

/// Parses an XRANGE bound: an ID, or `-` or `+` for the lowest or highest ID possible.
fn parse_range_bound(arg: &[u8], missing_seq: u64) -> Option<StreamId> {
    match arg {
        b"-" => Some(StreamId::MIN),
        b"+" => Some(StreamId::MAX),
        _ => parse_stream_id(arg, missing_seq),
    }
}

/// Parses a stream ID: `<ms>-<seq>`, or just `<ms>` with `missing_seq` as the sequence number.
fn parse_stream_id(arg: &[u8], missing_seq: u64) -> Option<StreamId> {
    let arg = std::str::from_utf8(arg).ok()?;
    let (ms, seq) = match arg.split_once('-') {
        Some((ms, seq)) => (ms.parse().ok()?, seq.parse().ok()?),
        None => (arg.parse().ok()?, missing_seq),
    };
    Some(StreamId { ms, seq })
}

/// Returns the entries with IDs from `start` to `end` inclusive.
fn stream_range(stream: &[StreamEntry], start: StreamId, end: StreamId) -> Vec<Value> {
    // Entries are kept in ID order, so a binary search finds where the range begins.
    let first = stream.partition_point(|entry| entry.id < start);
    stream[first..]
        .iter()
        .take_while(|entry| entry.id <= end)
        .map(StreamEntry::to_value)
        .collect()
}

/// XREAD's arguments, as parsed by `parse_xread_args`.
struct XreadArgs<'a> {
    keys: &'a [Vec<u8>],
    ids: &'a [Vec<u8>],
    /// How long to wait for new entries if there are none yet: `None` without BLOCK, and
    /// `Some(None)` to wait forever (BLOCK 0).
    block: Option<Option<Duration>>,
}

/// Parses XREAD's arguments: `[BLOCK <milliseconds>] STREAMS <key>... <id>...`.
fn parse_xread_args(args: &[Vec<u8>]) -> Result<XreadArgs<'_>, &'static str> {
    let (block, args) = match args {
        [option, ms, rest @ ..] if option.eq_ignore_ascii_case(b"BLOCK") => {
            let Some(ms) = parse_number::<i64>(ms) else {
                return Err("ERR timeout is not an integer or out of range");
            };
            if ms < 0 {
                return Err("ERR timeout is negative");
            }
            let timeout = (ms > 0).then(|| Duration::from_millis(ms as u64));
            (Some(timeout), rest)
        }
        _ => (None, args),
    };
    let streams = match args {
        [option, streams @ ..]
            if option.eq_ignore_ascii_case(b"STREAMS") && !streams.is_empty() =>
        {
            streams
        }
        // Other options, such as COUNT, aren't supported.
        _ => return Err("ERR syntax error"),
    };
    // STREAMS is followed by all the keys, then an ID for each of them.
    if !streams.len().is_multiple_of(2) {
        return Err(
            "ERR Unbalanced 'xread' list of streams: for each stream key an ID or '$' must be \
             specified.",
        );
    }
    let (keys, ids) = streams.split_at(streams.len() / 2);
    Ok(XreadArgs { keys, ids, block })
}

/// XREAD without the waiting: the entries after the ID given for each stream, or a null
/// array if there are none.
fn read_now(db: &mut Store, keys: &[Vec<u8>], ids: &[Vec<u8>]) -> Value {
    match read_positions(db, keys, ids) {
        Ok(afters) => entries_after(db, keys, &afters).unwrap_or(Value::NullArray),
        Err(reply) => reply,
    }
}

/// XREAD BLOCK outside a transaction: replies with the entries after the ID given for
/// each stream as soon as there are any, waiting for up to `timeout` (or forever if
/// there's no timeout) before replying with a null array.
async fn blocking_read(
    db: &Db,
    keys: &[Vec<u8>],
    ids: &[Vec<u8>],
    timeout: Option<Duration>,
) -> Value {
    // Work out the IDs up front, so that `$` keeps meaning the last ID from before any wait.
    let (afters, stream_added) = {
        let mut db = db.lock().unwrap();
        match read_positions(&mut db, keys, ids) {
            Ok(afters) => (afters, db.stream_added.clone()),
            Err(reply) => return reply,
        }
    };
    let read = async {
        loop {
            // Listen for new entries before looking, so that one added in between isn't missed.
            let added = stream_added.notified();
            tokio::pin!(added);
            added.as_mut().enable();
            let reply = entries_after(&mut db.lock().unwrap(), keys, &afters);
            if let Some(reply) = reply {
                return reply;
            }
            added.await;
        }
    };
    match timeout {
        None => read.await,
        Some(timeout) => tokio::time::timeout(timeout, read)
            .await
            .unwrap_or(Value::NullArray),
    }
}

/// Where XREAD reads each stream from: the entries after the ID given for it, where `$`
/// stands for the stream's last ID.
fn read_positions(
    db: &mut Store,
    keys: &[Vec<u8>],
    ids: &[Vec<u8>],
) -> Result<Vec<StreamId>, Value> {
    let mut afters = Vec::new();
    for (key, id) in keys.iter().zip(ids) {
        let last = match live_value(db, key) {
            Some(Data::Stream(stream)) => stream.last().map_or(StreamId::MIN, |entry| entry.id),
            Some(_) => return Err(wrong_type()),
            None => StreamId::MIN,
        };
        afters.push(match id.as_slice() {
            b"$" => last,
            id => parse_stream_id(id, 0).ok_or_else(invalid_stream_id)?,
        });
    }
    Ok(afters)
}

/// XREAD's reply: the key and entries of each stream that has entries after the given
/// position, or `None` if no stream has any.
fn entries_after(db: &mut Store, keys: &[Vec<u8>], afters: &[StreamId]) -> Option<Value> {
    let mut streams = Vec::new();
    for (key, &after) in keys.iter().zip(afters) {
        let Some(Data::Stream(stream)) = live_value(db, key) else {
            continue;
        };
        let first = stream.partition_point(|entry| entry.id <= after);
        if first < stream.len() {
            let entries = stream[first..].iter().map(StreamEntry::to_value).collect();
            streams.push(Value::Array(vec![
                Value::BulkString(key.clone()),
                Value::Array(entries),
            ]));
        }
    }
    if streams.is_empty() {
        None
    } else {
        Some(Value::Array(streams))
    }
}

/// Adds 1 to the integer stored at `key`, where a missing key counts as 0, and replies
/// with the result.
fn increment(db: &mut Store, key: &[u8]) -> Value {
    let Data::String(value) = live_value_or_insert(db, key, Data::String(b"0".to_vec())) else {
        return wrong_type();
    };
    let Some(n) = parse_number::<i64>(value) else {
        return Value::Error("ERR value is not an integer or out of range".into());
    };
    let Some(n) = n.checked_add(1) else {
        return Value::Error("ERR increment or decrement would overflow".into());
    };
    *value = n.to_string().into_bytes();
    db.touch(key);
    Value::Integer(n)
}

/// The replication ID, which identifies the history of a master's dataset. Redis generates
/// a new one each time it starts, but a fixed one is all that's needed here.
const REPLICATION_ID: &str = "8371b4fb1155b71f4a04d3e1bc3e18c4a990aeeb";

/// INFO's reply: the sections named, or the default ones if none are. Replication is the
/// only section so far, and as in Redis, a section that doesn't exist is just left out.
fn info(db: &Store, sections: &[Vec<u8>]) -> Value {
    let replication = sections.is_empty()
        || sections.iter().any(|section| {
            matches!(
                section.to_ascii_lowercase().as_slice(),
                b"replication" | b"default" | b"all" | b"everything"
            )
        });
    if !replication {
        return Value::BulkString(Vec::new());
    }
    let role = if db.is_replica { "slave" } else { "master" };
    // The offset counts the bytes of commands streamed to replicas, and none are yet.
    let text = format!(
        "role:{role}\r\n\
         master_replid:{REPLICATION_ID}\r\n\
         master_repl_offset:0\r\n"
    );
    Value::BulkString(text.into_bytes())
}

/// Clients send each command as an array of bulk strings, e.g. ["ECHO", "hey"].
fn command_args(request: Value) -> Option<Vec<Vec<u8>>> {
    let Value::Array(items) = request else {
        return None;
    };
    items
        .into_iter()
        .map(|item| match item {
            Value::BulkString(arg) => Some(arg),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Config, String> {
        parse_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn port_defaults_to_6379() {
        assert_eq!(parse(&[]).unwrap().port, 6379);
    }

    #[test]
    fn reads_the_port() {
        assert_eq!(parse(&["--port", "6380"]).unwrap().port, 6380);
    }

    #[test]
    fn replicates_nothing_by_default() {
        assert_eq!(parse(&[]).unwrap().replicaof, None);
    }

    #[test]
    fn reads_the_master_to_replicate() {
        for args in [
            ["--port", "6380", "--replicaof", "localhost 6379"],
            ["--replicaof", "localhost 6379", "--port", "6380"],
        ] {
            let config = parse(&args).unwrap();
            assert_eq!(config.port, 6380);
            assert_eq!(config.replicaof, Some(("localhost".to_string(), 6379)));
        }
    }

    #[test]
    fn rejects_bad_arguments() {
        let error = |args: &[&str]| parse(args).err().unwrap();
        assert_eq!(error(&["--port"]), "--port needs a port number");
        assert_eq!(error(&["--port", "abc"]), "invalid port: abc");
        assert_eq!(error(&["--port", "70000"]), "invalid port: 70000");
        assert_eq!(error(&["--bogus"]), "unknown argument: --bogus");
        assert_eq!(error(&["--replicaof"]), "--replicaof needs a host and port");
        for master in [
            "",
            "localhost",
            "localhost abc",
            "localhost 70000",
            "a b 6379",
        ] {
            assert_eq!(
                error(&["--replicaof", master]),
                format!("invalid master address: {master}")
            );
        }
    }
}
