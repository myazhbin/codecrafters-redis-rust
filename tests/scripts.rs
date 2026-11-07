//! Runs the end-to-end test scripts in this directory, in bash and Python, as part of
//! `cargo test`: one test each, against the server binary that Cargo builds.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

macro_rules! script_tests {
    ($($test:ident: $script:literal),* $(,)?) => {
        /// The scripts, each run by the test named before it below.
        const SCRIPTS: &[&str] = &[$($script),*];

        $(
            #[test]
            fn $test() {
                run_script($script);
            }
        )*
    };
}

script_tests! {
    rpush: "rpush.sh",
    lrange: "lrange.sh",
    lpush: "lpush.sh",
    llen: "llen.sh",
    lpop: "lpop.sh",
    blpop: "blpop.py",
    type_command: "type.sh",
    xadd: "xadd.sh",
    xrange: "xrange.sh",
    xread: "xread.sh",
    xread_block: "xread_block.py",
    incr: "incr.sh",
    transaction: "transaction.py",
    watch: "watch.py",
    port: "port.sh",
    info: "info.sh",
    handshake: "handshake.py",
    master_handshake: "master_handshake.py",
    propagation: "propagation.py",
    replica_commands: "replica_commands.py",
    wait: "wait.py",
    replication: "replication.py",
    rdb: "rdb.py",
    aof: "aof.py",
    del: "del.sh",
    pubsub: "pubsub.py",
    zset: "zset.sh",
    bitmap: "bitmap.sh",
}

/// Runs tests/<script>, failing if it reports a failure, with the script's output shown.
/// The tests run at the same time, so each script's server gets a port of its own: the
/// first script's the default port (so that gets tested), and the others' from 6401 up.
fn run_script(script: &str) {
    let index = SCRIPTS.iter().position(|&name| name == script).unwrap();
    let interpreter = match script.rsplit_once('.') {
        Some((_, "py")) => "python3",
        _ => "bash",
    };
    let mut command = Command::new(interpreter);
    command
        .arg(tests_dir().join(script))
        .arg(env!("CARGO_BIN_EXE_codecrafters-redis"))
        // Keeps Python from leaving a __pycache__ directory behind.
        .env("PYTHONDONTWRITEBYTECODE", "1");
    if index > 0 {
        command.env("TEST_PORT", (6400 + index).to_string());
    }
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("couldn't run {interpreter}: {error}"));
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "tests/{script} failed (its output is above)"
    );
}

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

#[test]
fn every_script_has_a_test() {
    for entry in fs::read_dir(tests_dir()).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        let is_script = name.ends_with(".sh") || name.ends_with(".py");
        if is_script && !name.starts_with("helpers.") {
            assert!(
                SCRIPTS.contains(&name.as_str()),
                "tests/{name} needs adding to script_tests! in tests/scripts.rs"
            );
        }
    }
}
