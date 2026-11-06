//! The server's settings, from the command line.

/// Settings from the command line.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// The port to listen on, set with `--port`.
    pub port: u16,
    /// The host and port of the master to replicate, set with `--replicaof`. A server
    /// without one is a master itself.
    pub replicaof: Option<(String, u16)>,
    /// The directory the RDB file is in, set with `--dir`: by default, the working
    /// directory the server started in.
    pub dir: String,
    /// The RDB file's name, set with `--dbfilename`.
    pub dbfilename: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            port: 6379,
            replicaof: None,
            dir: String::new(),
            dbfilename: "dump.rdb".to_string(),
        }
    }
}

impl Config {
    /// The parameters CONFIG GET can read, with their values.
    pub fn params(&self) -> Vec<(&'static str, String)> {
        vec![
            ("dir", self.dir.clone()),
            ("dbfilename", self.dbfilename.clone()),
            ("port", self.port.to_string()),
        ]
    }
}

/// Reads the command-line arguments that follow the program's name:
/// `[--port <port>] [--replicaof "<host> <port>"] [--dir <dir>] [--dbfilename <name>]`.
/// `cwd` is the working directory, which `--dir` defaults to.
pub fn parse_args(mut args: impl Iterator<Item = String>, cwd: String) -> Result<Config, String> {
    let mut config = Config {
        dir: cwd,
        ..Config::default()
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
            "--dir" => config.dir = args.next().ok_or("--dir needs a directory")?,
            "--dbfilename" => {
                config.dbfilename = args.next().ok_or("--dbfilename needs a file name")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Config, String> {
        parse_args(args.iter().map(|arg| arg.to_string()), "/cwd".to_string())
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
    fn rdb_file_defaults_to_dump_rdb_in_the_working_directory() {
        let config = parse(&[]).unwrap();
        assert_eq!(config.dir, "/cwd");
        assert_eq!(config.dbfilename, "dump.rdb");
    }

    #[test]
    fn reads_the_rdb_file_location() {
        let config = parse(&["--dir", "/tmp/redis-files", "--dbfilename", "dump.rdb"]).unwrap();
        assert_eq!(config.dir, "/tmp/redis-files");
        assert_eq!(config.dbfilename, "dump.rdb");
        let config = parse(&["--dbfilename", "x.rdb", "--port", "7000", "--dir", "d"]).unwrap();
        assert_eq!((config.dir.as_str(), config.dbfilename.as_str()), ("d", "x.rdb"));
        assert_eq!(config.port, 7000);
    }

    #[test]
    fn rejects_bad_arguments() {
        let error = |args: &[&str]| parse(args).err().unwrap();
        assert_eq!(error(&["--port"]), "--port needs a port number");
        assert_eq!(error(&["--port", "abc"]), "invalid port: abc");
        assert_eq!(error(&["--port", "70000"]), "invalid port: 70000");
        assert_eq!(error(&["--bogus"]), "unknown argument: --bogus");
        assert_eq!(error(&["--replicaof"]), "--replicaof needs a host and port");
        assert_eq!(error(&["--dir"]), "--dir needs a directory");
        assert_eq!(error(&["--dbfilename"]), "--dbfilename needs a file name");
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
