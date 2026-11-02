#![allow(unused_imports)]

mod resp;

use anyhow::Result;
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
    /// Notified whenever an entry is added to a stream, so that blocked XREADs look again.
    stream_added: Arc<Notify>,
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

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("127.0.0.1:6379").await.unwrap();
    let db = Db::default();

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

async fn handle_connection(mut stream: TcpStream, db: Db) -> Result<()> {
    let mut buf = BytesMut::with_capacity(512);
    loop {
        // A read can end partway through a command or hold several pipelined ones,
        // so answer every complete command buffered so far, then read more.
        while let Some((request, len)) = Value::parse(&buf)? {
            buf.advance(len);
            let command = handle_command(request, &db);
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

async fn handle_command(request: Value, db: &Db) -> Value {
    let Some(args) = command_args(request) else {
        return Value::Error("ERR expected an array of bulk strings".into());
    };
    let Some((name, args)) = args.split_first() else {
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
                    db.lock().unwrap().entries.insert(key.clone(), entry);
                    Value::SimpleString("OK".into())
                }
                Err(msg) => Value::Error(msg.into()),
            },
            _ => wrong_arity("set"),
        },
        b"GET" => match args {
            [key] => match live_value(&mut db.lock().unwrap(), key) {
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
                match live_value(&mut db.lock().unwrap(), key) {
                    Some(Data::List(list)) => Value::Array(list_range(list, start, stop)),
                    Some(_) => wrong_type(),
                    None => Value::Array(vec![]),
                }
            }
            _ => wrong_arity("lrange"),
        },
        b"LLEN" => match args {
            [key] => match live_value(&mut db.lock().unwrap(), key) {
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
            [key, timeout] => match parse_timeout(timeout) {
                Ok(timeout) => blocking_pop(db, key, timeout).await,
                Err(msg) => Value::Error(msg.into()),
            },
            _ => wrong_arity("blpop"),
        },
        b"TYPE" => match args {
            [key] => {
                let kind = match live_value(&mut db.lock().unwrap(), key) {
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
                match live_value(&mut db.lock().unwrap(), key) {
                    Some(Data::Stream(stream)) => Value::Array(stream_range(stream, start, end)),
                    Some(_) => wrong_type(),
                    None => Value::Array(vec![]),
                }
            }
            _ => wrong_arity("xrange"),
        },
        b"XREAD" => match args {
            [_, _, _, ..] => match parse_xread_args(args) {
                Ok(args) => read_streams(db, args).await,
                Err(msg) => Value::Error(msg.into()),
            },
            _ => wrong_arity("xread"),
        },
        b"INCR" => match args {
            [key] => increment(db, key),
            _ => wrong_arity("incr"),
        },
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

/// Looks up the value at `key`. Expired keys are only removed once something tries to
/// read them, so an expired key is removed here and treated as missing.
fn live_value<'a>(db: &'a mut Store, key: &[u8]) -> Option<&'a mut Data> {
    if db.entries.get(key).is_some_and(Entry::is_expired) {
        db.entries.remove(key);
    }
    db.entries.get_mut(key).map(|entry| &mut entry.value)
}

/// Like `live_value`, but first stores `new` at `key` if the key is missing or expired.
fn live_value_or_insert<'a>(db: &'a mut Store, key: &[u8], new: Data) -> &'a mut Data {
    if db.entries.get(key).is_some_and(Entry::is_expired) {
        db.entries.remove(key);
    }
    let entry = db.entries.entry(key.to_vec()).or_insert(Entry {
        value: new,
        expires_at: None,
    });
    &mut entry.value
}

/// Pushes `elements` onto the list at `key` one at a time (so LPUSH a b c gives c b a),
/// creating the list if needed. Replies with the list's new length.
fn push_to_list(
    db: &Db,
    key: &[u8],
    elements: &[Vec<u8>],
    push: fn(&mut VecDeque<Vec<u8>>, Vec<u8>),
) -> Value {
    let mut db = db.lock().unwrap();
    let Data::List(list) = live_value_or_insert(&mut db, key, Data::List(VecDeque::new())) else {
        return wrong_type();
    };
    for element in elements {
        push(list, element.clone());
    }
    // Like Redis, reply with the length from before blocked clients take their elements.
    let len = list.len();
    serve_blocked(&mut db, key);
    Value::Integer(len as i64)
}

/// Removes elements from the front of the list at `key`: just the first one as a bulk
/// string, or up to `count` of them as an array.
fn pop_from_list(db: &Db, key: &[u8], count: Option<usize>) -> Value {
    let mut db = db.lock().unwrap();
    let list = match live_value(&mut db, key) {
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
    reply
}

/// Hands elements from the front of the list at `key` to the clients blocked on it,
/// longest-waiting first.
fn serve_blocked(db: &mut Store, key: &[u8]) {
    let Some(waiters) = db.blocked.get_mut(key) else {
        return;
    };
    // Forget clients that have stopped waiting: they timed out or hung up.
    waiters.retain(|waiter| !waiter.is_closed());
    if let Some(Data::List(list)) = db.entries.get_mut(key).map(|entry| &mut entry.value) {
        while !list.is_empty() {
            let Some(waiter) = waiters.pop_front() else {
                break;
            };
            let element = list.pop_front().unwrap();
            // A client can still stop waiting after the retain above, in which case its
            // element goes back to the list for the next one.
            if let Err(element) = waiter.send(element) {
                list.push_front(element);
            }
        }
        if list.is_empty() {
            db.entries.remove(key);
        }
    }
    if waiters.is_empty() {
        db.blocked.remove(key);
    }
}

/// Pops the first element of the list at `key`, waiting for one to be pushed if the
/// list is empty: for up to `timeout`, or forever if there is none.
async fn blocking_pop(db: &Db, key: &[u8], timeout: Option<Duration>) -> Value {
    let mut receiver = {
        let mut db = db.lock().unwrap();
        if !matches!(live_value(&mut db, key), None | Some(Data::List(_))) {
            return wrong_type();
        }
        let (sender, receiver) = oneshot::channel();
        db.blocked
            .entry(key.to_vec())
            .or_default()
            .push_back(sender);
        // Clients only wait while the list is empty, so if it has elements, this hands
        // the first one straight to us.
        serve_blocked(&mut db, key);
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
    match element {
        Some(element) => Value::Array(vec![
            Value::BulkString(key.to_vec()),
            Value::BulkString(element),
        ]),
        None => Value::NullArray,
    }
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
fn add_to_stream(db: &Db, key: &[u8], id: &[u8], fields: &[Vec<u8>]) -> Value {
    let Some((ms, seq)) = parse_xadd_id(id) else {
        return invalid_stream_id();
    };
    // Rejected before the stream is created, so that this can't leave an empty one behind.
    if (ms, seq) == (Some(0), Some(0)) {
        return Value::Error("ERR The ID specified in XADD must be greater than 0-0".into());
    }
    let mut db = db.lock().unwrap();
    let Data::Stream(stream) = live_value_or_insert(&mut db, key, Data::Stream(Vec::new())) else {
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

/// Replies with the entries after the ID given for each stream (`$` standing for the
/// stream's last ID), leaving out streams that have none. If none of them has any, it
/// waits for new entries while BLOCK allows, and otherwise replies with a null array.
async fn read_streams(db: &Db, args: XreadArgs<'_>) -> Value {
    let XreadArgs { keys, ids, block } = args;
    // Work out the IDs up front, so that `$` keeps meaning the last ID from before any wait.
    let (afters, stream_added) = {
        let mut db = db.lock().unwrap();
        let mut afters = Vec::new();
        for (key, id) in keys.iter().zip(ids) {
            let last = match live_value(&mut db, key) {
                Some(Data::Stream(stream)) => stream.last().map_or(StreamId::MIN, |entry| entry.id),
                Some(_) => return wrong_type(),
                None => StreamId::MIN,
            };
            afters.push(match id.as_slice() {
                b"$" => last,
                id => match parse_stream_id(id, 0) {
                    Some(id) => id,
                    None => return invalid_stream_id(),
                },
            });
        }
        (afters, db.stream_added.clone())
    };
    let read = async {
        loop {
            // Listen for new entries before looking, so that one added in between isn't missed.
            let added = stream_added.notified();
            tokio::pin!(added);
            added.as_mut().enable();
            let streams = entries_after(&mut db.lock().unwrap(), keys, &afters);
            if !streams.is_empty() {
                return Value::Array(streams);
            }
            if block.is_none() {
                return Value::NullArray;
            }
            added.await;
        }
    };
    match block {
        Some(Some(timeout)) => tokio::time::timeout(timeout, read)
            .await
            .unwrap_or(Value::NullArray),
        _ => read.await,
    }
}

/// The key and entries of each stream that has entries after the ID given for it, as XREAD
/// replies with them.
fn entries_after(db: &mut Store, keys: &[Vec<u8>], afters: &[StreamId]) -> Vec<Value> {
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
    streams
}

/// Adds 1 to the integer stored at `key`, where a missing key counts as 0, and replies
/// with the result.
fn increment(db: &Db, key: &[u8]) -> Value {
    let mut db = db.lock().unwrap();
    let Data::String(value) = live_value_or_insert(&mut db, key, Data::String(b"0".to_vec()))
    else {
        return wrong_type();
    };
    let Some(n) = parse_number::<i64>(value) else {
        return Value::Error("ERR value is not an integer or out of range".into());
    };
    let Some(n) = n.checked_add(1) else {
        return Value::Error("ERR increment or decrement would overflow".into());
    };
    *value = n.to_string().into_bytes();
    Value::Integer(n)
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
