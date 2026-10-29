#![allow(unused_imports)]

mod resp;

use anyhow::Result;
use bytes::{Buf, BytesMut};
use resp::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("127.0.0.1:6379").await.unwrap();

    loop {
        let stream = listener.accept().await;

        match stream {
            Ok((stream, _)) => {
                println!("accepted new connection");

                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream).await {
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

async fn handle_connection(mut stream: TcpStream) -> Result<()> {
    let mut buf = BytesMut::with_capacity(512);
    loop {
        // A read can end partway through a command or hold several pipelined ones,
        // so answer every complete command buffered so far, then read more.
        while let Some((request, len)) = Value::parse(&buf)? {
            buf.advance(len);
            stream.write_all(&handle_command(request).encode()).await?;
        }

        if stream.read_buf(&mut buf).await? == 0 {
            return Ok(());
        }
    }
}

fn handle_command(request: Value) -> Value {
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
            _ => Value::Error("ERR wrong number of arguments for 'echo' command".into()),
        },
        _ => Value::Error(format!("ERR unknown command '{}'", String::from_utf8_lossy(name))),
    }
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
