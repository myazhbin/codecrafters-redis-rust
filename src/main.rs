#![allow(unused_imports)]

mod resp;

use anyhow::Result;
use bytes::{Buf, BytesMut};
use resp::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

type Db = Arc<Mutex<HashMap<Vec<u8>, Entry>>>;

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
            stream
                .write_all(&handle_command(request, &db).encode())
                .await?;
        }

        if stream.read_buf(&mut buf).await? == 0 {
            return Ok(());
        }
    }
}

fn handle_command(request: Value, db: &Db) -> Value {
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
                    db.lock().unwrap().insert(key.clone(), entry);
                    Value::SimpleString("OK".into())
                }
                Err(msg) => Value::Error(msg.into()),
            },
            _ => wrong_arity("set"),
        },
        b"GET" => match args {
            [key] => match live_value(&mut db.lock().unwrap(), key) {
                Some(Data::String(value)) => Value::BulkString(value.clone()),
                Some(Data::List(_)) => wrong_type(),
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
                let (Some(start), Some(stop)) = (parse_integer(start), parse_integer(stop)) else {
                    return Value::Error("ERR value is not an integer or out of range".into());
                };
                match live_value(&mut db.lock().unwrap(), key) {
                    Some(Data::List(list)) => Value::Array(list_range(list, start, stop)),
                    Some(Data::String(_)) => wrong_type(),
                    None => Value::Array(vec![]),
                }
            }
            _ => wrong_arity("lrange"),
        },
        b"LLEN" => match args {
            [key] => match live_value(&mut db.lock().unwrap(), key) {
                Some(Data::List(list)) => Value::Integer(list.len() as i64),
                Some(Data::String(_)) => wrong_type(),
                None => Value::Integer(0),
            },
            _ => wrong_arity("llen"),
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
    let Some(amount) = parse_integer::<u64>(amount) else {
        return Err("ERR value is not an integer or out of range");
    };
    // checked_add so that an absurdly long expiry is an error rather than a panic.
    match Instant::now().checked_add(to_duration(amount)) {
        Some(expires_at) if amount > 0 => Ok(Some(expires_at)),
        _ => Err("ERR invalid expire time in 'set' command"),
    }
}

/// Parses an integer argument, such as an expiry time or a list index.
fn parse_integer<T: std::str::FromStr>(arg: &[u8]) -> Option<T> {
    std::str::from_utf8(arg).ok()?.parse().ok()
}

/// Looks up the value at `key`. Expired keys are only removed once something tries to
/// read them, so an expired key is removed here and treated as missing.
fn live_value<'a>(db: &'a mut HashMap<Vec<u8>, Entry>, key: &[u8]) -> Option<&'a mut Data> {
    if db.get(key).is_some_and(Entry::is_expired) {
        db.remove(key);
    }
    db.get_mut(key).map(|entry| &mut entry.value)
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
    // An expired key counts as missing, so pushing to it starts a new list.
    if db.get(key).is_some_and(Entry::is_expired) {
        db.remove(key);
    }
    let entry = db.entry(key.to_vec()).or_insert_with(|| Entry {
        value: Data::List(VecDeque::new()),
        expires_at: None,
    });
    match &mut entry.value {
        Data::List(list) => {
            for element in elements {
                push(list, element.clone());
            }
            Value::Integer(list.len() as i64)
        }
        Data::String(_) => wrong_type(),
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
