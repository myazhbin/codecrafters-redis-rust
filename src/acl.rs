//! Users, for the ACL commands and AUTH.

use std::collections::HashMap;

/// The user every connection starts out as.
pub const DEFAULT_USER: &[u8] = b"default";

pub struct User {
    /// With `nopass`, any password will do, so new connections are logged in as this user
    /// straight away.
    pub nopass: bool,
    /// The SHA-256 hashes of the user's passwords, in lowercase hex.
    pub passwords: Vec<String>,
}

/// The users the server knows about.
pub struct Users(HashMap<Vec<u8>, User>);

impl Default for Users {
    /// Just the default user, who needs no password.
    fn default() -> Self {
        let default = User {
            nopass: true,
            passwords: Vec::new(),
        };
        Users(HashMap::from([(DEFAULT_USER.to_vec(), default)]))
    }
}

impl Users {
    pub fn get(&self, name: &[u8]) -> Option<&User> {
        self.0.get(name)
    }

    /// The user a new connection is logged in as: the default user, unless it needs a
    /// password, in which case nobody until the client sends AUTH.
    pub fn new_connection_user(&self) -> Option<Vec<u8>> {
        let default = self.get(DEFAULT_USER)?;
        default.nopass.then(|| DEFAULT_USER.to_vec())
    }
}
