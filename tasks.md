# Tasks: Build your own Redis

Stage descriptions from the CodeCrafters [Build your own Redis](https://app.codecrafters.io/courses/redis/overview) course, from **RDB Persistence → RDB file config** through **Authentication → Authenticate using AUTH**, in the order the site lists them.

- RDB Persistence (6 stages)
- AOF Persistence (10 stages)
- Pub/Sub (7 stages)
- Sorted Sets (8 stages)
- Bitmaps (9 stages)
- Geospatial Commands (8 stages)
- Authentication (8 stages)

## RDB Persistence

### RDB file config

<https://app.codecrafters.io/courses/redis/stages/zg5>

Welcome to the RDB Persistence Extension! In this extension, you'll add support for reading [RDB files](https://redis.io/docs/latest/operate/oss_and_stack/management/persistence/) (Redis Database files).

In this stage, you'll add support for two configuration parameters related to RDB persistence, as well as the [CONFIG GET](https://redis.io/docs/latest/commands/config-get/) command.

#### RDB files

An RDB file is a point-in-time snapshot of a Redis dataset. When RDB persistence is enabled, the Redis server syncs its in-memory state with an RDB file, by doing the following:

1. On startup, the Redis server loads the data from the RDB file.
2. While running, the Redis server periodically takes new snapshots of the dataset, in order to update the RDB file.

#### `dir` and `dbfilename`

The configuration parameters `dir` and `dbfilename` specify where an RDB file is stored:
- `dir` - the path to the directory where the RDB file is stored (example: `/tmp/redis-data`)
- `dbfilename` - the name of the RDB file (example: `rdbfile`)

#### The `CONFIG GET` command

The [`CONFIG GET`](https://redis.io/docs/latest/commands/config-get/) command returns the values of configuration parameters.

It takes in one or more configuration parameters and returns a [RESP array](https://redis.io/docs/latest/develop/reference/protocol-spec/#arrays) of key-value pairs:

```bash
$ redis-cli CONFIG GET dir
1) "dir"
2) "/tmp/redis-data"
```

Although `CONFIG GET` can fetch multiple parameters at a time, the tester will only send `CONFIG GET` commands with one parameter at a time.

#### Tests

The tester will execute your program like this:

```bash
./your_program.sh --dir /tmp/redis-files --dbfilename dump.rdb
```

It'll then send the following commands:

```bash
$ redis-cli CONFIG GET dir
$ redis-cli CONFIG GET dbfilename
```

Your server must respond to each `CONFIG GET` command with a RESP array containing two elements:

1. The parameter **name**, encoded as a [RESP Bulk string](https://redis.io/docs/latest/develop/reference/protocol-spec/#bulk-strings)
2. The parameter **value**, encoded as a RESP Bulk string

For example, if the value of `dir` is `/tmp/redis-files`, then the expected response to `CONFIG GET dir` is:

```bash
*2\r\n$3\r\ndir\r\n$16\r\n/tmp/redis-files\r\n
```

#### Notes

- You don't need to read the RDB file in this stage, you only need to store `dir` and `dbfilename`. Reading from the file will be covered in later stages.
- If your repository was created before 5th Oct 2023, it's possible that your `./your_program.sh` script is not passing arguments to your program. To fix this, you'll need to edit `./your_program.sh`. Check the [update CLI args PR](https://github.com/codecrafters-io/build-your-own-redis/pull/89/files) for details on how to do this.

### Read a key

<https://app.codecrafters.io/courses/redis/stages/jz6>

In this stage, you'll add support for reading a single key from an RDB file.

#### RDB file format

<details>
  <summary>Click to expand/collapse</summary>
##### RDB file format overview

Here are the different sections of the RDB file, in order:

1.  Header section
2.  Metadata section
3.  Database section
4.  End of file section

RDB files use special encodings to store different types of data. The ones relevant to this stage are "size encoding" and "string encoding." These are explained near the end of this page.

The following breakdown of the RDB file format is based on [Redis RDB File Format](https://rdb.fnordig.de/file_format.html) by Jan-Erik Rediger. We’ve only included the parts that are relevant to this stage.

##### Header section

RDB files begin with a header section, which looks something like this:
```
52 45 44 49 53 30 30 31 31  // Magic string + version number (ASCII): "REDIS0011".
```

The header contains the magic string `REDIS`, followed by a four-character RDB version number. In this challenge, the test RDB files all use version 11. So, the header is always `REDIS0011`.

##### Metadata section

Next is the metadata section. It contains zero or more "metadata subsections," which each specify a single metadata attribute. Here's an example of a metadata subsection that specifies `redis-ver`:
```
FA                             // Indicates the start of a metadata subsection.
09 72 65 64 69 73 2D 76 65 72  // The name of the metadata attribute (string encoded): "redis-ver".
06 36 2E 30 2E 31 36           // The value of the metadata attribute (string encoded): "6.0.16".
```

The metadata name and value are always string encoded.

##### Database section

Next is the database section. It contains zero or more "database subsections," which each describe a single database. Here's an example of a database subsection:
```
FE                       // Indicates the start of a database subsection.
00                       /* The index of the database (size encoded).
                            Here, the index is 0. */

FB                       // Indicates that hash table size information follows.
03                       /* The size of the hash table that stores the keys and values (size encoded).
                            Here, the total key-value hash table size is 3. */
02                       /* The size of the hash table that stores the expires of the keys (size encoded).
                            Here, the number of keys with an expiry is 2. */
```

```
00                       /* The 1-byte flag that specifies the value’s type and encoding.
                            Here, the flag is 0, which means "string." */
06 66 6F 6F 62 61 72     // The name of the key (string encoded). Here, it's "foobar".
06 62 61 7A 71 75 78     // The value (string encoded). Here, it's "bazqux".
```

```
FC                       /* Indicates that this key ("foo") has an expire,
                            and that the expire timestamp is expressed in milliseconds. */
15 72 E7 07 8F 01 00 00  /* The expire timestamp, expressed in Unix time,
                            stored as an 8-byte unsigned long, in little-endian (read right-to-left).
                            Here, the expire timestamp is 1713824559637. */
00                       // Value type is string.
03 66 6F 6F              // Key name is "foo".
03 62 61 72              // Value is "bar".
```

```
FD                       /* Indicates that this key ("baz") has an expire,
                            and that the expire timestamp is expressed in seconds. */
52 ED 2A 66              /* The expire timestamp, expressed in Unix time,
                            stored as an 4-byte unsigned integer, in little-endian (read right-to-left).
                            Here, the expire timestamp is 1714089298. */
00                       // Value type is string.
03 62 61 7A              // Key name is "baz".
03 71 75 78              // Value is "qux".
```

Here's a more formal description of how each key-value pair is stored:

1. Optional expire information (one of the following):
    * Timestamp in seconds:
          1.  `FD`
          2.  Expire timestamp in seconds (4-byte unsigned integer)
    * Timestamp in milliseconds:
          1.  `FC`
          2.  Expire timestamp in milliseconds (8-byte unsigned long)
2. Value type (1-byte flag)
3. Key (string encoded)
4. Value (encoding depends on value type)

##### End of file section

This section marks the end of the file. It looks something like this:
```
FF                       /* Indicates that the file is ending,
                            and that the checksum follows. */
89 3b b7 4e f8 0f 77 19  // An 8-byte CRC64 checksum of the entire file.
```

##### Length encoding

Length-encoded values specify the length or size of something. Here are some examples:
- The database indexes and hash table sizes are length encoded.
- String encoding begins with a length-encoded value that specifies the number of characters in the string.
- List encoding begins with a length-encoded value that specifies the number of elements in the list.

The first (most significant) two bits of a length-encoded value indicate how the value should be parsed. Here's a guide (bits are shown in both hexadecimal and binary):
```
/* If the first two bits are 0b00:
   The length is the remaining 6 bits of the byte.
   In this example, the length is 10: */
0A
00001010

/* If the first two bits are 0b01:
   The length is the next 14 bits
   (remaining 6 bits in the first byte, combined with the next byte),
   in big-endian (read left-to-right).
   In this example, the length is 700: */
42 BC
01000010 10111100

/* If the first two bits are 0b10:
   Ignore the remaining 6 bits of the first byte.
   The length is the next 4 bytes, in big-endian (read left-to-right).
   In this example, the length is 17000: */
80 00 00 42 68
10000000 00000000 00000000 01000010 01101000

/* If the first two bits are 0b11:
   The remaining 6 bits specify a type of string encoding.
   See string encoding section. */
```

##### String encoding

A string-encoded value consists of two parts:
1.  The size of the string (size encoded).
2.  The string.

Here's an example:
```
/* The 0x0D size specifies that the string is 13 characters long.
   The remaining characters spell out "Hello, World!". */
0D 48 65 6C 6C 6F 2C 20 57 6F 72 6C 64 21
```

For sizes that begin with `0b11`, the remaining 6 bits indicate a type of string format:
```
/* The 0xC0 size indicates the string is an 8-bit integer.
   In this example, the string is "123". */
C0 7B

/* The 0xC1 size indicates the string is a 16-bit integer.
   The remaining bytes are in little-endian (read right-to-left).
   In this example, the string is "12345". */
C1 39 30

/* The 0xC2 size indicates the string is a 32-bit integer.
   The remaining bytes are in little-endian (read right-to-left),
   In this example, the string is "1234567". */
C2 87 D6 12 00

/* The 0xC3 size indicates that the string is compressed with the LZF algorithm.
   You will not encounter LZF-compressed strings in this challenge. */
C3 ...
```
</details>


#### The `KEYS` command
<details>
  <summary>Click to expand/collapse</summary>

The [`KEYS command`](https://redis.io/docs/latest/commands/keys/) returns all the keys that match a given pattern, as a RESP array:
```
$ redis-cli SET foo bar
OK
$ redis-cli SET baz qux
OK
$ redis-cli KEYS "f*"
1) "foo"
```

When the pattern is `*`, the command returns all the keys in the database:
```
$ redis-cli KEYS "*"
1) "baz"
2) "foo"
```

In this stage, you must add support for the `KEYS` command. However, you only need to support the `*` pattern.
</details>

#### Tests

The tester will create an RDB file with a single key and execute your program like this:
```
$ ./your_program.sh --dir <dir> --dbfilename <filename>
```

It'll then send a `KEYS "*"` command to your server.
```
$ redis-cli KEYS "*"
```

Your server must respond with a RESP array that contains the key from the RDB file:
```
*1\r\n$3\r\nfoo\r\n
```

#### Notes

- The RDB file provided by `--dir` and `--dbfilename` might not exist. If the file doesn't exist, your program must treat the database as empty.
- RDB files use both little-endian and big-endian to store numbers. See the [MDN article on endianness](https://developer.mozilla.org/en-US/docs/Glossary/Endianness) to learn more.
- To generate an RDB file, use the [`SAVE` command](https://redis.io/docs/latest/commands/save/).

### Read a string value

<https://app.codecrafters.io/courses/redis/stages/gc6>

In this stage, you'll add support for reading the value corresponding to a key from an RDB file.

Just like with previous stages, we'll stick to supporting RDB files that contain a single key for now.

The tester will create an RDB file with a single key and execute your program like this:

```
./your_program.sh --dir <dir> --dbfilename <filename>
```

It'll then send a `GET <key>` command to your server.

```bash
$ redis-cli GET "foo"
```

The response to `GET <key>` should be a RESP bulk string with the value of the key.

For example, let's say the RDB file contains a key called `foo` with the value `bar`. The expected response will be `$3\r\nbar\r\n`.

Strings can be encoded in three different ways in the RDB file format:

- Length-prefixed strings
- Integers as strings
- Compressed strings

In this stage, you only need to support length-prefixed strings. We won't cover the other two types in this challenge.

We recommend using [this blog post](https://rdb.fnordig.de/file_format.html) as a reference when working on this stage.

### Read multiple keys

<https://app.codecrafters.io/courses/redis/stages/jw4>

In this stage, you'll add support for reading multiple keys from an RDB file.

The tester will create an RDB file with multiple keys and execute your program like this:

```bash
$ ./your_program.sh --dir <dir> --dbfilename <filename>
```

It'll then send a `KEYS *` command to your server.

```bash
$ redis-cli KEYS "*"
```

The response to `KEYS *` should be a RESP array with the keys as elements.

For example, let's say the RDB file contains two keys: `foo` and `bar`. The expected response will be:

```
*2\r\n$3\r\nfoo\r\n$3\r\nbar\r\n
```

- `*2\r\n` indicates that the array has two elements
- `$3\r\nfoo\r\n` indicates that the first element is a bulk string with the value `foo`
- `$3\r\nbar\r\n` indicates that the second element is a bulk string with the value `bar`

### Read multiple string values

<https://app.codecrafters.io/courses/redis/stages/dq3>

In this stage, you'll add support for reading multiple string values from an RDB file.

The tester will create an RDB file with multiple keys and execute your program like this:

```bash
$ ./your_program.sh --dir <dir> --dbfilename <filename>
```

It'll then send multiple `GET <key>` commands to your server.

```bash
$ redis-cli GET "foo"
$ redis-cli GET "bar"
```

The response to each `GET <key>` command should be a RESP bulk string with the value corresponding to the key.

### Read value with expiry

<https://app.codecrafters.io/courses/redis/stages/sm4>

In this stage, you'll add support for reading values that have an expiry set.

The tester will create an RDB file with multiple keys. Some of these keys will have an expiry set, and some won't. The expiry timestamps
will also be random, some will be in the past and some will be in the future.

The tester will execute your program like this:

```bash
$ ./your_program.sh --dir <dir> --dbfilename <filename>
```

It'll then send multiple `GET <key>` commands to your server.

```bash
$ redis-cli GET "foo"
$ redis-cli GET "bar"
```

When a key has expired, the expected response is `$-1\r\n` (a "null bulk string").

When a key hasn't expired, the expected response is a RESP bulk string with the value corresponding to the key.

## AOF Persistence

### Default AOF options

<https://app.codecrafters.io/courses/redis/stages/uj3>

In this stage, you'll set up default values for AOF-related configuration options.

#### AOF Persistence in Redis

Redis keeps all its data in memory, which makes it fast but vulnerable. If the server crashes, everything is gone. AOF (Append Only File) is one way to solve this. When enabled, the server logs every write command to a file, and on restart, it replays the file to rebuild its state.

The AOF behavior is controlled by several configuration options. For this stage, you'll set up their default values so they can be retrieved with `CONFIG GET`. You don't need to implement any actual persistence logic yet.

Here are the options and their defaults:

| Option | Purpose | Default |
|---|---|---|
| `dir` | The base directory where Redis stores its data files | Current working directory at startup |
| `appendonly` | Controls whether AOF persistence is enabled or disabled | `no` |
| `appenddirname` | The subdirectory under `dir` where AOF and manifest files are stored | `appendonlydir` |
| `appendfilename` | The name of the append-only file that records write operations | `appendonly.aof` |
| `appendfsync` | How often buffered writes are flushed to the AOF file on disk | `everysec` |

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then query each option using `CONFIG GET`:

```bash
$ redis-cli CONFIG GET dir
1) "dir"
2) "/path/to/current/directory"

$ redis-cli CONFIG GET appendonly
1) "appendonly"
2) "no"

$ redis-cli CONFIG GET appenddirname
1) "appenddirname"
2) "appendonlydir"

$ redis-cli CONFIG GET appendfilename
1) "appendfilename"
2) "appendonly.aof"

$ redis-cli CONFIG GET appendfsync
1) "appendfsync"
2) "everysec"
```

Each response is a RESP array with two [bulk strings](https://redis.io/docs/latest/develop/reference/protocol-spec/#bulk-strings): the option name and its value. For example, `CONFIG GET appendonly` should return:
 
```
*2\r\n$10\r\nappendonly\r\n$2\r\nno\r\n
```

The tester will verify that:

- Each `CONFIG GET` returns a two-element RESP array
- The option name and default value are correct for each option

#### Notes

- You don't need to implement any AOF persistence logic in this stage. Just store and return the default values.
- If you've already implemented `CONFIG GET` for other options in a previous challenge, you may just need to add these new keys to your existing configuration.

### AOF options from flags

<https://app.codecrafters.io/courses/redis/stages/vd9>

In this stage, you'll accept AOF configuration values from command-line flags.

#### Overriding Defaults With Flags

In previous stages, you set up default values for AOF options. Now you'll allow those defaults to be overridden by command-line flags: `--dir`, `--appendonly`, `--appenddirname`, `--appendfilename`, and `--appendfsync`.

When a flag is provided, its value takes precedence over the default. When a flag is omitted, the default from previous stages still applies.

#### Tests

The tester will execute your program with some or all of the flags:

```bash
$ ./your_program.sh --dir <dir> --appendonly yes --appenddirname <append_dir_name> --appendfilename <append_file_name> --appendfsync everysec
```

It will then query each option using `CONFIG GET`:

```bash
$ redis-cli CONFIG GET dir
$ redis-cli CONFIG GET appendonly
$ redis-cli CONFIG GET appenddirname
$ redis-cli CONFIG GET appendfilename
$ redis-cli CONFIG GET appendfsync
```

Each response should return the value passed via the corresponding flag. For example, if the program is started with `--appendonly yes`, then `CONFIG GET appendonly` should return:

```
*2\r\n$10\r\nappendonly\r\n$3\r\nyes\r\n
```

The tester will verify that:

- Each `CONFIG GET` returns the value from the corresponding command-line flag
- Options without flags still return their default values

#### Notes

- You don't need to implement any AOF persistence logic in this stage. Just parse the flags and return the values from `CONFIG GET`.
- If you already parse command-line flags for other options (like `--port`), you can follow the same approach for these new flags.
- You don't need to handle duplicate flags or any specific flag ordering. The tester will pass each flag at most once.

### Create append-only directory

<https://app.codecrafters.io/courses/redis/stages/fm0>

In this stage, you'll create the append-only directory when AOF persistence is enabled.

#### Creating the Append-Only Directory

Now that your server can accept AOF configuration from command-line flags, it's time to start acting on them. When the server starts with `--appendonly yes`, it needs a directory to store its AOF files in. This directory lives inside the `dir` path and is named according to the `appenddirname` option.

For example, if the server is started with:

```bash
$ ./your_program.sh --dir /tmp/redis-data --appendonly yes --appenddirname appendonlydir
```

It should create the directory `/tmp/redis-data/appendonlydir/` if it doesn't already exist.

If `--appendonly` is not set to `yes`, the directory should not be created.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh --dir <dir> --appendonly yes --appenddirname <append_dir_name> --appendfilename <append_file_name>
```

The tester will verify that:

- The directory `<dir>/<append_dir_name>` has been created
- The directory exists before any client commands are sent (it should be created at startup)

#### Notes

- You don't need to create any files inside the directory yet. That comes in later stages.
- If the directory already exists, your server should not return an error.

### Create append-only file

<https://app.codecrafters.io/courses/redis/stages/dw4>

In this stage, you'll create the append-only file when AOF persistence is enabled.

#### Creating the Append-Only File

Along with the append-only directory, your server also needs to create an empty AOF file inside it at startup.

The file follows a specific naming convention: `<appendfilename>.1.incr.aof`. The `.1.incr` suffix indicates this is the first incremental AOF file. Redis uses incremental files to record commands as they come in, which it can replay later to restore state after a restart.

For example, if the server is started with:

```bash
$ ./your_program.sh --dir /tmp/redis-data --appendonly yes --appenddirname appendonlydir --appendfilename appendonly.aof
```

It should create an empty file at `/tmp/redis-data/appendonlydir/appendonly.aof.1.incr.aof`.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh --dir <dir> --appendonly yes --appenddirname <append_dir_name> --appendfilename <append_file_name>
```

The tester will then verify that:

- The directory `<dir>/<append_dir_name>` exists
- An empty file `<dir>/<append_dir_name>/<append_file_name>.1.incr.aof` is created

#### Notes

- The file should be created at startup, not on the first write command.
- You don't need to write any content to the file yet. That comes in later stages.

### Create manifest file

<https://app.codecrafters.io/courses/redis/stages/pb9>

In this stage, you'll create a manifest file alongside the append-only file.

#### The Manifest File

The manifest file tells Redis which AOF files exist and in what order they should be replayed during recovery. It follows the naming convention `<appendfilename>.manifest` and lives in the same directory as the AOF file. 

The manifest file contains a single line describing the AOF file you created earlier:

```
file <appendfilename>.1.incr.aof seq 1 type i
```

Here's what each part means:

| Field | Value | Meaning |
|---|---|---|
| `file` | `<appendfilename>.1.incr.aof` | The name of the AOF file |
| `seq` | `1` | Sequence number for this file |
| `type` | `i` | An incremental file, meaning it contains write commands appended one by one |

For example, if the server is started with `--appendfilename appendonly.aof`, the manifest file would be named `appendonly.aof.manifest` and should contain:

```
file appendonly.aof.1.incr.aof seq 1 type i
```

You can hardcode the sequence number and type for this stage.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh --dir <dir> --appendonly yes --appenddirname <append_dir_name> --appendfilename <append_file_name>
```

It will then verify that:

- The directory `<dir>/<append_dir_name>` exists
- The empty AOF file `<dir>/<append_dir_name>/<append_file_name>.1.incr.aof` exists
- The manifest file `<dir>/<append_dir_name>/<append_file_name>.manifest` exists
- The manifest file contains the line `file <append_file_name>.1.incr.aof seq 1 type i`

#### Notes

- The manifest file should be created at startup alongside the AOF file.
- Each token in the manifest line is separated by a single space. No tabs or extra spaces.
- Make sure the manifest line ends with a newline character.

### Write a single command

<https://app.codecrafters.io/courses/redis/stages/dc8>

In this stage, you'll write a single modifying command to the append-only file.

#### Writing to the Append-Only File

So far, you've created the AOF directory, file, and manifest at startup. Now you'll start using them. When `--appendonly yes` is set and your server processes a write command (like `SET`), it should append that command to the append-only file in [RESP](https://redis.io/docs/latest/develop/reference/protocol-spec/) format.

Your server should read the manifest to find the name of the AOF file to write to. The manifest's `type i` entry tells you which file is the active incremental file.

For example, if your server receives:

```bash
$ redis-cli SET foo 100
```

It should append the following to the AOF file:

```
*3\r\n$3\r\nSET\r\n$3\r\nfoo\r\n$3\r\n100\r\n
```

This is the same RESP encoding your server already uses for communication. The command is stored exactly as it was received, so it can be replayed on restart to rebuild the state.

#### The `appendfsync always` Option

When `appendfsync` is set to `always`, the server must flush the write to disk before sending a response to the client. This guarantees that no acknowledged write can be lost, even if the server crashes immediately after responding.

#### Tests

The tester will create the following under `<dir>/<append_dir_name>`:

- A manifest whose `type i` entry names a file `<random_file_name>.1.incr.aof`
- The corresponding empty AOF file: `<random_file_name>.1.incr.aof`

The tester will execute your program like this:

```bash
$ ./your_program.sh --dir <dir> --appendonly yes --appenddirname <append_dir_name> --appendfilename <append_file_name> --appendfsync always
```

It will then send a single write command:

```bash
$ redis-cli SET <key> <value>
```

The tester will verify that:

- The command is appended to the AOF file named in the manifest (not the default `<appendfilename>.1.incr.aof`)
- The command is written in a valid RESP format
- The write is flushed to disk before the client receives a response

#### Notes

- You must read the manifest to determine which file to write to. The tester intentionally uses a non-default filename to make sure your server follows the manifest.
- You don't need to handle `--appendfsync everysec` in this stage. Only `always` is tested.
- The RESP encoding of the command is the same format you're already using for client-server communication. You're just writing it to a file instead of a socket.

### Write multiple commands

<https://app.codecrafters.io/courses/redis/stages/fi1>

In this stage, you'll extend AOF logging to handle multiple write commands.

#### Appending Multiple Commands

Each write command should be appended to the AOF file in the order it was received, so the file builds up a complete log of all modifications. This applies to any command that modifies data (like `SET`, `DEL`, `INCR`, `LPUSH`, etc.), not just `SET`.

For example, if the server receives:

```bash
$ redis-cli SET foo 100
$ redis-cli SET bar 200
```

The AOF file should contain both commands in order:

```
*3\r\n
$3\r\n
SET\r\n
$3\r\n
foo\r\n
$3\r\n
100\r\n
*3\r\n
$3\r\n
SET\r\n
$3\r\n
bar\r\n
$3\r\n
200\r\n
```

*(The `\r\n` sequences above are shown on separate lines for readability. In the actual file, each command is a continuous sequence of bytes.)*

Each command is appended immediately after the previous one with no separators between them. On replay, the RESP framing (`*3\r\n...`) is enough to tell where one command ends and the next begins.

#### Tests

The tester will create the following under `<dir>/<append_dir_name>`:

- A manifest whose `type i` entry names a file `<random_file_name>.1.incr.aof`
- The corresponding empty AOF file: `<random_file_name>.1.incr.aof`

The tester will execute your program like this:

```bash
$ ./your_program.sh --dir <dir> --appendonly yes --appenddirname <append_dir_name> --appendfilename <append_file_name> --appendfsync always
```

It will then send two write commands with different keys:

```bash
$ redis-cli SET <key1> <value1>
$ redis-cli SET <key2> <value2>
```

The tester will verify that:

- Both commands appear in the append-only file in the order they were sent
- Each command is encoded in a valid RESP format
- The writes are flushed to disk before the client receives a response (since `appendfsync` is `always`)

#### Notes

- If your implementation from earlier stages already appends (rather than overwrites), this stage may already pass without changes.
- You must read the manifest to determine which file to write to. The tester uses a non-default filename.
- You don't need to handle `--appendfsync everysec` in this stage.

### Filter write commands

<https://app.codecrafters.io/courses/redis/stages/ep6>

In this stage, you'll ensure that only write commands are logged to the append-only file.

#### Filtering Non-Modifying Commands

Commands that don't modify data, like `PING`, `GET`, and `ECHO`, should never be written to an AOF file since they don't change any state.

If the server receives a mix of read and write commands, only the write commands should appear in the AOF file in the order they were received.

For example, if a client sends:

```bash
$ redis-cli SET foo 1
$ redis-cli GET foo
$ redis-cli PING
$ redis-cli ECHO hello
$ redis-cli SET bar 2
```

The AOF file should contain only the two `SET` commands:

```
*3\r\n
$3\r\n
SET\r\n
$3\r\n
foo\r\n
$1\r\n
1\r\n
*3\r\n
$3\r\n
SET\r\n
$3\r\n
bar\r\n
$1\r\n
2\r\n
```

*(The `\r\n` sequences above are shown on separate lines for readability. In the actual file, each command is a continuous sequence of bytes.)*

No trace of `GET`, `PING`, or `ECHO` should appear in the file.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh --dir <dir> --appendonly yes --appenddirname <append_dir_name> --appendfilename <append_file_name> --appendfsync always
```

It will send a mix of modifying and non-modifying commands, including `PING`, `GET`, `ECHO`, and `CONFIG GET` interleaved with at least one write command.

The tester will verify that:

- The append-only file contains only the write commands, in order
- No bytes from non-modifying commands appear in the file
- Each logged command is in valid RESP format

#### Notes

- If you already have a way to distinguish write commands from read commands (e.g., for replication), you can reuse that logic here.
- You must read the manifest to determine which file to write to. The tester uses a non-default filename.
- You don't need to handle `--appendfsync everysec` in this stage.

### Replay a single command

<https://app.codecrafters.io/courses/redis/stages/xz2>

In this stage, you'll restore the state on startup by replaying commands from the append-only file.

#### Replaying the Append-Only File

In earlier stages, you wrote commands to the AOF file as they came in. Now you'll do the reverse: on startup, read those commands back and replay them to rebuild the database.

When the server starts with `--appendonly yes` and the append-only directory already exists, it should:

1. Read the manifest file at `<dir>/<append_dir_name>/<append_file_name>.manifest`
2. Find the entry with `type i` (the incremental file)
3. Open that file and parse the RESP-encoded commands inside it
4. Execute each command as if a client had sent it

After replaying, the database should be in the same state it was in when the commands were originally written. This is how Redis recovers data after a restart.

#### Tests

The tester will create a directory `<dir>/<append_dir_name>` containing:

- An AOF file `<random_file_name>.1.incr.aof` with a single RESP-encoded `SET` command
- A manifest file `<append_file_name>.manifest` pointing to that AOF file:

```
file <random_file_name>.1.incr.aof seq 1 type i
```

The tester will execute your program like this:

```bash
$ ./your_program.sh --dir <dir> --appendonly yes --appenddirname <append_dir_name> --appendfilename <append_file_name>
```

After startup, the tester will send a `GET` command to verify the replayed data:

```bash
$ redis-cli GET <key>
```

The tester will verify that:

- The value returned by `GET` matches the value from the `SET` command in the AOF file
- Your server read the AOF file named in the manifest, not a hardcoded filename

#### Notes

- You must read the manifest to find which file to replay. The tester intentionally uses a non-default filename (`<random_file_name>.1.incr.aof` instead of `<append_file_name>.1.incr.aof`) to make sure your server follows the manifest.
- The AOF file contains the same RESP format you already parse from client connections. You can reuse your existing RESP parser here.

### Replay multiple commands

<https://app.codecrafters.io/courses/redis/stages/kn2>

In this stage, you'll extend replay to handle append-only files with multiple commands.

#### Replaying Multiple Commands

An append-only file typically contains many commands, not just one. Your server needs to parse the file as a sequence of RESP-encoded commands and replay each one in order, just as if a client had sent them live.

Since RESP is self-framing (each command starts with `*<n>\r\n` indicating its length), you can read commands one after another until you reach the end of the file. Each command you parse should be executed against your server's state before moving on to the next.

After replaying all commands, your database should reflect the cumulative effect of every write in the file.

#### Tests

The tester will create a directory `<dir>/<append_dir_name>` containing:

- An AOF file `<random_file_name>.1.incr.aof` with multiple RESP-encoded commands (e.g., several `SET` commands for different keys)
- A manifest file `<append_file_name>.manifest` pointing to that AOF file:

```
file <random_file_name>.1.incr.aof seq 1 type i
```

The tester will execute your program like this:

```bash
$ ./your_program.sh --dir <dir> --appendonly yes --appenddirname <append_dir_name> --appendfilename <append_file_name>
```

After startup, the tester will send several `GET` commands to verify each key was restored:

```bash
$ redis-cli GET <key1>
$ redis-cli GET <key2>
...
```

The tester will verify that:

- Each `GET` returns the value from the corresponding command in the AOF file
- All commands from the AOF file were replayed, not just the first one

#### Notes

- If your implementation from earlier stages already loops through the file until EOF, this stage may already pass without changes.
- You must read and replay from the file named in the manifest, not from the hardcoded filename `<append_file_name>.1.incr.aof`.
- RESP is self-framing, so you don't need any separator between commands. Just keep parsing until you run out of bytes.

## Pub/Sub

### Subscribe to a channel

<https://app.codecrafters.io/courses/redis/stages/mx3>

In this stage, you’ll add support for the  `SUBSCRIBE` command.

#### The `SUBSCRIBE` Command

[The `SUBSCRIBE` command](https://redis.io/docs/latest/commands/subscribe/) registers the client as a subscriber to a channel.

Example Usage:

```bash
$ redis-cli
> SUBSCRIBE mychan
1) "subscribe"
2) "mychan"
3) (integer) 1
Reading messages... (press Ctrl-C to quit or any key to type command)
```

The response is a RESP Array which contains three elements:

1. "subscribe" (as a RESP bulk string)
1. The channel name (as a RESP bulk string) 
1. The number of channels this client has subscribed to so far (as a RESP integer)

#### Tests

The tester will  run your program like this:

```bash
$ ./your_program.sh
```

It will then send a `SUBSCRIBE` command with a channel name.

```
$ redis-cli SUBSCRIBE foo
```

The tester will then expect the response to be `["subscribe", "foo", 1]`, which is encoded in RESP as:

```
*3\r\n
$9\r\n
subscribe\r\n
$3\r\n
foo\r\n
:1\r\n
```

#### Notes

- In this stage, you'll only need to handle a `SUBSCRIBE` command being sent once and from a single client. We'll get to handling multiple `SUBSCRIBE` commands in later stages.

### Subscribe to multiple channels

<https://app.codecrafters.io/courses/redis/stages/zc8>

In this stage, you'll add support for subscribing to multiple channels using the `SUBSCRIBE` command.

#### Multiple SUBSCRIBE commands

A client can send `SUBSCRIBE` multiple times to subscribe to different channels. For example:

```bash
$ redis-cli
> SUBSCRIBE channel_1
1) "subscribe"
2) "channel_1"
3) (integer) 1

(subscribed mode)> SUBSCRIBE channel_2
1) "subscribe"
2) "channel_2"
3) (integer) 2
```

A client can also subscribe to the same channel multiple times, but the total subscribed channels count (returned as the 3rd item in the response array) won't change.

For example:

```bash
$ redis-cli
> SUBSCRIBE channel_1
1) "subscribe"
2) "channel_1"
3) (integer) 1

# The count remains 1 since channel is same
(subscribed mode)> SUBSCRIBE channel_1
1) "subscribe"
2) "channel_1"
3) (integer) 1
```

The subscribed channels count is per-client, i.e. how many channels the current client is subscribed to.

#### Tests

The tester will  run your program like this:

```bash
$ ./your_program.sh
```

It will then spawn a client and send multiple `SUBSCRIBE` commands specifying a new channel each time.

```bash
$ redis-cli
> SUBSCRIBE foo
# Expect ["subscribe", "foo", 1]

> SUBSCRIBE bar
# Expect ["subscribe", "bar", 2]

> SUBSCRIBE bar
# Expect ["subscribe", "bar", 2]
```

The tester will repeat this with other clients and verify that the subscribed channel counts are maintained per-client.

### Enter subscribed mode

<https://app.codecrafters.io/courses/redis/stages/aw8>

In this stage, you'll add support for marking a client as having entered Subscribed mode.

#### Subscribe Mode

After a subscribe command is sent, a client enters "Subscribed mode". In this mode, only a subset of commands is allowed:

- `SUBSCRIBE`
- `UNSUBSCRIBE`
- `PSUBSCRIBE`
- `PUNSUBSCRIBE`
- `PING`
- `QUIT`

The server will reject any other commands with an error, example:

```
$ redis-cli
> SUBSCRIBE channel
1) "subscribe"
2) "channel"
3) (integer) 1

(subscribed mode)> ECHO hey
(error) ERR Can't execute 'echo': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context
```

#### Tests

The tester will  run your program like this:

```bash
$ ./your_program.sh
```

It will then send a `SUBSCRIBE` command to your server to enter Subscribed mode:

```bash
$ redis-cli
> SUBSCRIBE foo
# Expecting ["subscribe", "foo", 1]
```

The tester will then send a series of commands, which might be allowed or un-allowed for subscribed mode.

For un-allowed commands (like `SET`, `GET`, and `ECHO`) the tester will verify that your server responds with the following error:

```
> SET key value
- ERR Can't execute 'set': only (P|S)SUBSCRIBE / (P|S)UNSUBSCRIBE / PING / QUIT / RESET are allowed in this context 
```

The tester only verifies that error message starts with "ERR Can't execute '&lt;command_name&gt;'", so you're free to use a flexible error message and not stick to the exact format that Redis uses.

For the `SUBSCRIBE` command, the tester will verify that the response is its usual response.
```bash
> SUBSCRIBE bar
# Expecting ["subscribe", "bar", 2] as RESP-encoded array
```

#### Notes

- For un-allowed commands, the tester is lenient in checking error messages so you don't have to stick to the exact format Redis uses. The exact format it checks for is `ERR Can't execute '<command>'` (case-insensitive). Examples of error message strings that will pass tests: 
    - `ERR Can't execute 'set' in subscribed mode`
    - `ERR can't execute 'SET' when one or more subscriptions exist`

- In subscribed mode, `PING` has a different response (it doesn't respond with `+PONG\r\n`). We'll get to this in later stages. 

### PING in subscribed mode

<https://app.codecrafters.io/courses/redis/stages/lf1>

In this stage, you'll add support for responding to `PING` when a client is in subscribed mode.

#### `PING` in subscribed mode

If a ping command is issued from a client after it enters subscribed mode, Redis does not respond with the usual response (`+PONG\r\n`). Instead it responds with a RESP array of two elements:

1. "pong" (encoded as a bulk string)
2. "" (empty bulk string)

#### Tests

The tester will  run your program like this:

```bash
$ ./your_program.sh
```

It will then send a `SUBSCRIBE` command to your server to enter Subscribed mode:

```bash
$ redis-cli
> SUBSCRIBE foo
# Expecting ["subscribe", "foo", 1]
```

The tester will then send a `PING` command.
```
> PING 
```

It will expect the response to be an RESP-encoded array `["pong", ""]`, which would look like this:
```
*2\r\n
$4\r\n
pong\r\n
$0\r\n
\r\n
```

The tester will also send a `PING` command using a separate client, which is not subscribed to any channels.
```
> PING
```
It will expect the response to be `+PONG\r\n`, which is "PONG" encoded as a RESP simple string.

### Publish a message

<https://app.codecrafters.io/courses/redis/stages/hf2>

In this stage, you'll add support for the `PUBLISH` command\.

#### The `PUBLISH` Command

[The PUBLISH command](https://redis.io/docs/latest/commands/publish/) delivers a message to all clients subscribed to a channel.

```
> PUBLISH channel_name message_contents
(integer) 3
```

The response to the command is the number of clients that are subscribed to the channel.

In this stage, you will only implement responding back to the `PUBLISH` command. You don't need to deliver the message to clients yet — we'll get to this in later stages.

#### Tests

The tester will  run your program like this:

```bash
$ ./your_program.sh
```

It will then spawn multiple clients that listen on multiple channels.

```bash
# Client 1 subscribes to channel "foo"
$ redis-cli
> SUBSCRIBE foo

# Client 2 subscribes to channel "bar"
$ redis-cli
> SUBSCRIBE bar

# Client 3 subscribes to channel "bar"
$ redis-cli
> SUBSCRIBE bar
```

The tester will then publish messages to different channels using `PUBLISH` and check whether the response matches the number of clients subscribed to the channel.

```
$ redis-cli PUBLISH bar "msg"
(integer) 2
```

In the above example, the expected response is 2 (encoded as a RESP integer) since 2 clients are subscribed to channel bar.

Similarly for the `foo` channel

```
$ redis-cli PUBLISH foo "msg"
(integer) 1
```

The tester expects the response to be 1 (encoded as a RESP integer) since only 1 client is subscribed to it.

### Deliver messages

<https://app.codecrafters.io/courses/redis/stages/dn4>

In this stage, you will add support for delivering published messages to subscribed clients.

#### Delivering messages

When a client runs `PUBLISH` the command, the message is "delivered" to all clients currently subscribed to the channel.

For example, if a client was subscribed to `channel_1` and the message `hello` was sent, it would see something like this:

```bash
$ redis-cli
> SUBSCRIBE channel_1
1) "subscribe"
2) "channel_1"
3) (integer) 1

# (client is now in subscribed mode)

# When a message is published, it receives the following array: 
1) "message"
2) "channel_1"
3) "hello"
```

Each subscribed client receives an array with 3 elements:

1. "message"(as a RESP bulk string)
1. The channel name (as a RESP bulk string)
1. The message contents (as a RESP bulk string)

In this case, each subscribed client will receive  `["message", "channel_1", "hello"]`, encoded as:

```
*3\r\n
$7\r\n
message\r\n
$9\r\n
channel_1\r\n
$5\r\n
hello\r\n
```

#### Tests

The tester will  run your program like this:

```bash
$ ./your_program.sh
```

It will then spawn multiple clients that listen on multiple channels.

```bash
# Client 1 subscribes to foo
$ redis-cli
> SUBSCRIBE foo

# Client 2 subscribes to foo
$ redis-cli
> SUBSCRIBE foo

# Client 3 subscribes to bar
$ redis-cli
> SUBSCRIBE bar
```

The tester will then spawn a separate client and send `PUBLISH` commands for random channels.

```bash
$ redis-cli
> PUBLISH foo "hello"
2
> PUBLISH bar "world"
1
```

It'll verify the `PUBLISH` commands returns the number of clients subscribed to the channel.

For every client, it'll also assert that the client receives the message if it is subscribed. The tester will verify that each subscribed client receives an array like this: `["message", "foo", "hello"]`. When RESP-encoded, this looks like:

```
*3\r\n
$7\r\n
message\r\n
$3\r\n
foo\r\n
$5\r\n
hello\r\n
```

If a client isn't subscribed to a channel, the tester will validate that it doesn't receive messages sent to that channel.

### Unsubscribe

<https://app.codecrafters.io/courses/redis/stages/ze9>

In this stage, you'll add support for the `UNSUBSCRIBE` command, which is used to unsubscribe from a channel.

#### The `UNSUBSCRIBE` Command

This command removes the client from one or more channels. Example usage:

```bash
$ redis-cli
> SUBSCRIBE foo
1) "subscribe"
2) "foo"
3) (integer) 1
(subscribed mode)> SUBSCRIBE bar
1) "subscribe"
2) "bar"
3) (integer) 2
(subscribed mode)> UNSUBSCRIBE foo
1) "unsubscribe"
2) "foo"
3) (integer) 1
(subscribed mode)> UNSUBSCRIBE bar
1) "unsubscribe"
2) "bar"
3) (integer) 0
```

When unsubscribed, the server replies with a RESP Array with 3 elements:

1. "unsubscribe" (as a RESP bulk string)
1. The channel name (as a RESP bulk string)
1. Count of remaining channels the client has subscribed to (as a RESP integer)

When unsubscribing from a channel that has not been subscribed yet, it returns the array with no change in the remaining channels count. For eg,

```bash
$ redis-cli
> subscribe foo
1) "subscribe"
2) "foo"
3) (integer) 1

(subscribed mode)> unsubscribe bar
1) "unsubscribe"
2) "bar"
3) (integer) 1
```

#### Tests

The tester will  run your program like this:

```bash
$ ./your_program.sh
```

It will then spawn multiple clients that listen on multiple channels:

```bash
# Client 1 subscribes to 'foo' and 'baz'
$ redis-cli
> SUBSCRIBE foo
> SUBSCRIBE baz

# Client 2 subscribes to 'foo' and 'bar'
$ redis-cli
> SUBSCRIBE foo
> SUBSCRIBE bar
```

The tester will then spawn a separate client that publishes messages to random channels.

```bash
$ redis-cli
> PUBLISH foo "before-unsubscribe"
```

It will verify that any clients subscribed to the channel receive the message, and that other clients don't.

The tester will then issue random `UNSUBSCRIBE` commands to the subscribed clients:

```bash
# In client 1, which was subscribed to 'foo' and 'baz'
> UNSUBSCRIBE foo
# Expects ["unsubscribed", "foo", 1] as an RESP array
```

Finally, the tester will again publish messages to random channels:.

```bash
> PUBLISH foo "after-unsubscribe"
```

It will verify that any clients subscribed to the channel receive the message, and that other clients don't.

## Sorted Sets

### Create a sorted set

<https://app.codecrafters.io/courses/redis/stages/ct1>

In this stage, you'll add support for creating a [sorted set](https://redis.io/docs/latest/develop/data-types/sorted-sets/) using the `ZADD` command.

#### Redis Sorted Sets

Sorted sets are one of the data types that Redis supports. A sorted set is a collection of unique elements, where each element is associated with a floating-point score. Unlike regular sets, sorted sets maintain elements in a defined order based on their scores.

This makes them useful for use cases like leaderboards, priority queues, or any scenario where you need fast access to items sorted by a numerical value.

For example, if you were using sorted sets to store user rankings in a game leaderboard, the contents of the sorted set might look like this:

```yaml
racer_scores:
    - member: "Ford"
      score: 6.1
    - member: "Royce"
      score: 8.2
    - member: "Sam-Bodden"
      score: 8.2
    - member: "Prickett"
      score: 14.5
```

Sorted sets are ordered based on increasing scores.


#### The `ZADD` Command

The [ZADD](https://redis.io/docs/latest/commands/zadd/) command is used to add a member to a sorted set.

If the sorted set does not exist, it is created and the member is added to it.

Example usage:

```bash
> ZADD racer_scores 8.0 "Sam"
(integer) 1
```

The `ZADD` command takes the key, a score, and the member name as arguments. It returns an integer representing the number of new members added to the sorted set.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send a `ZADD` command specifying a key, value and score.

```bash
$ redis-cli ZADD zset_key 10.0 zset_member
```

The tester will verify that the response to the command is `:1\r\n`, which is 1 (the number of members added to the sorted set), encoded as a RESP Integer.

#### Notes
- In this stage, you'll only need to handle creating a new sorted set with a single member. We will get to adding new members to existing sorted set in the later stages.
- It is recommended to store the score as a 64 bit floating point number for highest precision as the official Redis implementation uses [`double`](https://github.com/redis/redis/blob/bec644aab198049eaa5583631c419b4574b137e1/tests/modules/zset.c#L34).

- We suggest that you implement sorted sets using a data structure where the members are stored in a sorted fashion according to their scores. It'll come in handy in the later stages.
    -  Redis implements sorted sets using a combination of [hash table and skip list](https://github.com/redis/redis/blob/674b829981c0b8ad15a670a32df503e0e4514e96/src/server.h#L1560).

### Add members

<https://app.codecrafters.io/courses/redis/stages/hf1>

In this stage, you'll add support for adding elements to an existing sorted set.

#### Adding elements using `ZADD`

The `ZADD` command can be used to add a new member to an existing sorted set, or update the score of an existing member. It returns the count of new members added to the sorted set as an integer.


Example usage:
```bash
> ZADD zset_key 0.0043 foo
(integer) 1
> ZADD zset_key 8.0 bar
(integer) 1

# No new members were added
> ZADD zset_key 10.0 bar
(integer) 0
```

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send a `ZADD` command to create a new sorted set.

```bash
$ redis-cli ZADD zset_key 20.0 zset_member1 (Expecting ":1\r\n")
```

The tester will then send the `ZADD` command a few times to add new members.

```bash
$ redis-cli
> ZADD zset_key 30.1 zset_member2 (Expecting ":1\r\n")
> ZADD zset_key 40.2 zset_member3 (Expecting ":1\r\n")
> ZADD zset_key 50.3 zset_member4 (Expecting ":1\r\n")
```

The tester expects the response to be `:1\r\n` whenever a new member is added.

It will then update the score of an existing member.
```bash
> ZADD zset_key 100.0 zset_member1 (Expecting ":0\r\n")
```

The tester expects the response to be `:0\r\n` whenever an existing member is updated.

### Retrieve member rank

<https://app.codecrafters.io/courses/redis/stages/lg6>

In this stage, you'll add support for retrieving the rank of a sorted set member using the `ZRANK` command.

#### The `ZRANK` Command

The `ZRANK` command is used to query the rank of a member in a sorted set. It returns an integer, which is 0-based index of the member when the sorted set is ordered by increasing score.
If two members have same score, the members are ordered lexicographically.

Example usage:
```bash
> ZADD zset_key 1.0 member_with_score_1
(integer) 1
> ZADD zset_key 2.0 member_with_score_2
(integer) 1
> ZADD zset_key 2.0 another_member_with_score_2
(integer) 1


> ZRANK zset_key member_with_score_1
(integer) 0
> ZRANK zset_key member_with_score_2
(integer) 2
> ZRANK zset_key another_member_with_score_2
(integer) 1
```

The rank of `another_member_with_score_2` is 1, and `member_with_score_2` is 2. It is because though the both members have same scores, `another_member_with_score_2` preceeds `member_with_score_2` lexicographically.


If the member, or the sorted set does not exist, the command returns null bulk string (`$-1\r\n`).
```bash
# Missing sorted set and member
> ZRANK zset_key missing_member
(nil)
> ZRANK missing_key member
(nil)
```

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send a `ZADD` command to create and add new members to it.

```bash
$ redis-cli
> ZADD zset_key 100.0 foo (Expecting ":1\r\n")
> ZADD zset_key 100.0 bar (Expecting ":1\r\n")
> ZADD zset_key 20.0 baz (Expecting ":1\r\n")
> ZADD zset_key 30.1 caz (Expecting ":1\r\n")
> ZADD zset_key 40.2 paz (Expecting ":1\r\n")

# Expected Ranks
# baz -> 0
# caz -> 1
# paz -> 2
# bar -> 3
# foo -> 4
```

The tester will then send multiple `ZRANK` commands specifying the members of the sorted set.
```bash
> ZRANK zset_key caz (Expecting ":1\r\n")
> ZRANK zset_key baz (Expecting ":0\r\n")
> ZRANK zset_key foo (Expecting ":4\r\n")
> ZRANK zset_key bar (Expecting ":3\r\n")
```

The tester will also send `ZRANK` commands where either the member or key doesn't exist.

```bash
> ZRANK zset_key missing_member (Expecting RESP bulk string "$-1\r\n")
> ZRANK missing_key member (Expecting RESP bulk string "$-1\r\n")
```

### List sorted set members

<https://app.codecrafters.io/courses/redis/stages/ic1>

In this stage, you'll add support for listing the members of a sorted set using the `ZRANGE` command.

#### The `ZRANGE` command
The `ZRANGE` command is used to list the members in a sorted set given a start index and an end index. The index of the first element is 0. The end index is inclusive, which means that the element at the end index will be included in the response.

Example usage:
```bash
> ZADD racer_scores 8.1 "Sam-Bodden"
(integer) 1
> ZADD racer_scores 10.2 "Royce"
(integer) 1
> ZADD racer_scores 6.0 "Ford"
(integer) 1
> ZADD racer_scores 14.1 "Prickett"
(integer) 1

# List members from index 0 to 2
> ZRANGE racer_scores 0 2
1) "Ford"
2) "Sam-Bodden"
3) "Royce"
```

Here are some additional notes on how the `ZRANGE` command behaves with different types of inputs:

- If the sorted set does not exist, an empty array (`*0\r\n`) is returned
- If the start index is greater than or equal to the cardinality of the sorted set, an empty array is returned.
- If the stop index is greater than the cardinality of the sorted set, the stop index is treated as the last element.
- If the start index is greater than the stop index, the result is an empty array.


#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then create a new sorted set with multiple members.

```bash
$ redis-cli
> ZADD zset_key 100.0 foo (Expecting ":1\r\n")
> ZADD zset_key 100.0 bar (Expecting ":1\r\n")
> ZADD zset_key 20.0 baz (Expecting ":1\r\n")
> ZADD zset_key 30.1 caz (Expecting ":1\r\n")
> ZADD zset_key 40.2 paz (Expecting ":1\r\n")
```

After that the tester will send your program a series of `ZRANGE` commands. It will expect the response to be a RESP array, or an empty array in each case, depending on the test case.

As an example, the tester might send your program a command like this.
```bash
> ZRANGE zset_key 2 4
# Expect RESP Encoded Array: ["paz", "bar", "foo"]
```

It will expect the response to be an RESP-encoded array `["paz", "bar", "foo"]`, which would look like this:
```
*3\r\n
$3\r\n
paz\r\n
$3\r\n
bar\r\n
$3\r\n
foo\r\n
```

The tester will issue multiple such commands and verify their responses.

#### Notes

- In this stage, you will only implement `ZRANGE` with non-negative indexes. We will get to handling `ZRANGE` with negative indexes in later stages.

### ZRANGE with negative indexes

<https://app.codecrafters.io/courses/redis/stages/bj4>

In this stage, you'll add support for negative indexes for the `ZRANGE` command.

#### `ZRANGE` with negative indexes

The `ZRANGE` command can accept negative indexes too.

Example usage:

```bash
> ZADD racer_scores 8.5 "Sam-Bodden"
(integer) 1
> ZADD racer_scores 10.2 "Royce"
(integer) 1
> ZADD racer_scores 6.1 "Ford"
(integer) 1
> ZADD racer_scores 14.9 "Prickett"
(integer) 1
> ZADD racer_scores 10.2 "Ben"
(integer) 1


# List last 2 elements
> ZRANGE racer_scores -2 -1
1) "Royce"
2) "Prickett"

# List all items except last 2
> ZRANGE racer_scores 0 -3
1) "Ford"
2) "Sam-Bodden"
3) "Ben"
```

An index of -1 refers to the last element, -2 to the second last, and so on. If a absolute value of the negative index is out of range (i.e. >= the cardinality of the sorted set), it is treated as 0 (start of the sorted set).

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send a `ZADD` command to create a sorted set and add members to it.

```bash
$ redis-cli
> ZADD zset_key 20.0 foo (Expecting ":1\r\n")
> ZADD zset_key 30.1 bar (Expecting ":1\r\n")
> ZADD zset_key 40.2 baz (Expecting ":1\r\n")
> ZADD zset_key 25.0 paz (Expecting ":1\r\n")
> ZADD zset_key 25.0 caz (Expecting ":1\r\n")
```

The tester will then send your program a series of `ZRANGE` commands with one or more negative indexes.

For example, the tester might send you this command:

```bash
> ZRANGE zset_key 2 -1
1) "paz"
2) "bar"
3) "baz"
```

In this case, the tester will verify that the response is the array `["paz", "bar", "baz"]`, which is RESP Encoded as:

```
*3\r\n
$3\r\n
paz\r\n
$3\r\n
bar\r\n
$3\r\n
baz\r\n
```

### Count sorted set members

<https://app.codecrafters.io/courses/redis/stages/kn4>

In this stage, you'll add support for counting the number of members in a sorted set using the `ZCARD` command.

#### The `ZCARD` Command

The `ZCARD` command is used to query the cardinality (number of elements) of a sorted set. It returns an integer. The response is 0 if the sorted set specified does not exist.

```bash
> ZADD zset_key 1.2 "one"
(integer) 1
> ZADD zset_key 2.2 "two"
(integer) 1
> ZCARD zset_key
(integer) 2

> ZCARD missing_key
(integer) 0
```

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send a `ZADD` command to create and add members to it.

```bash
$ redis-cli
> ZADD zset_key 20.0 zset_member1 (Expecting ":1\r\n")
> ZADD zset_key 30.1 zset_member2 (Expecting ":1\r\n")
> ZADD zset_key 40.2 zset_member3 (Expecting ":1\r\n")
> ZADD zset_key 50.3 zset_member4 (Expecting ":1\r\n")
```

It will check the number of elements in the sorted set using the `ZCARD` command.
```bash
$ redis-cli ZCARD zset_key (Expecting ":4\r\n")
```

The tester will then update the score of an existing member.
```bash
$ redis-cli ZADD zset_key 100.0 zset_member1 (Expecting ":0\r\n")
```

It will again check the cardinality of the sorted set using the `ZCARD` command.
```bash
$ redis-cli ZCARD zset_key (Expecting ":4\r\n")
```

The tester will also check the cardinality of a non existing sorted set.
```bash
$ redis-cli ZCARD missing_key (Expecting ":0\r\n")
```

### Retrieve member score

<https://app.codecrafters.io/courses/redis/stages/gd7>

In this stage, you'll add support for retrieving the score of a sorted set member using the `ZSCORE` command.

#### The `ZSCORE` Command

The `ZSCORE` command is used to query the score of a member of a sorted set. If the sorted set and the member both exist, the score of the member is returned as a RESP bulk string.
```bash
> ZADD zset_key 24.34 "one"
(integer) 1
> ZADD zset_key 90.34 "two"
(integer) 1
> ZSCORE zset_key "one"
"24.34"
```

If the member or the sorted set specified in the argument does not exist, RESP null bulk string(`$-1\r\n`) is returned.
```bash
> ZSCORE zset_key "three"
(nil)
> ZSCORE missing_key "member"
(nil)
```


#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send a `ZADD` command to create a sorted set and add new members to it.

```bash
$ redis-cli
> ZADD zset_key 20.0 zset_member1 (Expecting ":1\r\n")
> ZADD zset_key 30.1 zset_member2 (Expecting ":1\r\n")
> ZADD zset_key 40.2 zset_member3 (Expecting ":1\r\n")
> ZADD zset_key 50.3 zset_member4 (Expecting ":1\r\n")
```

The tester will then send a `ZSCORE` command specifying one of the members. For example, the tester may send your program a command like this:

```bash
> ZSCORE zset_key zset_member2 (Expecting RESP bulk string "30.1")
```

It will expect the response to be "30.1", which is the score of `zset_member2`. The response is encoded as a RESP bulk string:
```
$4\r\n
30.1\r\n
```

The tester will then update the value of the member.
```bash
> ZADD zset_key 100.99 zset_member2 (Expecting ":0\r\n")
```

The tester will then send a `ZSCORE` command specifying the updated member.

```bash
> ZSCORE zset_key zset_member2 (Expecting RESP bulk string "100.99")
```

The tester will also send `ZSCORE` commands where either the member or the key doesn't exist.
```bash
> ZSCORE zset_key zset_member100 (Expecting RESP bulk string "$-1\r\n")
> ZSCORE missing_key member (Expecting RESP bulk string "$-1\r\n")
```

### Remove a member

<https://app.codecrafters.io/courses/redis/stages/sq7>

In this stage, you'll add support for removing a member of a sorted set using the `ZREM` command.

#### The `ZREM` command

The [`ZREM`](https://redis.io/docs/latest/commands/zrem/) command is used to remove a member from a sorted set given the member's name.

Example Usage:

```bash
> ZADD racer_scores 8.3 "Sam-Bodden"
(integer) 1
> ZADD racer_scores 10.5 "Royce"
(integer) 1

# Remove "Royce" from the sorted set
> ZREM racer_scores "Royce"
(integer) 1

# List the remaining members
> ZRANGE racer_scores 0 -1
1) "Sam-Bodden"

# Remove a non-existing member
> ZREM racer_scores "missing_member"
(integer) 0
```

It returns the number of members removed from the sorted set. If the specified member does not exist, 0 is returned.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then create a new sorted set with multiple members.

```bash
$ redis-cli
> ZADD zset_key 80.5 foo (Expecting ":1\r\n")
> ZADD zset_key 50.3 baz (Expecting ":1\r\n")
> ZADD zset_key 80.5 bar (Expecting ":1\r\n")
```

After that the tester will send your program a `ZREM` command specifying the member to be removed.

As an example, the tester might send your program a command like this.
```bash
> ZREM zset_key "baz"
# Expected: (integer) 1
```

It will then send your program a `ZRANGE` command and check for the remaining members.

```bash
> ZRANGE zset_key 0 -1
# Expected RESP Encoded Array: ["bar", "foo"]
```

The tester will also send a `ZREM` command where the member doesn't exist.
```bash
> ZREM zset_key "missing_member"
# Expected: (integer) 0
```

## Bitmaps

### Create a bitmap

<https://app.codecrafters.io/courses/redis/stages/bq9>

In this stage, you'll add support for creating a [bitmap](https://redis.io/docs/latest/develop/data-types/bitmaps/) using the `SETBIT` command.

#### Redis Bitmaps

Bitmaps let you store individual bits (`0` or `1`). Unlike lists or sorted sets, they are not a separate Redis data type. They are strings that you can access and update at the bit level.

Because each value takes only one bit, bitmaps are extremely space-efficient. This makes them useful for use cases like tracking subscriptions, counting daily active users, or any scenario where you need a yes/no flag for a large population.

For example, if you were using a bitmap to track newsletter subscriptions, the contents might look like this:

```
newsletter_subscribers: 10001010
```

Here, user numbers `0`, `4`, and `6` have subscribed. Redis numbers bits from left to right, starting at offset `0`.

#### The `SETBIT` Command

The [SETBIT](https://redis.io/docs/latest/commands/setbit/) command is used to set a bit to `0` or `1` in the string value stored at `key`.

When `key` does not exist, a new string is created and the bit is set.

Example usage:

```bash
> SETBIT bitmap_key 3 1
(integer) 0
```

The `SETBIT` command takes the key, an offset, and a value (`0` or `1`) as arguments. It returns the original bit at that offset. On a new key, that is always `0`.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send a `SETBIT` command specifying a key, offset, and value.

```bash
$ redis-cli SETBIT bitmap_key 3 1
```

The tester will verify that the response to the command is `:0\r\n`, which is 0 (the original bit at that offset), encoded as a [RESP integer](https://redis.io/docs/latest/develop/reference/protocol-spec/#integers). 

#### Notes

- As bitmaps are strings, we suggest that you store them using the same data structure you already use for `SET` and `GET`. It'll come in handy in later stages.
- Redis treats offset `0` as the most significant bit of the first byte. For example, `SETBIT key 1 1` stores the byte `01000000`. This will matter when you implement `GETBIT` and `BITCOUNT` in later stages.

### Retrieve a bit

<https://app.codecrafters.io/courses/redis/stages/qj1>

In this stage, you'll add support for retrieving a bit from a bitmap using the `GETBIT` command.

#### The `GETBIT` Command

The [GETBIT](https://redis.io/docs/latest/commands/getbit/) command returns the bit stored at a given `offset` in a bitmap.

If the key does not exist, Redis treats the bitmap as all zeros and returns `0` for any offset.

Example usage:

```bash
> SETBIT bit_key 2 1
(integer) 0

> GETBIT bit_key 2
(integer) 1
> GETBIT bit_key 3
(integer) 0
> GETBIT new_key 2
(integer) 0
```

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send a `SETBIT` command to create a bitmap.

```bash
$ redis-cli SETBIT bit_key 2 1
```

The tester will then send a `GETBIT` command for a bit that was set.

```bash
$ redis-cli GETBIT bit_key 2
```

The tester will expect the response to be `:1\r\n`, which is 1 encoded as a [RESP integer](https://redis.io/docs/latest/develop/reference/protocol-spec/#integers).

It will also send a `GETBIT` command for an unset offset on the same key.

```bash
$ redis-cli GETBIT bit_key 3
```

The tester will expect the response to be `:0\r\n`.

The tester will also send a `GETBIT` command for a key that does not exist.

```bash
$ redis-cli GETBIT new_key 2
```

In this case, your program should also respond with `:0\r\n`.

#### Notes

- Using `GETBIT` with an offset beyond the current string length should return `0`.

### Read a string as bits

<https://app.codecrafters.io/courses/redis/stages/yj2>

In this stage, you'll add support for using `GETBIT` on a key created with `SET`.

#### Strings and bitmaps

As bitmaps are strings, bitmap commands work on any string value, including keys created with `SET`.

These two views operate on the same bytes. `SET` writes the string. `GETBIT` reads one bit of it.

For example, the ASCII character `A` is the byte `01000001`. Offset `0` is the most significant bit of that byte:

```bash
> SET mykey "A"
OK
> GETBIT mykey 1
(integer) 1
> GETBIT mykey 7
(integer) 1
> GETBIT mykey 0
(integer) 0
```

Here, bits `1` and `7` are set and bit `0` is not.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send a `SET` command to store a one-byte string. For example:

```bash
$ redis-cli SET mykey "A"
```

The tester will then send `GETBIT` commands for offsets in that string.

```bash
$ redis-cli GETBIT mykey 1
```

The tester will expect the response to be `:1\r\n`, which is 1 encoded as a [RESP integer](https://redis.io/docs/latest/develop/reference/protocol-spec/#integers).

```bash
$ redis-cli GETBIT mykey 7
```

The tester will expect the response to be `:1\r\n`.

```bash
$ redis-cli GETBIT mykey 0
```

The tester will expect the response to be `:0\r\n`.

#### Notes

- In this stage, you'll only need to handle `GETBIT` on a key created with `SET`. Reading a bitmap back with `GET` is covered in the later stages.
- Redis applies `GETBIT` to the string's bytes. Keys created with `SET` and `SETBIT` share that representation. See [the official implementation](https://github.com/redis/redis/blob/e1d7d50f9c244ce52f724b279fcb19773fffa98c/src/bitops.c#L915-L917).

### Read bits as a string

<https://app.codecrafters.io/courses/redis/stages/pk5>

In this stage, you'll add support for using `GET` on a key created with `SETBIT`.

#### Strings and bitmaps

As bitmaps are strings, string commands work on keys created with `SETBIT`.

These two views operate on the same bytes. `SETBIT` writes bits into the string. `GET` returns that string.

Offset `0` is the most significant bit of the first byte. Setting bit `1` produces the byte `01000000`, which is the ASCII character `@`:

```bash
> SETBIT mykey 1 1
(integer) 0
> GET mykey
"@"
```

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send a `SETBIT` command to create a bitmap.

```bash
$ redis-cli SETBIT mykey 1 1
```

The tester will then send a `GET` command for that key.

```bash
$ redis-cli GET mykey
```

The tester will expect the response to be `$1\r\n@\r\n`, which is `@` encoded as a [RESP bulk string](https://redis.io/docs/latest/develop/reference/protocol-spec/#bulk-strings).

#### Notes

- In this stage, you'll only need to handle `GET` on a key created with a single `SETBIT`. Growing the string with a larger offset is covered in the later stages. 
- `GET` returns the string value stored at the key. `SETBIT` writes that value as bytes. Keys created with `SETBIT` and `SET` share that representation.

### Grow a bitmap

<https://app.codecrafters.io/courses/redis/stages/yf6>

In this stage, you'll add support for growing a bitmap with `SETBIT`, and for checking its length with `STRLEN`.

#### Growing a bitmap

When `SETBIT` uses an offset past the end of the string, Redis grows the string so that the offset fits. The string is padded with zeros, and the target bit is set.

[STRLEN](https://redis.io/docs/latest/commands/strlen/) returns the length of that string in bytes, not bits. Offsets `0` through `7` fit in one byte. Offset `8` is the first bit of a second byte. After `SETBIT` grows a key, `STRLEN` is `floor(offset / 8) + 1`. For offset `10`, that is `2`.

```bash
> SETBIT bitmap_key 1 1
(integer) 0
```

After the `SETBIT` command runs, Redis creates a one-byte string `01000000`.

```bash
> SETBIT bitmap_key 10 1
(integer) 0
```

After the second `SETBIT` command runs, Redis appends a second byte and then sets offset `10` to `1`. The final string becomes `01000000 00100000`.

```bash
> STRLEN bitmap_key
(integer) 2
```



#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send a `SETBIT` command to create a one-byte bitmap.

```bash
$ redis-cli SETBIT bitmap_key 1 1
```

The tester will send a `STRLEN` command for that key.

```bash
$ redis-cli STRLEN bitmap_key
```

The tester will expect the response to be `:1\r\n`, which is 1 encoded as a [RESP integer](https://redis.io/docs/latest/develop/reference/protocol-spec/#integers).

It will then send a `SETBIT` command with an offset in a second byte.

```bash
$ redis-cli SETBIT bitmap_key 10 1
```

The tester will expect the response to be `:0\r\n`.

The tester will then send `GETBIT` commands for the new bit

```bash
$ redis-cli GETBIT bitmap_key 10
```

The tester will expect the response to be `:1\r\n`.

```bash
$ redis-cli STRLEN bitmap_key
```

The tester will expect the response to be `:2\r\n`.

#### Notes

- In this stage, you'll only need to handle creating or growing a key by a small offset (creating the key if it does not exist). You won't need to handle out-of-range offsets.
- `STRLEN` returns the string's length in bytes. If the key does not exist, `STRLEN` returns `0`.

### Count set bits

<https://app.codecrafters.io/courses/redis/stages/nx3>

In this stage, you'll add support for counting set bits in a bitmap using the `BITCOUNT` command.

#### The `BITCOUNT` Command

The [BITCOUNT](https://redis.io/docs/latest/commands/bitcount/) command returns the number of bits set to `1` in a bitmap.

With no extra arguments, it counts the whole string. It can also take a `start` and `end` index. These indexes are **bytes**, not bits. Both are inclusive.

Offset `0` through `7` is byte `0`. Offset `8` through `15` is byte `1`. `BITCOUNT key 0 0` counts only the first byte.

```bash
> SETBIT bitmap_key 1 1
(integer) 0
> SETBIT bitmap_key 10 1
(integer) 0
```

After those `SETBIT` commands, the string is `01000000 00100000`. Byte `0` has one set bit. Byte `1` has one set bit.

```bash
> BITCOUNT bitmap_key
(integer) 2
> BITCOUNT bitmap_key 0 1
(integer) 2
```

Calling `BITCOUNT` with no arguments counts the `1`s in every byte. It is the same as calling `BITCOUNT` with `0 1` in this example.

```bash
> BITCOUNT bitmap_key 0 0
(integer) 1
> BITCOUNT bitmap_key 1 1
(integer) 1
```

Calling `BITCOUNT` with `0 0` counts the `1`s in the first byte. Calling it with `1 1` counts the `1`s in the second byte.

Here are some additional notes on how the `BITCOUNT` command behaves with different types of inputs:

- If the key does not exist, `BITCOUNT` returns `0`.
- If `start` is past the end of the string, `BITCOUNT` returns `0`.
- If `end` is past the end of the string, Redis treats it as the last byte.
- If `start` is greater than `end`, `BITCOUNT` returns `0`.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send `SETBIT` commands to create a two-byte bitmap.

```bash
$ redis-cli SETBIT bitmap_key 1 1
$ redis-cli SETBIT bitmap_key 10 1
```

After that, the tester will send your program a series of `BITCOUNT` commands with no extra arguments, or with non-negative indexes. For each command, it will expect the number of set bits, encoded as a [RESP integer](https://redis.io/docs/latest/develop/reference/protocol-spec/#integers).

As an example, the tester might send your program a command like this:

```bash
$ redis-cli BITCOUNT bitmap_key 0 0
```

The tester will expect the response to be `:1\r\n`.

#### Notes

- In this stage, you'll only need to handle `BITCOUNT` with no range, and with non-negative indexes. We won't deal with negative indexes in this challenge.
- `BITCOUNT` supports the `BYTE` and `BIT` [optional arguments](https://redis.io/docs/latest/commands/bitcount/#optional-arguments), but we won't deal with them in this challenge.

### AND two bitmaps

<https://app.codecrafters.io/courses/redis/stages/hv4>

In this stage, you'll add support for combining bitmaps with `BITOP AND`.

#### The `BITOP` Command

The [BITOP](https://redis.io/docs/latest/commands/bitop/) command runs a bitwise operation on one or more bitmaps and stores the result at a destination key.

`BITOP AND` sets a bit in the destination only when that bit is set in every source bitmap.

In this example, we have two bitmaps: `key1` is `10001000`  and `key2` is `10000010` , created using `SETBIT` commands.

```bash
> BITOP AND dest key1 key2
(integer) 1
> GETBIT dest 0
(integer) 1
> GETBIT dest 4
(integer) 0
> GETBIT dest 6
(integer) 0
```

After performing a `BITOP AND` operation on `key1` and `key2` , we get another bitmap `dest` with the value `10000000` . Only bit `0` is set in both.

`BITOP` returns the length of the destination string in bytes, encoded as a [RESP integer](https://redis.io/docs/latest/develop/reference/protocol-spec/#integers). Here the destination string is 1 byte, so the return value is `1`.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send `SETBIT` commands to create two bitmaps of the same length.

```bash
$ redis-cli SETBIT key1 0 1
$ redis-cli SETBIT key1 4 1
$ redis-cli SETBIT key2 0 1
$ redis-cli SETBIT key2 6 1
```

The tester will send a `BITOP AND` command that stores the result at a new key.

```bash
$ redis-cli BITOP AND dest key1 key2
```

The tester will expect the response to be `:1\r\n`.

It will then send `GETBIT` commands for the destination key.

```bash
$ redis-cli GETBIT dest 0
```

The tester will expect the response to be `:1\r\n`.

```bash
$ redis-cli GETBIT dest 4
```

The tester will expect the response to be `:0\r\n`.

#### Notes

- In this stage, you'll only need to handle `BITOP AND` with two source keys of the same length. Combining bitmaps of different lengths is covered in later stages.
- `BITOP` also supports other [operations](https://redis.io/docs/latest/commands/bitop/) like `OR`, `XOR`, and `NOT`. The `BITOP OR` operation is covered in later stages. We won't deal with `XOR` and `NOT` in this challenge.

### AND bitmaps of different lengths

<https://app.codecrafters.io/courses/redis/stages/dk2>

In this stage, you'll add support for `BITOP AND` when the source bitmaps have different lengths.

#### `BITOP AND` with different lengths

When the source strings have different lengths, Redis treats the shorter ones as if they were padded with zeros up to the length of the longest string. The destination is as long as that longest string.

```bash
> SETBIT key1 1 1
(integer) 0
> SETBIT key1 10 1
(integer) 0
> SETBIT key2 1 1
(integer) 0
```

In this example, we have two bitmaps: `key1` is `01000000 00100000` (2 bytes) and `key2` is `01000000` (1 byte), created using `SETBIT` commands.

```bash
> BITOP AND dest key1 key2
(integer) 2
> GETBIT dest 1
(integer) 1
> GETBIT dest 10
(integer) 0
```

During the `BITOP AND` operation, Redis treats the shorter string as zero-padded. In this example, `key2` is treated as `01000000 00000000` (second byte is padded with `0`s). 

Only bit `1` is set in both keys. So `dest` will hold the value `01000000 00000000`. `BITOP` returns `2`, the length of the destination.

Additional notes:

- A missing key is treated as a string of zeros of the same length as the longest source.



#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send `SETBIT` commands to create two bitmaps of different lengths.

```bash
$ redis-cli SETBIT key1 1 1
$ redis-cli SETBIT key1 10 1
$ redis-cli SETBIT key2 1 1
```

The tester will send a `BITOP AND` command that stores the result at a new key.

```bash
$ redis-cli BITOP AND dest key1 key2
```

The tester will expect the response to be `:2\r\n`, which is 2 encoded as a [RESP integer](https://redis.io/docs/latest/develop/reference/protocol-spec/#integers).

It will then send `GETBIT` commands for the destination key.

```bash
$ redis-cli GETBIT dest 1
```

The tester will expect the response to be `:1\r\n`.

```bash
$ redis-cli GETBIT dest 10
```

The tester will expect the response to be `:0\r\n`.

#### Notes

- `BITOP` also supports other [operations](https://redis.io/docs/latest/commands/bitop/) like `OR`, `XOR`, and `NOT`. The `BITOP OR` operation is covered in later stages. We won't deal with `XOR` and `NOT` in this challenge.

### OR two bitmaps

<https://app.codecrafters.io/courses/redis/stages/fr8>

In this stage, you'll add support for combining bitmaps with `BITOP OR`.

#### The `BITOP OR` Command

`BITOP OR` sets a bit in the destination when that bit is set in at least one source bitmap.

In this example, we have two bitmaps: `key1` is `10001000` and `key2` is `10000010`, created using `SETBIT` commands.

```bash
> BITOP OR dest key1 key2
(integer) 1
> GETBIT dest 0
(integer) 1
> GETBIT dest 4
(integer) 1
> GETBIT dest 6
(integer) 1
```

After performing a `BITOP OR` operation on `key1` and `key2`, we get another bitmap `dest` with the value `10001010` because bits `0`, `4`, and `6` are set in at least one source.

`BITOP` returns the length of the destination string in bytes. Here that is `1`.

When the source strings have different lengths, Redis treats the shorter ones as if they were padded with zeros up to the length of the longest string.

In this example, we have two bitmaps: `key1` is `01000000 00100000` (2 bytes) and `key2` is `01000000` (1 byte).

```bash
> BITOP OR dest key1 key2
(integer) 2
> GETBIT dest 1
(integer) 1
> GETBIT dest 10
(integer) 1
```

`key2` is treated as `01000000 00000000`. Bit `10` is set in `key1` and padded to `0` in `key2`, so it is set in the destination. `dest` holds `01000000 00100000`. `BITOP` returns `2`, the length of the destination.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send `SETBIT` commands to create two bitmaps of the same length.

```bash
$ redis-cli SETBIT key1 0 1
$ redis-cli SETBIT key1 4 1
$ redis-cli SETBIT key2 0 1
$ redis-cli SETBIT key2 6 1
```

The tester will send a `BITOP OR` command that stores the result at a new key.

```bash
$ redis-cli BITOP OR dest key1 key2
```

The tester will expect the response to be `:1\r\n`.

It will then send a `GETBIT` command for a bit that is set in only one source.

```bash
$ redis-cli GETBIT dest 4
```

The tester will expect the response to be `:1\r\n`.

The tester will then send `SETBIT` commands to create two bitmaps of different lengths.

```bash
$ redis-cli SETBIT key3 1 1
$ redis-cli SETBIT key3 10 1
$ redis-cli SETBIT key4 1 1
```

The tester will send a `BITOP OR` command that stores the result at a new key.

```bash
$ redis-cli BITOP OR dest2 key3 key4
```

The tester will expect the response to be `:2\r\n`, which is 2 encoded as a [RESP integer](https://redis.io/docs/latest/develop/reference/protocol-spec/#integers).

It will then send a `GETBIT` command for a bit that is past the end of the shorter source.

```bash
$ redis-cli GETBIT dest2 10
```

The tester will expect the response to be `:1\r\n`.

#### Notes

- `BITOP` also supports other [operations](https://redis.io/docs/latest/commands/bitop/) like `XOR` and `NOT`, but we won't deal with them in this challenge.

## Geospatial Commands

### Respond to GEOADD

<https://app.codecrafters.io/courses/redis/stages/zt4>

In this stage, you'll add support for responding to the `GEOADD` command.

#### Extension prerequisites

This stage depends on the [**Sorted Sets**](https://redis.io/docs/latest/data-types/sorted-sets/) extension. Before attempting this extension, please make sure you've completed the Sorted Sets extension.

#### The `GEOADD` command

The [`GEOADD` command](https://redis.io/docs/latest/commands/geoadd/) adds a location (with longitude, latitude, and name) to a key.

Example usage:

```bash
> GEOADD places -0.0884948 51.506479 "London"
(integer) 1
```

The arguments `GEOADD` accepts are:

1. `key`: The key to store the location in.
2. `longitude`: The longitude of the location.
3. `latitude`: The latitude of the location.
4. `member`: The name of the location.

It returns the count of elements added, encoded as a RESP Integer.

In this stage, you'll only implement the response to the `GEOADD` command. We'll get to validating arguments and storing locations in later stages.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send a `GEOADD` command:

```bash
$ redis-cli GEOADD places 11.5030378 48.164271 Munich
```

The tester will expect the response to be `:1\r\n`, which is 1 (number of locations added) encoded as a RESP integer.

#### Notes

- In this stage, you only need to implement responding to the `GEOADD` command. We'll get to validating arguments and storing locations in later stages.

### Validate coordinates

<https://app.codecrafters.io/courses/redis/stages/ck3>

In this stage, you'll add support for validating the latitude and longitude values provided in a `GEOADD` command.

#### Validating latitude and longitude values

The latitude and longitude values used in the `GEOADD` command should be in a certain range as per [EPSG:3857](https://epsg.io/3857).

- Valid longitudes are from -180° to +180°
  - Both these limits are inclusive, so -180° and +180° are both valid.
- Valid latitudes are from -85.05112878° to +85.05112878°
  - Both of these limits are inclusive, so -85.05112878° and +85.05112878° are both valid.
  - The reason these limits are not -/+90° is because of the [Web Mercator projection](https://en.wikipedia.org/wiki/Web_Mercator_projection) that Redis uses.

If either of these values aren't within the appropriate range, `GEOADD` returns an error. Examples:

```bash
# Invalid latitude
> GEOADD places 180 90 test1
(error) ERR invalid longitude,latitude pair 180.000000,90.000000

# Invalid longitude
> GEOADD places 181 0.3 test2
(error) ERR invalid longitude,latitude pair 181.000000,0.300000
```

In this stage, you'll implement validating latitude and longitude values and returning error messages as shown above. The tester is lenient with error message formats, so you don't have to use the exact error message format mentioned above.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send multiple `GEOADD` commands.

If the coordinates are invalid, it will expect an error response. For example:

```bash
# Expecting error
$ redis-cli GEOADD location_key 200 100 foo
(error) ERR invalid longitude,latitude pair 200,100
```

The returned value must:

- Be a [RESP simple error](https://redis.io/docs/latest/develop/reference/protocol-spec/#simple-errors) i.e. start with `-` and end with `\r\n`
- The error message must start with `ERR`, like standard Redis error messages
- The error message must contain the word "latitude" if the latitude is invalid
- The error message must contain the word "longitude" if the longitude is invalid

For example, if the latitude is invalid, valid error messages that the tester will accept are:

```bash
-ERR invalid latitude argument\r\n
-ERR invalid latitude value\r\n
-ERR latitude value (200.0) is invalid\r\n
```

#### Notes

- You don't need to implement storing locations yet, we'll get to that in later stages.
- The boundary of latitude are clipped at -/+85.05112878° instead of -/+90°. This is because of the [Web Mercator projection](https://en.wikipedia.org/wiki/Web_Mercator_projection) that Redis uses.

### Store a location

<https://app.codecrafters.io/courses/redis/stages/tn5>

In this stage, you'll add support for storing locations in a sorted set.

#### Storing locations in sorted set

The locations added using the `GEOADD` command are stored in a sorted set. Redis internally calculates a score for the specified location using a location's latitude and longitude.

For example, the following two commands are equivalent in Redis.

```bash
# Adding a location
$ redis-cli GEOADD places_key 2.2944692 48.8584625 location

# This command is equivalent to the command above
$ redis-cli ZADD places_key 3663832614298053 location
```

In this stage, you'll implement adding locations to a sorted set when a `GEOADD` command is run.

You can hardcode the score to be 0 for now, we'll get to calculating the score in later stages.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send a `GEOADD` command:

```bash
$ redis-cli GEOADD places 2.2944692 48.8584625 Paris
# Expect: (integer) 1
```

The tester will then send a `ZRANGE` command to validate that the location was added to the sorted set:

```bash
$ redis-cli ZRANGE places 0 -1
# Expect RESP Array: ["Paris"]
```

#### Notes

- In this stage, you can hardcode the score of the location to be 0. We'll get to calculating the value of score using latitude and longitude in later stages.
- The implementation of the `ZRANGE` command is covered in the sorted sets extension.

### Calculate location score

<https://app.codecrafters.io/courses/redis/stages/cr3>

In this stage, you'll add support for calculating the score of a location.

#### Location scores

To store locations in a sorted set, Redis converts latitude and longitude values to a single "score".

We've created a [GitHub repository](https://github.com/codecrafters-io/redis-geocoding-algorithm) that explains how this conversion is done. It includes:

- A description of the algorithm used, along with pseudocode
- Code samples in multiple languages.
- A set of locations & scores to test against

Here's the [repository link](https://github.com/codecrafters-io/redis-geocoding-algorithm).

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send multiple `GEOADD` commands:

```bash
$ redis-cli GEOADD places 2.2944692 48.8584625 Paris
# Expect: (integer) 1
$ redis-cli GEOADD places -0.127758 51.507351 London
# Expect: (integer) 1
```

The tester will validate that scores are calculated correctly by sending multiple `ZSCORE` commands:

```bash
$ redis-cli ZSCORE places Paris
# Expecting bulk string: "3663832614298053"
```

The calculated scores must match the expected values as described in [this repository](https://github.com/codecrafters-io/redis-geocoding-algorithm).

### Respond to GEOPOS

<https://app.codecrafters.io/courses/redis/stages/xg4>

In this stage, you'll add support for responding to the `GEOPOS` command.

#### The `GEOPOS` command

The `GEOPOS` command returns the longitude and latitude of the specified location.

Example usage:

```bash
> GEOADD places -0.0884948 51.506479 "London"
> GEOADD places 11.5030378 48.164271 "Munich"

> GEOPOS places London
1) 1) "-0.08849412202835083"
   2) "51.50647814139934"

> GEOPOS places Munich
1) 1) "11.503036916255951"
   2) "48.16427086232978"
```

It returns an array with one entry for each location requested.

- If a location exists under the key, its entry is an array of two items:
  - Longitude (Encoded as a [RESP Bulk string](https://redis.io/docs/latest/develop/reference/protocol-spec/#bulk-strings))
  - Latitude (Encoded as a [RESP Bulk string](https://redis.io/docs/latest/develop/reference/protocol-spec/#bulk-strings))
- If either the location or key don’t exist, the corresponding entry is a [null array](https://redis.io/docs/latest/develop/reference/protocol-spec/#null-arrays) `(*-1\r\n)`.

To return the latitude and longitude values, Redis decodes the "score" back to latitude and longitude values. We'll cover this process in later stages, for now you can hardcode the returned latitude and longitude values to be 0 (or any number).

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then add multiple locations using the `GEOADD` command.

```bash
$ redis-cli
> GEOADD location_key -0.0884948 51.506479 "London"
# Expect: (integer) 1
> GEOADD location_key 11.5030378 48.164271 "Munich"
# Expect: (integer) 1
```

The tester will then send multiple `GEOPOS` commands:

```bash
> GEOPOS location_key London Munich
# Expecting: [["0", "0"], ["0", "0"]], encoded as "*2\r\n*2\r\n$1\r\n0\r\n$1\r\n0\r\n*2\r\n$1\r\n0\r\n$1\r\n0\r\n"

> GEOPOS location_key missing_location
# Expecting: [nil], encoded as "*1\r\n*-1\r\n"
```

The tester will assert that:

- The response is a RESP array that contains as many elements as the number of locations requested
- For each location requested:
  - If the location exists:
    - The corresponding element is a RESP array with two elements (i.e. longitude and latitude)
    - Both elements are "0" (or any other valid floating point number), encoded as a RESP bulk string
  - If the location doesn't exist:
    - The corresponding element is a [null array](https://redis.io/docs/latest/develop/reference/protocol-spec/#null-arrays) `(*-1\r\n)`.

The tester will also send a `GEOPOS` command using a key that doesn't exist:

```bash
> GEOPOS missing_key London Munich
# Expecting [nil, nil], encoded as "*2\r\n*-1\r\n*-1\r\n"
```

The tester will assert that:

- The response is a RESP array that contains as many elements as the number of locations requested
- Each element of the array is a [null array](https://redis.io/docs/latest/develop/reference/protocol-spec/#null-arrays) `(*-1\r\n)`

#### Notes

- In this stage, you can hardcode the returned latitude and longitude values to be 0 (or any other valid floating point number). We'll get to testing actual latitude and longitude values in later stages.

### Decode coordinates

<https://app.codecrafters.io/courses/redis/stages/hb5>

In this stage, you'll add support for decoding the coordinates of a location.

#### Decoding latitude and longitude from score

The algorithm to get back the latitude and longitude is essentially the reverse of the one used to compute the score from them.

The [GitHub repository](https://github.com/codecrafters-io/redis-geocoding-algorithm) we referenced earlier explains how this conversion is done. It includes:

- A description of the algorithm used, along with pseudocode
- Code samples in multiple languages.
- A set of locations & scores to test against

Here's the [repository link](https://github.com/codecrafters-io/redis-geocoding-algorithm).

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will add multiple locations using the `ZADD` command. The scores used are valid scores that can be converted back to latitude and longitude values.

```bash
$ redis-cli
> ZADD location_key 3663832614298053 "Foo"
> ZADD location_key 3876464048901851 "Bar"
> ZADD location_key 3468915414364476 "Baz"
> ZADD location_key 3781709020344510 "Caz"
```

The tester will then send multiple `GEOPOS` commands:

```bash
> GEOPOS location_key Foo
# Expecting [["2.294471561908722", "48.85846255040141"]]
```

The tester will validate that the response is a RESP array, which is encoded as:

```
*1\r\n
*2\r\n
$17\r\n
2.294471561908722\r\n
$17\r\n
48.85846255040141\r\n
```

#### Notes

- The conversion from latitude/longitude to score and back is lossy, so the tester will be lenient in checking the coordinates provided - it will accept any coordinates that are within 6 decimal places of the original values. For example, for the example shown above, any of the following values will be accepted:
  - `["2.2944715", "48.8584625"]`
  - `["2.294472", "48.858463"]`
  - `["2.294471594", "48.858462987"]`

### Calculate distance

<https://app.codecrafters.io/courses/redis/stages/ek6>

In this stage, you'll add support for calculating the distance between two locations using the `GEODIST` command.

#### The `GEODIST` command

The `GEODIST` command returns the distance between two members of a key.

Example usage:

```bash
> GEODIST places Munich Paris
"682477.7582"
```

The distance is returned in meters, encoded as a [RESP bulk string](https://redis.io/docs/latest/develop/reference/protocol-spec/#bulk-strings).

Redis uses the [Haversine's Formula](https://rosettacode.org/wiki/Haversine_formula) to calculate the distance between two points. You can see how this is done in the Redis source code [here](https://github.com/redis/redis/blob/4322cebc1764d433b3fce3b3a108252648bf59e7/src/geohash_helper.c#L228C1-L228C72).

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then add multiple locations using the `GEOADD` command:

```bash
$ redis-cli
> GEOADD places 11.5030378 48.164271 "Munich"
> GEOADD places 2.2944692 48.8584625 "Paris"
```

The tester will then send multiple `GEODIST` commands specifying two locations:

```bash
> GEODIST places Munich Paris
# Expecting "682477.7582"
```

The tester will validate that the response is a RESP bulk string that contains the distance between the two locations, for example:

```bash
$11\r\n682477.7582\r\n
```

#### Notes

- When computing distance, set the value of Earth's radius to 6372797.560856 meters. This is the exact value [used by Redis](https://github.com/redis/redis/blob/35aacdf80a0871c933047fc46655b98a73a9374e/src/geohash_helper.c#L52).

- The distance should match the actual distance between the provided locations with a precision of up to 4 decimal places after rounding. For example, if the expected distance is 12345.6789, any of the following returned values will be accepted:
  - `12345.67891`
  - `12345.6789`
  - `12345.67889`

### Search within radius

<https://app.codecrafters.io/courses/redis/stages/rm9>

In this stage, you'll add support for searching locations within a given radius using the `GEOSEARCH` command.

#### The GEOSEARCH command

The `GEOSEARCH` command lets you search for locations within a given radius.

It supports several search modes. In our implementation, we'll focus only on the `FROMLONLAT` mode. The `FROMLONLAT` mode searches by directly specifying longitude and latitude.

Example usage:

```bash
> GEOSEARCH places FROMLONLAT 2 48 BYRADIUS 100 m
1) "Paris"
```

The example command above searches for locations in the `places` key that are within 100 meters of the point (longitude: 2, latitude: 48).

The response is a RESP array containing the names of the locations that match the search criteria, each encoded as a [RESP bulk string](https://redis.io/docs/latest/develop/reference/protocol-spec/#bulk-strings).

Note that there are two options we passed to the command:

- `FROMLONLAT <longitude> <latitude>` — This option specifies the center point for the search.
- `BYRADIUS <radius> <unit>` — This option searches within a circular area of the given radius and unit (m, km, mi, etc.).

Redis supports other such options, but in this challenge we'll only use `FROMLONLAT` and `BYRADIUS`.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will add multiple locations using the `GEOADD` command.

```bash
$ redis-cli
> GEOADD places 11.5030378 48.164271 "Munich"
> GEOADD places 2.2944692 48.8584625 "Paris"
> GEOADD places -0.0884948 51.506479 "London"
```

The tester will then send multiple `GEOSEARCH` commands:

```bash
> GEOSEARCH places FROMLONLAT 2 48 BYRADIUS 100000 m
# Expecting ["Paris"]

> GEOSEARCH places FROMLONLAT 2 48 BYRADIUS 500000 m
# Expecting ["Paris, "London"] (Any order)

> GEOSEARCH places FROMLONLAT 11 50 BYRADIUS 300000 m
# Expecting ["Munich"]
```

The tester will validate that the response is a RESP array, for example

```
*2\r\n
$5\r\n
Paris\r\n
$6\r\n
London\r\n
```

Locations can be returned in any order.

#### Notes

- The tester will always use the `FROMLONLAT` and `BYRADIUS` options when sending a `GEOSEARCH` command.

## Authentication

### Respond to ACL WHOAMI

<https://app.codecrafters.io/courses/redis/stages/jn4>

In this stage, you'll add support for responding to the `ACL WHOAMI` command.

#### The `ACL WHOAMI` Command

The [`ACL WHOAMI`](https://redis.io/docs/latest/commands/acl-whoami/) command returns the username associated with the current connection. 

By default, every new connection in Redis is automatically authenticated as the `default` user. 

For example:

```bash
> ACL WHOAMI
"default"
```

The command returns the username of the currently authenticated user, encoded as a [RESP bulk string](https://redis.io/docs/latest/develop/reference/protocol-spec/#bulk-strings).

This default authentication behavior can be changed so that new connections start out unauthenticated. We'll cover that in later stages.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send an `ACL WHOAMI` command:

```bash
# Expect RESP bulk string: "default"
$ redis-cli ACL WHOAMI
"default"
```

The tester will expect to receive `$7\r\ndefault\r\n` as a response. That's the string `default` encoded as a [bulk string](https://redis.io/docs/latest/develop/reference/protocol-spec/#bulk-strings).

#### Notes

- In this stage, you can hardcode the response of the `ACL WHOAMI` command to be `default`. We'll get to enforcing authentication in the later stages.

### Respond to ACL GETUSER

<https://app.codecrafters.io/courses/redis/stages/gx8>

In this stage, you'll add support for the `ACL GETUSER` command.

#### The `ACL GETUSER` Command

The [`ACL GETUSER`](https://redis.io/docs/latest/commands/acl-getuser/) command retrieves the properties of a specified user. In Redis, the `default` user is present from the start and does not need to be created.

For example:

```bash
> ACL GETUSER default
1) "flags"
2) (empty array)
...
```

The return value is a nested RESP array of property name-value pairs for the user:

```bash
[property_name_1, property_value_1, property_name_2, property_value_2, ...]
```

For this stage, you'll implement just the `flags` property. 

#### The `flags` Property

The `flags` property represents a set of attributes that describe how a user behaves or what special permissions they have. Each flag is a short label that defines part of the user’s configuration.

For example, after creating or modifying a user, the flags array might look like this:

```bash
> ACL GETUSER alice
1) "flags"
2) 1) "on"
   2) "allkeys"
   3) "allcommands"
```

For this stage, since the `default` user has no `flags` to report yet, you will hardcode the value to be an empty RESP array (`[]`). 

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send an `ACL GETUSER` command specifying the `default` user:

```bash
# Expect RESP array: ["flags", []]
$ redis-cli
> ACL GETUSER default
1) "flags"
2) (empty array)
```

The tester will verify that the response is a RESP array with two elements:

1. The first element is the bulk string `flags`.
2. The second element is an empty RESP array.

### The nopass flag

<https://app.codecrafters.io/courses/redis/stages/ql6>

In this stage, you'll add support for responding to the `ACL GETUSER` command with the `nopass` flag set.

#### The `nopass` flag

The `nopass` flag is one of the user flags in Redis. It controls the password authentication behaviour:

- If `nopass` is set for a user, authentication succeeds with any password (or no password)
- Setting `nopass` clears any passwords associated with the user

The `default` user has `nopass` set by default, which is why new connections are automatically authenticated.

For example:
```bash
> ACL GETUSER default
1) "flags"
2) 1) "nopass"
...
```

The flags are encoded as a RESP array of bulk strings. 

For this stage, you only need to respond to the `ACL GETUSER` command with the `nopass` flag set. We'll get to enforcing the behavior of the `nopass` flag in later stages.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send an `ACL GETUSER` command specifying the `default` user.

```bash
# Expect RESP array: ["flags", ["nopass"]]
$ redis-cli
> ACL GETUSER default
1) "flags"
2) 1) "nopass"
```

The tester will verify that the response is a RESP array with two elements:
1. The first element is the bulk string `flags`.
2. The second element is a RESP array containing one element: the bulk string `nopass`.

### The passwords property

<https://app.codecrafters.io/courses/redis/stages/pl7>

In this stage, you'll add support for responding to the `ACL GETUSER` command with the `passwords` property.

#### The `passwords` Property

The `passwords` property lists all hash-encoded passwords associated with a user. 

The `default` user does not have any associated passwords unless explicitly configured:

```bash
> ACL GETUSER default
 1) "flags"
 2) 1) "nopass"
 3) "passwords"
 4) (empty array)
```

Your `ACL GETUSER` response must now include the following:

1. The string `passwords`, encoded as a bulk string.
2. A RESP array containing the list of passwords. Since the `default` user has none, you must hardcode this to be an empty array.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send an `ACL GETUSER` command specifying the `default` user:

```bash
# Expect RESP array: ["flags", ["nopass"], "passwords", []]
$ redis-cli ACL GETUSER default
 1) "flags"
 2) 1) "nopass"
 3) "passwords"
 4) (empty array)
```

The tester will validate that the response is a RESP array with four elements:

1. The first element of the array is the string `flags`, encoded as a RESP bulk string.
2. The second element of the array is a RESP array and contains the `nopass` flag.
3. The third element of the array is the string `passwords`, encoded as a bulk string.
4. The fourth element of the array is an empty array.

### Setting default user password

<https://app.codecrafters.io/courses/redis/stages/uv9>

In this stage, you'll add support for setting the `default` user's password.

#### The `ACL SETUSER` Command

The [`ACL SETUSER`](https://redis.io/docs/latest/commands/acl-setuser/) command modifies the properties of an existing user. 

When the command is used with the `>` rule, it adds a password for the specified user:

```bash
> ACL SETUSER default >mypassword
OK
```

The server responds with `OK` encoded as a RESP simple string (`+OK\r\n`).

#### Password Storage

Adding a password for a user with `ACL SETUSER` has two effects:
- The password is stored as a SHA-256 hash.
- The `nopass` flag is automatically removed.

For example:

```bash
> ACL SETUSER default >mypassword
OK

> ACL GETUSER default
 1) "flags"
 2) (empty array)
 3) "passwords"
 4) 1) "89e01536ac207279409d4de1e5253e01f4a1769e696db0d6062ca9b8f56767c8"
```

Notice that the `nopass` flag is now gone from the `flags` array. Also, the `mypassword` SHA-256 hash is stored as a bulk string in the `passwords` array. 

Storing only the SHA-256 hash is a security best practice Redis uses to prevent password leaks if the database is compromised.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send an `ACL GETUSER` command, specifying the `default` user:

```bash
# Expect RESP array: ["flags", ["nopass"], "passwords", []]
$ redis-cli ACL GETUSER default
1) "flags"
2) 1) "nopass"
3) "passwords"
4) (empty array)
```

The tester will verify that:

- The `nopass` flag is present in the flags array.
- The `passwords` array is empty.

Next, the tester will send a `ACL SETUSER` command, specifying the `default` user and a password:

```bash
# Expect: +OK\r\n
> ACL SETUSER default >mypassword
OK
```

Your server must respond with `+OK\r\n`.

Finally, the tester will send a `ACL GETUSER` command, specifying the `default` user:

```bash
# Expect RESP array: ["flags", ["nopass"], "passwords", ["89e01536ac207279409d4de1e5253e01f4a1769e696db0d6062ca9b8f56767c8"]]
> ACL GETUSER default
1) "flags"
2) 1) "nopass"
3) "passwords"
4) 1) "89e01536ac207279409d4de1e5253e01f4a1769e696db0d6062ca9b8f56767c8"
```

The tester will validate the following for your response:

- The `nopass` flag is no longer present.
- The `passwords` array contains the SHA-256 hash of `mypassword` encoded as a bulk string.

#### Notes

- Redis uses the SHA-256 hashing algorithm for password storage. You'll need to compute the SHA-256 hash of the provided password and store it.
- The password hash should be stored as a lowercase hexadecimal string.

### The AUTH command

<https://app.codecrafters.io/courses/redis/stages/hz3>

In this stage, you'll add support for the `AUTH` command.

#### The `AUTH` Command

The [`AUTH`](https://redis.io/docs/latest/commands/auth/) command authenticates the current connection with a specified username and password.

The command format is:

```bash
AUTH <username> <password>
```

For example:

```bash
# Authentication failure
> AUTH default wrongpassword
(error) WRONGPASS invalid username-password pair or user is disabled.

# Authentication success
> AUTH default correctpassword
OK
```

The server responds with `+OK\r\n` if the specified password's hash matches any of the hashes in the user's password list.

If no match is found, the server responds with the [RESP simple error](https://redis.io/docs/latest/develop/reference/protocol-spec/#simple-errors): `WRONGPASS invalid username-password pair or user is disabled.`

For this stage, you just need to respond to the `AUTH` command appropriately. You don't need to actually authenticate the connection. We'll get to that in later stages.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send an `ACL SETUSER` command, specifying the `default` user and a password:

```bash
$ redis-cli ACL SETUSER default >mypassword
OK
```

Your server should respond with `OK` encoded as a simple string (`+OK\r\n`).

Next, the tester will send two `AUTH` commands: one with a wrong password and another with a correct password.

```bash
# Expect error starting with: WRONGPASS
> AUTH default wrongpassword
(error) WRONGPASS invalid username-password pair or user is disabled.

# Expect: +OK\r\n
> AUTH default mypassword
OK
```

The tester will verify that:
- Wrong passwords return a [simple error](https://redis.io/docs/latest/develop/reference/protocol-spec/#simple-errors) starting with `WRONGPASS`.
- Correct passwords return `+OK\r\n`.

#### Notes

- The tester will be lenient in checking error messages. Any authentication failure starting with `WRONGPASS` is valid. For example:
    - `WRONGPASS wrong password`
    - `WRONGPASS invalid authentication`

### Enforce authentication

<https://app.codecrafters.io/courses/redis/stages/nm2>

In this stage, you'll add support for enforcing authentication for the `default` user.

#### Enforcing `default` User Authentication

When you create a new connection, it is automatically authenticated as the `default` user. This happens because the `nopass` flag is set for the `default` user from the start.

Once you set a password for the `default` user, new connections will no longer be automatically authenticated. However, any connections that were already authenticated will stay logged in.

For example:

```bash
# Client 1
$ redis-cli ACL SETUSER default >password
OK

# This connection remains authenticated as the default user
> ACL WHOAMI
"default"

# Client 2
# This connection is not authenticated
$ redis-cli ACL WHOAMI
(error) NOAUTH Authentication required.
```

When an unauthenticated connection tries to execute a command, return the simple error: `NOAUTH Authentication required.`

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send commands to two different clients:

```bash
# Client 1
$ redis-cli
# Expect: +OK\r\n
> ACL SETUSER default >newpassword
OK

# Expect RESP bulk string: "default"
> ACL WHOAMI
"default"

# Client 2 (new connection)
$ redis-cli
# Expect error starting with: "NOAUTH"
> ACL WHOAMI
(error) NOAUTH Authentication required.
```

The tester will verify that:
- Client 1 can still execute commands and remains authenticated as the `default` user.
- Client 2 receives a `NOAUTH` error when trying to execute commands without authentication.

### Authenticate using AUTH

<https://app.codecrafters.io/courses/redis/stages/ws7>

In this stage, you'll implement enforcing authentication using the `AUTH` command.

#### Enforcing Authentication Using `AUTH`

After the `AUTH` command succeeds, the connection becomes authenticated as the specified user. Once authenticated, the connection can execute commands that previously returned `NOAUTH` errors:

For example:

```bash
# Client 1
$ redis-cli
> ACL SETUSER default >newpassword
OK

> ACL WHOAMI
"default"

# Client 2 (new connection)
$ redis-cli
> ACL WHOAMI
(error) NOAUTH Authentication required.

> AUTH default newpassword
OK

# Client 2 is now authenticated as the 'default' user
> ACL WHOAMI
"default"
```

The authentication lasts for the entire connection, so the client does not need to re-authenticate for every command.

#### Tests

The tester will execute your program like this:

```bash
$ ./your_program.sh
```

It will then send commands to two different clients:

```bash
# Client 1
$ redis-cli
> ACL SETUSER default >newpassword
OK

> ACL WHOAMI
"default"  # Still authenticated

# Client 2 (new connection)
$ redis-cli
> PING
(error) NOAUTH Authentication required.

> AUTH default newpassword
OK

> PING
PONG
```

The tester will verify that:

1. A new client receives a `NOAUTH` error when attempting to execute commands before authenticating.
2. The `AUTH` command returns `OK` as a simple string on successful authentication.
3. The client can execute commands successfully after authentication.

#### Notes

- All commands except `AUTH` should return `NOAUTH` errors when executed by an unauthenticated connection. This rule applies to every command you have implemented, not only to `ACL` commands.
