#![allow(unused_imports)]

mod acl;
mod aof;
mod bitmap;
mod config;
mod geo;
mod glob;
mod rdb;
mod resp;
mod zset;

use anyhow::{Context, Result, bail};
use bytes::{Buf, BytesMut};
use config::Config;
use resp::Value;
use zset::SortedSet;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, mpsc, oneshot};

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
    /// The replicas connected to this server, by an ID that tells them apart.
    replicas: HashMap<u64, Replica>,
    next_replica_id: u64,
    /// Notified when a replica connects or acknowledges an offset, so that blocked WAITs
    /// count again.
    replicas_changed: Arc<Notify>,
    /// How far through the replication stream this server is, in bytes: how much of it a
    /// master has sent, or how much of it a replica has processed.
    repl_offset: u64,
    /// Where the replication stream ended after the last write, which is as far as a
    /// replica has to get to have all the writes. (The stream also holds GETACKs.)
    write_offset: u64,
    /// The settings the server started with, for CONFIG GET.
    config: Config,
    /// With `--appendonly yes`, the append-only file that writes are logged to.
    aof: Option<aof::Aof>,
    /// The clients subscribed to each Pub/Sub channel, by client ID, with where to send
    /// them the messages published to it.
    channels: HashMap<Vec<u8>, HashMap<u64, mpsc::UnboundedSender<Vec<u8>>>>,
    /// The users clients can log in as.
    users: acl::Users,
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

    /// Starts sending the replication stream to a new replica, which counts as having all
    /// of it up to now. Returns the replica's ID.
    fn add_replica(&mut self, stream: mpsc::UnboundedSender<Vec<u8>>) -> u64 {
        let id = self.next_replica_id;
        self.next_replica_id += 1;
        let acked_offset = self.repl_offset;
        self.replicas.insert(
            id,
            Replica {
                stream,
                acked_offset,
            },
        );
        self.replicas_changed.notify_waiters();
        id
    }

    /// Records a replica's acknowledgement that it has got as far as `offset`.
    fn record_ack(&mut self, id: u64, offset: u64) {
        if let Some(replica) = self.replicas.get_mut(&id) {
            replica.acked_offset = offset;
            self.replicas_changed.notify_waiters();
        }
    }

    /// How many connected replicas have acknowledged getting as far as `offset`.
    fn replicas_at(&mut self, offset: u64) -> usize {
        // Forget replicas that have hung up.
        self.replicas
            .retain(|_, replica| !replica.stream.is_closed());
        self.replicas
            .values()
            .filter(|replica| replica.acked_offset >= offset)
            .count()
    }

    /// Adds `command` to the replication stream. With no replicas there's no stream, as in
    /// Redis, so the offset stays where it is.
    fn send_to_replicas<T: AsRef<[u8]>>(&mut self, command: &[T]) {
        if self.replicas.is_empty() {
            return;
        }
        let encoded = Value::command(command).encode();
        self.repl_offset += encoded.len() as u64;
        // Forget replicas that have hung up.
        self.replicas
            .retain(|_, replica| replica.stream.send(encoded.clone()).is_ok());
    }

    /// Sends a write to the replicas, to keep them in step, and logs it to the AOF.
    fn propagate<T: AsRef<[u8]>>(&mut self, command: &[T]) {
        self.send_to_replicas(command);
        self.write_offset = self.repl_offset;
        if let Some(aof) = &mut self.aof {
            aof.append(&aof::logged_form(command, unix_time_ms()));
        }
    }

    /// Propagates the pop of an element from the list at `key`. Replicas get it as LPOP
    /// however the client came to pop it, so that a BLPOP never leaves them waiting.
    fn propagate_pop(&mut self, key: &[u8]) {
        self.propagate(&[b"LPOP".as_slice(), key]);
    }
}

/// A replica connected to this server.
struct Replica {
    /// Takes the replication stream to the task serving the replica's connection.
    stream: mpsc::UnboundedSender<Vec<u8>>,
    /// How far through the stream the replica has acknowledged getting.
    acked_offset: u64,
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
    SortedSet(SortedSet),
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
    let cwd = std::env::current_dir().map_or_else(|_| String::new(), |dir| dir.display().to_string());
    let config = config::parse_args(std::env::args().skip(1), cwd).unwrap_or_else(|msg| {
        eprintln!("{msg}");
        std::process::exit(1);
    });
    let db = Db::new(Mutex::new(Store {
        is_replica: config.replicaof.is_some(),
        config: config.clone(),
        ..Store::default()
    }));
    // The data and the AOF's files are ready before the server starts listening, so that a
    // client can't get in first.
    // With the AOF on, the data comes from it rather than from the RDB file, as in Redis.
    if config.appendonly {
        if let Err(e) = start_aof(&mut db.lock().unwrap(), &config) {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    } else {
        load_rdb_file(&mut db.lock().unwrap(), &config);
    }
    let listener = TcpListener::bind(("127.0.0.1", config.port)).await.unwrap();

    if let Some((host, master_port)) = config.replicaof {
        let db = db.clone();
        tokio::spawn(async move {
            if let Err(e) = connect_to_master(&host, master_port, config.port, &db).await {
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

/// Connects a replica to its master and goes through the handshake, waiting for the reply
/// to each command before sending the next, then follows the master until it hangs up.
async fn connect_to_master(host: &str, port: u16, listening_port: u16, db: &Db) -> Result<()> {
    let mut master = TcpStream::connect((host, port)).await?;
    let mut buf = BytesMut::with_capacity(512);
    let listening_port = listening_port.to_string();
    let first_commands = [
        vec!["PING"],
        vec!["REPLCONF", "listening-port", &listening_port],
        vec!["REPLCONF", "capa", "psync2"],
    ];
    for command in first_commands {
        ask_master(&mut master, &mut buf, &command).await?;
    }
    // The replica doesn't know the master's ID yet, and has none of its data.
    let reply = ask_master(&mut master, &mut buf, &["PSYNC", "?", "-1"]).await?;
    let Some(offset) = parse_fullresync(&reply) else {
        bail!("the reply to PSYNC was: {reply:?}");
    };
    // The master's data comes as an RDB file. Masters here only ever send empty ones, but
    // a real Redis master sends what it has.
    let rdb = read_from_master(&mut master, &mut buf, resp::parse_file).await?;
    {
        let mut db = db.lock().unwrap();
        load_rdb(&mut db, &rdb);
        db.repl_offset = offset;
    }
    follow_master(&mut master, &mut buf, db).await
}

/// Gets the AOF going at startup: sets up its files, replays them to get the data back,
/// and then opens the incremental file for writes to be appended to.
fn start_aof(db: &mut Store, config: &Config) -> Result<()> {
    let files = aof::set_up(config)?;
    for (path, kind) in aof::replay_order(config, &files) {
        let data = std::fs::read(&path)
            .with_context(|| format!("couldn't read the append-only file {}", path.display()))?;
        // A base file can be an RDB file rather than a list of commands.
        if kind == aof::FileKind::Base && data.starts_with(b"REDIS") {
            load_rdb(db, &data);
        } else {
            replay_commands(db, &path, &data)?;
        }
    }
    // Nobody can be waiting yet for the lists the replay pushed to.
    db.pushed.clear();
    db.aof = Some(aof::Aof::open(config, &files)?);
    Ok(())
}

/// Runs the commands in an AOF file, one after another, as a client sending them would.
/// If the file ends partway through a command, as a crash while writing can leave it,
/// that part is cut off the file, as Redis does.
fn replay_commands(db: &mut Store, path: &std::path::Path, data: &[u8]) -> Result<()> {
    let mut pos = 0;
    while pos < data.len() {
        let parsed = Value::parse(&data[pos..]).with_context(|| {
            format!("the append-only file {} isn't valid RESP at byte {pos}", path.display())
        })?;
        let Some((request, len)) = parsed else {
            eprintln!(
                "the append-only file {} ends partway through a command; cutting off its last {} bytes",
                path.display(),
                data.len() - pos
            );
            let file = std::fs::OpenOptions::new().write(true).open(path)?;
            file.set_len(pos as u64)?;
            break;
        };
        pos += len;
        let Some(command) = command_args(request) else {
            bail!("the append-only file {} holds something that isn't a command", path.display());
        };
        // Redis wraps transactions in MULTI and EXEC; their commands are all that matter.
        let name = command.first().map(|name| name.to_ascii_uppercase());
        if matches!(name.as_deref(), Some(b"MULTI" | b"EXEC")) {
            continue;
        }
        if let Value::Error(msg) = run_command(db, &command) {
            eprintln!("replaying the append-only file: {msg}");
        }
    }
    Ok(())
}

/// Loads the keys from the RDB file that `--dir` and `--dbfilename` name, if it exists.
fn load_rdb_file(db: &mut Store, config: &Config) {
    let path = std::path::Path::new(&config.dir).join(&config.dbfilename);
    match std::fs::read(&path) {
        Ok(data) => load_rdb(db, &data),
        // With no file yet, the database starts out empty.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("couldn't read {}: {e}", path.display()),
    }
}

/// Adds the keys in an RDB file to the database, apart from any that have already
/// expired. A file that can't be read adds nothing.
fn load_rdb(db: &mut Store, data: &[u8]) {
    let keys = match rdb::parse(data) {
        Ok(keys) => keys,
        Err(e) => {
            eprintln!("couldn't load the RDB file: {e:#}");
            return;
        }
    };
    let now_ms = unix_time_ms();
    for key in keys {
        // With no SELECT here, only database 0 can be used.
        if key.db != 0 {
            continue;
        }
        let expires_at = match key.expires_at_ms {
            None => None,
            Some(at) if at <= now_ms => continue,
            // An expiry too far off to represent might as well be none.
            Some(at) => Instant::now().checked_add(Duration::from_millis(at - now_ms)),
        };
        let entry = Entry {
            value: Data::String(key.value),
            expires_at,
        };
        db.entries.insert(key.key, entry);
    }
}

/// The current Unix time in milliseconds.
fn unix_time_ms() -> u64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH);
    now.unwrap_or_default().as_millis() as u64
}

/// Sends a command to the master and waits for its reply, which mustn't be an error.
async fn ask_master(master: &mut TcpStream, buf: &mut BytesMut, command: &[&str]) -> Result<Value> {
    master.write_all(&Value::command(command).encode()).await?;
    let reply = read_from_master(master, buf, Value::parse).await?;
    if let Value::Error(msg) = &reply {
        bail!("the reply to {} was: {msg}", command.join(" "));
    }
    Ok(reply)
}

/// Reads what the master sends next during the handshake, however many reads it takes for
/// all of it to arrive, given how to parse it.
async fn read_from_master<T>(
    master: &mut TcpStream,
    buf: &mut BytesMut,
    parse: fn(&[u8]) -> Result<Option<(T, usize)>>,
) -> Result<T> {
    loop {
        if let Some((parsed, len)) = parse(buf)? {
            buf.advance(len);
            return Ok(parsed);
        }
        if master.read_buf(buf).await? == 0 {
            bail!("the master hung up during the handshake");
        }
    }
}

/// The replication offset in a master's reply to PSYNC: `FULLRESYNC <id> <offset>`.
fn parse_fullresync(reply: &Value) -> Option<u64> {
    let Value::SimpleString(reply) = reply else {
        return None;
    };
    match reply.split(' ').collect::<Vec<_>>()[..] {
        ["FULLRESYNC", _, offset] => offset.parse().ok(),
        _ => None,
    }
}

/// Processes the replication stream from the master until it hangs up. Its commands run
/// like a client's, but the master gets no replies, except when it asks for an ACK.
async fn follow_master(master: &mut TcpStream, buf: &mut BytesMut, db: &Db) -> Result<()> {
    let mut session = Session::default();
    loop {
        // A read can end partway through a command or hold several of them.
        while let Some((request, len)) = Value::parse(buf)? {
            buf.advance(len);
            if replconf_argument(&request, b"GETACK").is_some() {
                let offset = db.lock().unwrap().repl_offset.to_string();
                let ack = Value::command(&["REPLCONF", "ACK", &offset]);
                master.write_all(&ack.encode()).await?;
            } else {
                handle_request(request, db, &mut session).await;
            }
            // Added only now, because an ACK doesn't count the GETACK that asked for it.
            db.lock().unwrap().repl_offset += len as u64;
        }
        if master.read_buf(buf).await? == 0 {
            return Ok(());
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
    /// Set by PSYNC, which makes the client a replica: its ID, and where its replication
    /// stream arrives.
    replica: Option<(u64, mpsc::UnboundedReceiver<Vec<u8>>)>,
    /// The Pub/Sub channels the client is subscribed to, in the order it subscribed.
    subscriptions: Vec<Vec<u8>>,
    /// Where messages published to those channels arrive, once the client has subscribed.
    inbox: Option<Inbox>,
    /// The user the client is logged in as, if any.
    user: Option<Vec<u8>>,
}

/// Where the messages published to a client's channels arrive, ready to send it.
struct Inbox {
    /// Tells the client apart from other subscribers to a channel.
    id: u64,
    sender: mpsc::UnboundedSender<Vec<u8>>,
    receiver: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl Inbox {
    fn new() -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let (sender, receiver) = mpsc::unbounded_channel();
        Inbox {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            sender,
            receiver,
        }
    }
}

async fn handle_connection(stream: TcpStream, db: Db) -> Result<()> {
    let user = db.lock().unwrap().users.new_connection_user();
    let mut session = Session {
        user,
        ..Session::default()
    };
    let result = serve_client(stream, &db, &mut session).await;
    // A client that hangs up is no longer subscribed to anything.
    let mut db = db.lock().unwrap();
    for channel in std::mem::take(&mut session.subscriptions) {
        unsubscribe(&mut db, &channel, &session);
    }
    result
}

async fn serve_client(mut stream: TcpStream, db: &Db, session: &mut Session) -> Result<()> {
    let mut buf = BytesMut::with_capacity(512);
    loop {
        // A read can end partway through a command or hold several pipelined ones,
        // so answer every complete command buffered so far, then read more.
        while let Some((request, len)) = Value::parse(&buf)? {
            buf.advance(len);
            // Messages published before this command reach the client before its reply.
            if let Some(inbox) = session.inbox.as_mut() {
                while let Ok(message) = inbox.receiver.try_recv() {
                    stream.write_all(&message).await?;
                }
            }
            if let Some(replies) = pubsub_command(&request, db, session) {
                for reply in replies {
                    stream.write_all(&reply.encode()).await?;
                }
                continue;
            }
            let reply = {
                let command = handle_request(request, db, session);
                tokio::pin!(command);
                // Some commands can wait indefinitely, so keep reading meanwhile. If the
                // client hangs up, the wait is dropped rather than later taking an element
                // nobody receives. `biased` checks the command first, so a ready reply is
                // never skipped.
                loop {
                    tokio::select! {
                        biased;
                        reply = &mut command => break reply,
                        read = stream.read_buf(&mut buf) => {
                            if read? == 0 {
                                return Ok(());
                            }
                        }
                    }
                }
            };
            stream.write_all(&reply.encode()).await?;
            // PSYNC has made this a replication connection, for good.
            if let Some((id, replication_stream)) = session.replica.take() {
                return serve_replica(stream, db, id, replication_stream).await;
            }
        }

        // Wait for more from the client, meanwhile sending it any messages published to
        // the channels it's subscribed to.
        let message = async {
            match session.inbox.as_mut() {
                Some(inbox) => inbox.receiver.recv().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            read = stream.read_buf(&mut buf) => {
                if read? == 0 {
                    return Ok(());
                }
            }
            Some(message) = message => stream.write_all(&message).await?,
        }
    }
}

/// Handles the Pub/Sub commands that need the client's session and can reply more than
/// once (SUBSCRIBE and UNSUBSCRIBE reply once for each channel), and refuses the commands
/// that aren't allowed in subscribed mode. Returns `None` for any other command, which
/// `handle_request` deals with.
fn pubsub_command(request: &Value, db: &Db, session: &mut Session) -> Option<Vec<Value>> {
    let Value::Array(items) = request else {
        return None;
    };
    let args: Vec<&[u8]> = items
        .iter()
        .map(|item| match item {
            Value::BulkString(arg) => Some(arg.as_slice()),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let (name, channels) = args.split_first()?;
    let name = name.to_ascii_uppercase();
    // A client subscribed to a channel is in subscribed mode, where it's only meant to be
    // listening, so most commands are refused.
    const ALLOWED_WHEN_SUBSCRIBED: &[&[u8]] = &[
        b"SUBSCRIBE", b"UNSUBSCRIBE", b"PSUBSCRIBE", b"PUNSUBSCRIBE", b"SSUBSCRIBE",
        b"SUNSUBSCRIBE", b"PING", b"QUIT", b"RESET",
    ];
    if !session.subscriptions.is_empty() && !ALLOWED_WHEN_SUBSCRIBED.contains(&name.as_slice()) {
        let name = String::from_utf8_lossy(&name).to_lowercase();
        return Some(vec![Value::Error(format!(
            "ERR Can't execute '{name}': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT \
             / RESET are allowed in this context"
        ))]);
    }
    // In subscribed mode, PING replies the way a message arrives, as an array.
    if name == b"PING" && !session.subscriptions.is_empty() {
        let reply = match channels {
            [] => Value::Array(vec![Value::BulkString(b"pong".to_vec()), Value::BulkString(vec![])]),
            [message] => Value::Array(vec![
                Value::BulkString(b"pong".to_vec()),
                Value::BulkString(message.to_vec()),
            ]),
            _ => wrong_arity("ping"),
        };
        return Some(vec![reply]);
    }
    let subscribing = match name.as_slice() {
        b"SUBSCRIBE" => true,
        b"UNSUBSCRIBE" => false,
        _ => return None,
    };
    if session.transaction.is_some() {
        return Some(vec![Value::Error(
            "ERR Command not allowed inside a transaction".into(),
        )]);
    }
    let mut db = db.lock().unwrap();
    let mut replies = Vec::new();
    if subscribing {
        if channels.is_empty() {
            return Some(vec![wrong_arity("subscribe")]);
        }
        let inbox = session.inbox.get_or_insert_with(Inbox::new);
        for &channel in channels {
            // Subscribing to a channel again changes nothing.
            if !session.subscriptions.iter().any(|c| c == channel) {
                session.subscriptions.push(channel.to_vec());
                let subscribers = db.channels.entry(channel.to_vec()).or_default();
                subscribers.insert(inbox.id, inbox.sender.clone());
            }
            replies.push(subscription_reply("subscribe", Some(channel), session.subscriptions.len()));
        }
        return Some(replies);
    }
    // UNSUBSCRIBE with no channels means every channel the client is subscribed to.
    let channels: Vec<Vec<u8>> = if channels.is_empty() {
        session.subscriptions.clone()
    } else {
        channels.iter().map(|channel| channel.to_vec()).collect()
    };
    if channels.is_empty() {
        return Some(vec![subscription_reply("unsubscribe", None, 0)]);
    }
    for channel in channels {
        // Unsubscribing from a channel the client isn't subscribed to changes nothing.
        if let Some(i) = session.subscriptions.iter().position(|c| *c == channel) {
            session.subscriptions.remove(i);
            unsubscribe(&mut db, &channel, session);
        }
        replies.push(subscription_reply("unsubscribe", Some(&channel), session.subscriptions.len()));
    }
    Some(replies)
}

/// Takes the client out of the channel's subscribers.
fn unsubscribe(db: &mut Store, channel: &[u8], session: &Session) {
    let Some(inbox) = &session.inbox else {
        return;
    };
    if let Some(subscribers) = db.channels.get_mut(channel) {
        subscribers.remove(&inbox.id);
        if subscribers.is_empty() {
            db.channels.remove(channel);
        }
    }
}

/// PUBLISH: sends `message` to every client subscribed to `channel`, and replies with how
/// many there were.
fn publish(db: &mut Store, channel: &[u8], message: &[u8]) -> Value {
    let Some(subscribers) = db.channels.get_mut(channel) else {
        return Value::Integer(0);
    };
    let delivery = Value::Array(vec![
        Value::BulkString(b"message".to_vec()),
        Value::BulkString(channel.to_vec()),
        Value::BulkString(message.to_vec()),
    ])
    .encode();
    // A client whose connection has just closed is forgotten rather than counted.
    subscribers.retain(|_, inbox| inbox.send(delivery.clone()).is_ok());
    let count = subscribers.len();
    if subscribers.is_empty() {
        db.channels.remove(channel);
    }
    Value::Integer(count as i64)
}

/// The reply to subscribing to a channel or unsubscribing from one: what happened, the
/// channel, and how many channels the client is subscribed to now.
fn subscription_reply(kind: &str, channel: Option<&[u8]>, count: usize) -> Value {
    Value::Array(vec![
        Value::BulkString(kind.as_bytes().to_vec()),
        channel.map_or(Value::NullBulkString, |channel| Value::BulkString(channel.to_vec())),
        Value::Integer(count as i64),
    ])
}

/// An empty RDB file: a header, some metadata, an end-of-file marker and a checksum.
const EMPTY_RDB: &[u8] = b"REDIS0011\
    \xfa\x09redis-ver\x057.2.0\
    \xfa\x0aredis-bits\xc0\x40\
    \xfa\x05ctime\xc2\x6d\x08\xbc\x65\
    \xfa\x08used-mem\xc2\xb0\xc4\x10\x00\
    \xfa\x08aof-base\xc0\x00\
    \xff\xf0\x6e\x3b\xfe\xc0\xff\x5a\xa2";

/// Serves a connection that PSYNC has made a replication connection: sends the replica an
/// empty RDB file in place of the data so far, then the replication stream as it comes,
/// while listening for the replica's ACKs. Goes on until the replica hangs up.
async fn serve_replica(
    mut stream: TcpStream,
    db: &Db,
    id: u64,
    mut replication_stream: mpsc::UnboundedReceiver<Vec<u8>>,
) -> Result<()> {
    // Like a bulk string, but with no \r\n after the contents.
    let header = format!("${}\r\n", EMPTY_RDB.len());
    stream.write_all(header.as_bytes()).await?;
    stream.write_all(EMPTY_RDB).await?;
    let mut buf = BytesMut::with_capacity(512);
    loop {
        tokio::select! {
            Some(data) = replication_stream.recv() => stream.write_all(&data).await?,
            read = stream.read_buf(&mut buf) => {
                if read? == 0 {
                    return Ok(());
                }
                while let Some((request, len)) = Value::parse(&buf)? {
                    buf.advance(len);
                    let offset = replconf_argument(&request, b"ACK").and_then(parse_number);
                    if let Some(offset) = offset {
                        db.lock().unwrap().record_ack(id, offset);
                    }
                }
            }
        }
    }
}

/// Handles a client's request. Inside a MULTI transaction, commands other than EXEC,
/// DISCARD, MULTI, WATCH and PSYNC are queued until EXEC runs them all.
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
        Some(b"PSYNC") if command.len() == 3 => {
            if session.transaction.is_some() {
                return Value::Error("ERR Command not allowed inside a transaction".into());
            }
            let mut db = db.lock().unwrap();
            // Only masters have replicas, so `repl_offset` only ever means one thing.
            if db.is_replica {
                return Value::Error("ERR a replica can't have replicas of its own".into());
            }
            let (sender, receiver) = mpsc::unbounded_channel();
            session.replica = Some((db.add_replica(sender), receiver));
            // A full resync is all there is, whatever the replica asks for.
            Value::SimpleString(format!("FULLRESYNC {REPLICATION_ID} {}", db.repl_offset))
        }
        // The other ACL subcommands go through `run_command`, but WHOAMI needs the session.
        Some(b"ACL") if command.get(1).is_some_and(|sub| sub.eq_ignore_ascii_case(b"WHOAMI")) => {
            if command.len() != 2 {
                return wrong_arity("acl|whoami");
            }
            match &session.user {
                Some(user) => Value::BulkString(user.clone()),
                None => Value::Error("NOAUTH Authentication required.".into()),
            }
        }
        Some(b"MULTI") => wrong_arity("multi"),
        Some(b"EXEC") => wrong_arity("exec"),
        Some(b"DISCARD") => wrong_arity("discard"),
        Some(b"WATCH") => wrong_arity("watch"),
        Some(b"PSYNC") => wrong_arity("psync"),
        _ => match &mut session.transaction {
            Some(queued) => {
                queued.push(command);
                Value::SimpleString("QUEUED".into())
            }
            None => handle_command(&command, db).await,
        },
    }
}

/// Runs a command outside a transaction. BLPOP, XREAD BLOCK and WAIT can wait here, without
/// holding the lock; everything else runs straight through `execute`.
async fn handle_command(command: &[Vec<u8>], db: &Db) -> Value {
    if let [name, wanted, timeout] = command
        && name.eq_ignore_ascii_case(b"WAIT")
        && let Ok((wanted, timeout)) = parse_wait_args(wanted, timeout)
    {
        return wait_for_replicas(db, wanted, timeout).await;
    }
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

/// Runs a command against the locked database, and propagates it if it's a write. EXEC runs
/// a whole transaction through this without letting go of the lock in between.
fn execute(db: &mut Store, command: &[Vec<u8>]) -> Value {
    let reply = run_command(db, command);
    propagate_write(db, command, &reply);
    reply
}

/// Propagates `command`, which has just got `reply`, if it was a write that changed
/// something. BLPOP isn't here because `pop_now` deals with it.
fn propagate_write(db: &mut Store, command: &[Vec<u8>], reply: &Value) {
    let Some(name) = command.first() else {
        return;
    };
    match (name.to_ascii_uppercase().as_slice(), reply) {
        (_, Value::Error(_)) => {}
        (b"SET" | b"RPUSH" | b"LPUSH" | b"INCR", _) => db.propagate(command),
        (b"LPOP", Value::BulkString(_)) => db.propagate(command),
        (b"DEL", Value::Integer(removed)) if *removed > 0 => db.propagate(command),
        (b"ZADD" | b"SETBIT" | b"BITOP", Value::Integer(_)) => db.propagate(command),
        (b"GEOADD", Value::Integer(_)) => db.propagate(&geoadd_as_zadd(command)),
        (b"ZREM", Value::Integer(removed)) if *removed > 0 => db.propagate(command),
        (b"LPOP", Value::Array(popped)) if !popped.is_empty() => db.propagate(command),
        // With the ID spelled out, because a replica would generate a different one.
        (b"XADD", Value::BulkString(id)) => {
            let mut command = command.to_vec();
            command[2] = id.clone();
            db.propagate(&command);
        }
        _ => {}
    }
}

fn run_command(db: &mut Store, command: &[Vec<u8>]) -> Value {
    let Some((name, args)) = command.split_first() else {
        return Value::Error("ERR empty command".into());
    };

    match name.to_ascii_uppercase().as_slice() {
        b"PING" => match args {
            [] => Value::SimpleString("PONG".into()),
            [message] => Value::BulkString(message.clone()),
            _ => wrong_arity("ping"),
        },
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
                    Some(Data::SortedSet(_)) => "zset",
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
        // There's only one database, number 0, but replayed AOF files select it.
        b"SELECT" => match args {
            [index] if parse_number::<i64>(index) == Some(0) => Value::SimpleString("OK".into()),
            [index] if parse_number::<i64>(index).is_some() => {
                Value::Error("ERR DB index is out of range".into())
            }
            [_] => Value::Error("ERR value is not an integer or out of range".into()),
            _ => wrong_arity("select"),
        },
        b"PUBLISH" => match args {
            [channel, message] => publish(db, channel, message),
            _ => wrong_arity("publish"),
        },
        b"SETBIT" => match args {
            [key, offset, bit] => {
                let Some(offset) = bitmap::parse_offset(offset) else {
                    return Value::Error("ERR bit offset is not an integer or out of range".into());
                };
                let on = match parse_number::<i64>(bit) {
                    Some(0) => false,
                    Some(1) => true,
                    _ => return Value::Error("ERR bit is not an integer or out of range".into()),
                };
                let new = Data::String(Vec::new());
                let Data::String(bytes) = live_value_or_insert(db, key, new) else {
                    return wrong_type();
                };
                let was_on = bitmap::set_bit(bytes, offset, on);
                db.touch(key);
                Value::Integer(i64::from(was_on))
            }
            _ => wrong_arity("setbit"),
        },
        b"STRLEN" => match args {
            [key] => match live_value(db, key) {
                Some(Data::String(bytes)) => Value::Integer(bytes.len() as i64),
                Some(_) => wrong_type(),
                None => Value::Integer(0),
            },
            _ => wrong_arity("strlen"),
        },
        b"BITCOUNT" => match args {
            [key, range @ ..] if range.len() <= 3 => {
                // With no range, the whole string; otherwise bytes, or bits with BIT.
                let count: fn(&[u8], i64, i64) -> u64 = match range {
                    [_, _, mode] if mode.eq_ignore_ascii_case(b"BIT") => bitmap::count_in_bits,
                    [_, _, mode] if !mode.eq_ignore_ascii_case(b"BYTE") => {
                        return Value::Error("ERR syntax error".into());
                    }
                    [_] => return Value::Error("ERR syntax error".into()),
                    _ => bitmap::count_in_bytes,
                };
                let (start, end) = match range {
                    [] => (0, -1),
                    [start, end, ..] => match (parse_number(start), parse_number(end)) {
                        (Some(start), Some(end)) => (start, end),
                        _ => {
                            return Value::Error(
                                "ERR value is not an integer or out of range".into(),
                            );
                        }
                    },
                    _ => unreachable!(),
                };
                match live_value(db, key) {
                    Some(Data::String(bytes)) => Value::Integer(count(bytes, start, end) as i64),
                    Some(_) => wrong_type(),
                    None => Value::Integer(0),
                }
            }
            [_, ..] => Value::Error("ERR syntax error".into()),
            _ => wrong_arity("bitcount"),
        },
        b"BITOP" => match args {
            [op, dest, sources @ ..] if !sources.is_empty() => bitop(db, op, dest, sources),
            _ => wrong_arity("bitop"),
        },
        b"GETBIT" => match args {
            [key, offset] => {
                let Some(offset) = bitmap::parse_offset(offset) else {
                    return Value::Error("ERR bit offset is not an integer or out of range".into());
                };
                match live_value(db, key) {
                    Some(Data::String(bytes)) => Value::Integer(i64::from(bitmap::get_bit(bytes, offset))),
                    Some(_) => wrong_type(),
                    None => Value::Integer(0),
                }
            }
            _ => wrong_arity("getbit"),
        },
        b"GEOADD" => match args {
            [key, locations @ ..] if locations.len() >= 3 && locations.len() % 3 == 0 => {
                geoadd(db, key, locations)
            }
            [_, _, _, _, ..] => Value::Error(
                "ERR syntax error. Try GEOADD key [x1] [y1] [name1] [x2] [y2] [name2] ... ".into(),
            ),
            _ => wrong_arity("geoadd"),
        },
        b"GEODIST" => match args {
            [key, from, to, unit @ ..] if unit.len() <= 1 => {
                let to_meters = match unit {
                    [] => 1.0,
                    [unit] => match geo::unit_in_meters(unit) {
                        Some(meters) => meters,
                        None => return unsupported_unit(),
                    },
                    _ => unreachable!(),
                };
                let set = match live_value(db, key) {
                    Some(Data::SortedSet(set)) => set,
                    Some(_) => return wrong_type(),
                    None => return Value::NullBulkString,
                };
                let (Some(from), Some(to)) = (set.score(from), set.score(to)) else {
                    return Value::NullBulkString;
                };
                let meters = geo::distance(geo::decode(from as u64), geo::decode(to as u64));
                Value::BulkString(format!("{:.4}", meters / to_meters).into_bytes())
            }
            [_, _, _, ..] => Value::Error("ERR syntax error".into()),
            _ => wrong_arity("geodist"),
        },
        b"GEOSEARCH" => match args {
            [key, options @ ..] if options.len() >= 5 => geosearch(db, key, options),
            _ => wrong_arity("geosearch"),
        },
        b"GEOPOS" => match args {
            [key, members @ ..] => {
                let set = match live_value(db, key) {
                    Some(Data::SortedSet(set)) => Some(&*set),
                    Some(_) => return wrong_type(),
                    None => None,
                };
                // Each member's longitude and latitude, or a null array if it isn't there.
                let positions = members.iter().map(|member| {
                    match set.and_then(|set| set.score(member)) {
                        Some(score) => {
                            let (longitude, latitude) = geo::decode(score as u64);
                            Value::Array(vec![
                                Value::BulkString(zset::format_score(longitude).into_bytes()),
                                Value::BulkString(zset::format_score(latitude).into_bytes()),
                            ])
                        }
                        None => Value::NullArray,
                    }
                });
                Value::Array(positions.collect())
            }
            _ => wrong_arity("geopos"),
        },
        b"ZADD" => match args {
            [_, _, _, ..] if args.len() % 2 == 0 => {
                Value::Error("ERR syntax error".into())
            }
            [key, pairs @ ..] if pairs.len() >= 2 => zadd(db, key, pairs),
            _ => wrong_arity("zadd"),
        },
        b"ZRANGE" => match args {
            [key, start, stop, options @ ..] => {
                let with_scores = match options {
                    [] => false,
                    [option] if option.eq_ignore_ascii_case(b"WITHSCORES") => true,
                    _ => return Value::Error("ERR syntax error".into()),
                };
                let (Some(start), Some(stop)) = (parse_number(start), parse_number(stop)) else {
                    return Value::Error("ERR value is not an integer or out of range".into());
                };
                match live_value(db, key) {
                    Some(Data::SortedSet(set)) => {
                        let mut reply = Vec::new();
                        for (score, member) in set.range(start, stop) {
                            reply.push(Value::BulkString(member.clone()));
                            if with_scores {
                                let score = zset::format_score(*score);
                                reply.push(Value::BulkString(score.into_bytes()));
                            }
                        }
                        Value::Array(reply)
                    }
                    Some(_) => wrong_type(),
                    None => Value::Array(vec![]),
                }
            }
            _ => wrong_arity("zrange"),
        },
        b"ZSCORE" => match args {
            [key, member] => match live_value(db, key) {
                Some(Data::SortedSet(set)) => set.score(member).map_or(Value::NullBulkString, |score| {
                    Value::BulkString(zset::format_score(score).into_bytes())
                }),
                Some(_) => wrong_type(),
                None => Value::NullBulkString,
            },
            _ => wrong_arity("zscore"),
        },
        b"ZREM" => match args {
            [key, members @ ..] if !members.is_empty() => {
                let set = match live_value(db, key) {
                    Some(Data::SortedSet(set)) => set,
                    Some(_) => return wrong_type(),
                    None => return Value::Integer(0),
                };
                let removed = members.iter().filter(|member| set.remove(member)).count();
                // Redis deletes a sorted set along with its last member.
                if set.is_empty() {
                    db.entries.remove(key);
                }
                if removed > 0 {
                    db.touch(key);
                }
                Value::Integer(removed as i64)
            }
            _ => wrong_arity("zrem"),
        },
        b"ZCARD" => match args {
            [key] => match live_value(db, key) {
                Some(Data::SortedSet(set)) => Value::Integer(set.len() as i64),
                Some(_) => wrong_type(),
                None => Value::Integer(0),
            },
            _ => wrong_arity("zcard"),
        },
        b"ZRANK" => match args {
            [key, member] => match live_value(db, key) {
                Some(Data::SortedSet(set)) => set
                    .rank(member)
                    .map_or(Value::NullBulkString, |rank| Value::Integer(rank as i64)),
                Some(_) => wrong_type(),
                None => Value::NullBulkString,
            },
            [key, member, option] if option.eq_ignore_ascii_case(b"WITHSCORE") => {
                match live_value(db, key) {
                    Some(Data::SortedSet(set)) => match (set.rank(member), set.score(member)) {
                        (Some(rank), Some(score)) => Value::Array(vec![
                            Value::Integer(rank as i64),
                            Value::BulkString(zset::format_score(score).into_bytes()),
                        ]),
                        _ => Value::NullArray,
                    },
                    Some(_) => wrong_type(),
                    None => Value::NullArray,
                }
            }
            [_, _, _] => Value::Error("ERR syntax error".into()),
            _ => wrong_arity("zrank"),
        },
        b"KEYS" => match args {
            [pattern] => matching_keys(db, pattern),
            _ => wrong_arity("keys"),
        },
        b"DEL" => match args {
            [] => wrong_arity("del"),
            keys => {
                let mut removed = 0;
                for key in keys {
                    // A key that has expired counts as already gone.
                    remove_if_expired(db, key);
                    if db.entries.remove(key).is_some() {
                        db.touch(key);
                        removed += 1;
                    }
                }
                Value::Integer(removed)
            }
        },
        // `handle_request` handles UNWATCH outside a transaction. Inside one, it's left
        // with nothing to do: EXEC has already stopped watching by the time it runs.
        b"UNWATCH" => match args {
            [] => Value::SimpleString("OK".into()),
            _ => wrong_arity("unwatch"),
        },
        b"INFO" => info(db, args),
        b"ACL" => match args {
            [] => wrong_arity("acl"),
            [subcommand, rest @ ..] => match subcommand.to_ascii_uppercase().as_slice() {
                b"GETUSER" => match rest {
                    [name] => match db.users.get(name) {
                        Some(_) => Value::Array(vec![
                            Value::BulkString(b"flags".to_vec()),
                            Value::Array(vec![]),
                        ]),
                        None => Value::NullBulkString,
                    },
                    _ => wrong_arity("acl|getuser"),
                },
                _ => Value::Error(format!(
                    "ERR unknown subcommand '{}'. Try ACL HELP.",
                    String::from_utf8_lossy(subcommand)
                )),
            },
        },
        b"CONFIG" => config_command(&db.config, args),
        // Nothing uses what a replica says about itself in the handshake. The REPLCONFs that
        // do matter, ACK and GETACK, are dealt with on the replication connection.
        b"REPLCONF" => Value::SimpleString("OK".into()),
        b"WAIT" => match args {
            // Only inside a transaction does a valid WAIT get here (`handle_command` runs it
            // otherwise), and there it doesn't wait, as in Redis.
            [wanted, timeout] => match parse_wait_args(wanted, timeout) {
                Ok(_) => {
                    let offset = db.write_offset;
                    Value::Integer(db.replicas_at(offset) as i64)
                }
                Err(msg) => Value::Error(msg.into()),
            },
            _ => wrong_arity("wait"),
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

/// Parses the options after `SET key value`. So far only an expiry is supported: in
/// `EX seconds` or `PX milliseconds` from now, or at the Unix time `EXAT seconds` or
/// `PXAT milliseconds`.
fn parse_expiry(options: &[Vec<u8>]) -> Result<Option<Instant>, &'static str> {
    let (unit, amount) = match options {
        [] => return Ok(None),
        [unit, amount] => (unit.to_ascii_uppercase(), amount),
        _ => return Err("ERR syntax error"),
    };
    // How long from now until the key expires, given the amount: EX and PX count from
    // now, and EXAT and PXAT give a Unix time, which may already have passed.
    let from_now: fn(u64) -> Duration = match unit.as_slice() {
        b"EX" => Duration::from_secs,
        b"PX" => Duration::from_millis,
        b"EXAT" => |secs| Duration::from_millis(secs.saturating_mul(1000).saturating_sub(unix_time_ms())),
        b"PXAT" => |ms| Duration::from_millis(ms.saturating_sub(unix_time_ms())),
        _ => return Err("ERR syntax error"),
    };
    let Some(amount) = parse_number::<u64>(amount) else {
        return Err("ERR value is not an integer or out of range");
    };
    // checked_add so that an absurdly long expiry is an error rather than a panic.
    match Instant::now().checked_add(from_now(amount)) {
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

/// Parses a timeout in milliseconds, where 0 means wait forever.
fn parse_timeout_ms(arg: &[u8]) -> Result<Option<Duration>, &'static str> {
    let Some(ms) = parse_number::<i64>(arg) else {
        return Err("ERR timeout is not an integer or out of range");
    };
    if ms < 0 {
        return Err("ERR timeout is negative");
    }
    Ok((ms > 0).then(|| Duration::from_millis(ms as u64)))
}

/// Parses WAIT's arguments: how many replicas to wait for, and for how long.
fn parse_wait_args(wanted: &[u8], timeout: &[u8]) -> Result<(i64, Option<Duration>), &'static str> {
    let Some(wanted) = parse_number(wanted) else {
        return Err("ERR value is not an integer or out of range");
    };
    Ok((wanted, parse_timeout_ms(timeout)?))
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
        let mut served = 0;
        if let Some(Data::List(list)) = db.entries.get_mut(&key).map(|entry| &mut entry.value) {
            while !list.is_empty() {
                let Some(waiter) = waiters.pop_front() else {
                    break;
                };
                let element = list.pop_front().unwrap();
                match waiter.send(element) {
                    Ok(()) => served += 1,
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
        for _ in 0..served {
            db.touch(&key);
            db.propagate_pop(&key);
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
    db.propagate_pop(key);
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
    // If the clock has gone backwards, carry on from the last entry's time.
    let ms = ms.unwrap_or_else(|| unix_time_ms().max(last.ms));
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
            (Some(parse_timeout_ms(ms)?), rest)
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

/// BITOP: combines the strings at `sources` with the operation `op`, stores the result at
/// `dest`, and replies with its length. A missing source counts as an empty string, and an
/// empty result deletes `dest`.
fn bitop(db: &mut Store, op: &[u8], dest: &[u8], sources: &[Vec<u8>]) -> Value {
    let Some(op) = bitmap::BitOp::parse(op) else {
        return Value::Error("ERR syntax error".into());
    };
    if op == bitmap::BitOp::Not && sources.len() != 1 {
        return Value::Error("ERR BITOP NOT must be called with a single source key.".into());
    }
    let mut values = Vec::new();
    for key in sources {
        match live_value(db, key) {
            Some(Data::String(bytes)) => values.push(bytes.clone()),
            Some(_) => return wrong_type(),
            None => values.push(Vec::new()),
        }
    }
    let values: Vec<&[u8]> = values.iter().map(Vec::as_slice).collect();
    let result = bitmap::combine(op, &values);
    let len = result.len();
    if result.is_empty() {
        if db.entries.remove(dest).is_some() {
            db.touch(dest);
        }
    } else {
        // The result replaces whatever was at `dest`, along with any expiry.
        let entry = Entry {
            value: Data::String(result),
            expires_at: None,
        };
        db.entries.insert(dest.to_vec(), entry);
        db.touch(dest);
    }
    Value::Integer(len as i64)
}

/// ZADD: adds each member with the score before it, or gives a member that's already
/// there its new score. Replies with how many members were new.
fn zadd(db: &mut Store, key: &[u8], pairs: &[Vec<u8>]) -> Value {
    // Every score is checked before anything changes.
    let mut members = Vec::new();
    for pair in pairs.chunks(2) {
        let Some(score) = zset::parse_score(&pair[0]) else {
            return Value::Error("ERR value is not a valid float".into());
        };
        members.push((score, &pair[1]));
    }
    add_to_sorted_set(db, key, members)
}

/// Adds members with their scores to the sorted set at `key`, creating it if needed, and
/// replies with how many members were new.
fn add_to_sorted_set(db: &mut Store, key: &[u8], members: Vec<(f64, &Vec<u8>)>) -> Value {
    let new_set = Data::SortedSet(SortedSet::default());
    let Data::SortedSet(set) = live_value_or_insert(db, key, new_set) else {
        return wrong_type();
    };
    let mut added = 0;
    for (score, member) in members {
        if set.insert(member, score) {
            added += 1;
        }
    }
    db.touch(key);
    Value::Integer(added)
}

/// GEOADD: adds each location, given as longitude, latitude and member, to the sorted set
/// at `key`, with its geohash as its score. Replies with how many members were new.
fn geoadd(db: &mut Store, key: &[u8], locations: &[Vec<u8>]) -> Value {
    // Every location is checked before anything changes.
    let mut members = Vec::new();
    for location in locations.chunks(3) {
        let (Some(longitude), Some(latitude)) =
            (parse_number::<f64>(&location[0]), parse_number::<f64>(&location[1]))
        else {
            return Value::Error("ERR value is not a valid float".into());
        };
        if !geo::is_valid(longitude, latitude) {
            return Value::Error(format!(
                "ERR invalid longitude,latitude pair {longitude:.6},{latitude:.6}"
            ));
        }
        members.push((geo::encode(longitude, latitude) as f64, &location[2]));
    }
    add_to_sorted_set(db, key, members)
}

/// GEOSEARCH, with the center given by `FROMLONLAT <longitude> <latitude>` and the area by
/// `BYRADIUS <radius> <unit>`, the only options supported: the members of the sorted set
/// at `key` whose locations are within the circle, in the set's order.
fn geosearch(db: &mut Store, key: &[u8], options: &[Vec<u8>]) -> Value {
    let (mut center, mut radius) = (None, None);
    let mut rest = options;
    while let [option, ..] = rest {
        match (option.to_ascii_uppercase().as_slice(), rest) {
            (b"FROMLONLAT", [_, longitude, latitude, after @ ..]) => {
                let (Some(longitude), Some(latitude)) =
                    (parse_number::<f64>(longitude), parse_number::<f64>(latitude))
                else {
                    return Value::Error("ERR value is not a valid float".into());
                };
                if !geo::is_valid(longitude, latitude) {
                    return Value::Error(format!(
                        "ERR invalid longitude,latitude pair {longitude:.6},{latitude:.6}"
                    ));
                }
                center = Some((longitude, latitude));
                rest = after;
            }
            (b"BYRADIUS", [_, distance, unit, after @ ..]) => {
                let Some(distance) = parse_number::<f64>(distance) else {
                    return Value::Error("ERR need numeric radius".into());
                };
                if distance < 0.0 {
                    return Value::Error("ERR radius cannot be negative".into());
                }
                let Some(to_meters) = geo::unit_in_meters(unit) else {
                    return unsupported_unit();
                };
                radius = Some(distance * to_meters);
                rest = after;
            }
            _ => return Value::Error("ERR syntax error".into()),
        }
    }
    let Some(center) = center else {
        return Value::Error(
            "ERR exactly one of FROMMEMBER or FROMLONLAT can be specified for GEOSEARCH".into(),
        );
    };
    let Some(radius) = radius else {
        return Value::Error(
            "ERR exactly one of BYRADIUS and BYBOX can be specified for GEOSEARCH".into(),
        );
    };
    let set = match live_value(db, key) {
        Some(Data::SortedSet(set)) => set,
        Some(_) => return wrong_type(),
        None => return Value::Array(vec![]),
    };
    let found = set
        .iter()
        .filter(|(score, _)| geo::distance(center, geo::decode(*score as u64)) <= radius)
        .map(|(_, member)| Value::BulkString(member.to_vec()));
    Value::Array(found.collect())
}

fn unsupported_unit() -> Value {
    Value::Error("ERR unsupported unit provided. please use M, KM, FT, MI".into())
}

/// GEOADD as Redis passes it on to replicas and the AOF: as the ZADD it amounts to.
fn geoadd_as_zadd(command: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let mut zadd = vec![b"ZADD".to_vec(), command[1].clone()];
    for location in command[2..].chunks(3) {
        let longitude = parse_number::<f64>(&location[0]).unwrap_or_default();
        let latitude = parse_number::<f64>(&location[1]).unwrap_or_default();
        zadd.push(geo::encode(longitude, latitude).to_string().into_bytes());
        zadd.push(location[2].clone());
    }
    zadd
}

/// KEYS: every key that matches `pattern`, in no particular order. Keys that have expired
/// are removed, as looking at them would.
fn matching_keys(db: &mut Store, pattern: &[u8]) -> Value {
    let keys: Vec<Vec<u8>> = db.entries.keys().cloned().collect();
    let mut found = Vec::new();
    for key in keys {
        remove_if_expired(db, &key);
        if db.entries.contains_key(&key) && glob::matches(pattern, &key, false) {
            found.push(Value::BulkString(key));
        }
    }
    Value::Array(found)
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
    let offset = db.repl_offset;
    let text = format!(
        "role:{role}\r\n\
         master_replid:{REPLICATION_ID}\r\n\
         master_repl_offset:{offset}\r\n"
    );
    Value::BulkString(text.into_bytes())
}

/// CONFIG, of which only CONFIG GET is supported: the name and value of each parameter
/// whose name matches one of the patterns given, all in one flat array.
fn config_command(config: &Config, args: &[Vec<u8>]) -> Value {
    let Some((subcommand, patterns)) = args.split_first() else {
        return wrong_arity("config");
    };
    if !subcommand.eq_ignore_ascii_case(b"GET") {
        return Value::Error(format!(
            "ERR unknown subcommand '{}'. Try CONFIG HELP.",
            String::from_utf8_lossy(subcommand)
        ));
    }
    if patterns.is_empty() {
        return wrong_arity("config|get");
    }
    let mut reply = Vec::new();
    for (name, value) in config.params() {
        if patterns
            .iter()
            .any(|pattern| glob::matches(pattern, name.as_bytes(), true))
        {
            reply.push(Value::BulkString(name.into()));
            reply.push(Value::BulkString(value.into_bytes()));
        }
    }
    Value::Array(reply)
}

/// WAIT outside a transaction: replies with how many replicas have acknowledged all the
/// writes so far, as soon as that's at least `wanted`, or else after `timeout` (or never,
/// if there's no timeout).
async fn wait_for_replicas(db: &Db, wanted: i64, timeout: Option<Duration>) -> Value {
    let (offset, replicas_changed) = {
        let mut db = db.lock().unwrap();
        let offset = db.write_offset;
        let caught_up = db.replicas_at(offset);
        if caught_up as i64 >= wanted {
            return Value::Integer(caught_up as i64);
        }
        // Replicas only acknowledge what they've got when asked, so ask if any are behind.
        if caught_up < db.replicas.len() {
            db.send_to_replicas(&["REPLCONF", "GETACK", "*"]);
        }
        (offset, db.replicas_changed.clone())
    };
    let wait = async {
        loop {
            // Listen for changes before counting, so that one in between isn't missed.
            let changed = replicas_changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let caught_up = db.lock().unwrap().replicas_at(offset);
            if caught_up as i64 >= wanted {
                return caught_up;
            }
            changed.await;
        }
    };
    let caught_up = match timeout {
        None => wait.await,
        Some(timeout) => match tokio::time::timeout(timeout, wait).await {
            Ok(caught_up) => caught_up,
            Err(_) => db.lock().unwrap().replicas_at(offset),
        },
    };
    Value::Integer(caught_up as i64)
}

/// The argument of a `REPLCONF <option> <argument>` command, if that's what `request` is.
fn replconf_argument<'a>(request: &'a Value, option: &[u8]) -> Option<&'a [u8]> {
    let Value::Array(args) = request else {
        return None;
    };
    match args.as_slice() {
        [
            Value::BulkString(name),
            Value::BulkString(given),
            Value::BulkString(argument),
        ] if name.eq_ignore_ascii_case(b"REPLCONF") && given.eq_ignore_ascii_case(option) => {
            Some(argument)
        }
        _ => None,
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
