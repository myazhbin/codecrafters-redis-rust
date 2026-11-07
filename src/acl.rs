//! Users, for the ACL commands and AUTH.

use crate::sha256::sha256_hex;
use std::collections::HashMap;

/// The user every connection starts out as.
pub const DEFAULT_USER: &[u8] = b"default";

#[derive(Clone, Default)]
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

impl User {
    /// Applies one of ACL SETUSER's rules about passwords: `>password` and `<password` add
    /// and remove a password, `#hash` and `!hash` do the same by its hash, `nopass` lets
    /// any password do, and `resetpass` forgets them all, so none will do.
    fn apply(&mut self, rule: &[u8]) -> Result<(), &'static str> {
        match rule {
            [b'>', password @ ..] => self.add_password(sha256_hex(password)),
            [b'#', hash @ ..] => {
                let hash = std::str::from_utf8(hash).unwrap_or_default();
                let is_hex = |c: char| c.is_ascii_digit() || ('a'..='f').contains(&c);
                if hash.len() != 64 || !hash.chars().all(is_hex) {
                    return Err("The password hash must be exactly 64 characters and contain \
                                only lowercase hexadecimal characters");
                }
                self.add_password(hash.to_string());
            }
            [b'<', password @ ..] => self.remove_password(&sha256_hex(password))?,
            [b'!', hash @ ..] => self.remove_password(&String::from_utf8_lossy(hash))?,
            _ if rule.eq_ignore_ascii_case(b"nopass") => {
                self.nopass = true;
                self.passwords.clear();
            }
            _ if rule.eq_ignore_ascii_case(b"resetpass") => {
                self.nopass = false;
                self.passwords.clear();
            }
            _ => return Err("Syntax error"),
        }
        Ok(())
    }

    /// Adds a password by its hash. Once a user has a password, it's needed.
    fn add_password(&mut self, hash: String) {
        if !self.passwords.contains(&hash) {
            self.passwords.push(hash);
        }
        self.nopass = false;
    }

    fn remove_password(&mut self, hash: &str) -> Result<(), &'static str> {
        let Some(i) = self.passwords.iter().position(|h| h == hash) else {
            return Err("The password you are trying to remove from the user does not exist");
        };
        self.passwords.remove(i);
        Ok(())
    }
}

impl Users {
    pub fn get(&self, name: &[u8]) -> Option<&User> {
        self.0.get(name)
    }

    /// ACL SETUSER: applies `rules` to the user called `name`, creating the user if there
    /// isn't one. If a rule can't be applied, none of them are.
    pub fn set_user(&mut self, name: &[u8], rules: &[Vec<u8>]) -> Result<(), String> {
        let mut user = self.0.get(name).cloned().unwrap_or_default();
        for rule in rules {
            user.apply(rule).map_err(|why| {
                let rule = String::from_utf8_lossy(rule);
                format!("ERR Error in ACL SETUSER modifier '{rule}': {why}")
            })?;
        }
        self.0.insert(name.to_vec(), user);
        Ok(())
    }

    /// Whether `password` lets a client log in as the user called `name`.
    pub fn check(&self, name: &[u8], password: &[u8]) -> bool {
        self.get(name).is_some_and(|user| {
            user.nopass || user.passwords.contains(&sha256_hex(password))
        })
    }

    /// Whether the default user needs no password. While that's so, every connection can
    /// run commands as the default user, as in Redis, whether or not it has logged in.
    pub fn default_needs_no_password(&self) -> bool {
        self.get(DEFAULT_USER).is_some_and(|user| user.nopass)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MYPASSWORD: &str = "89e01536ac207279409d4de1e5253e01f4a1769e696db0d6062ca9b8f56767c8";

    fn rules(rules: &[&str]) -> Vec<Vec<u8>> {
        rules.iter().map(|rule| rule.as_bytes().to_vec()).collect()
    }

    fn default(users: &Users) -> &User {
        users.get(DEFAULT_USER).unwrap()
    }

    #[test]
    fn a_password_replaces_nopass() {
        let mut users = Users::default();
        assert!(default(&users).nopass);
        assert!(users.default_needs_no_password());
        users.set_user(DEFAULT_USER, &rules(&[">mypassword"])).unwrap();
        assert!(!default(&users).nopass);
        assert_eq!(default(&users).passwords, [MYPASSWORD]);
        assert!(!users.default_needs_no_password());
    }

    #[test]
    fn adds_and_removes_passwords() {
        let mut users = Users::default();
        users.set_user(DEFAULT_USER, &rules(&[">a", ">b", ">a"])).unwrap();
        assert_eq!(default(&users).passwords.len(), 2);
        users.set_user(DEFAULT_USER, &rules(&["<a"])).unwrap();
        assert_eq!(default(&users).passwords, [sha256_hex(b"b")]);
        users.set_user(DEFAULT_USER, &rules(&[&format!("#{MYPASSWORD}")])).unwrap();
        users.set_user(DEFAULT_USER, &rules(&[&format!("!{}", sha256_hex(b"b"))])).unwrap();
        assert_eq!(default(&users).passwords, [MYPASSWORD]);
        users.set_user(DEFAULT_USER, &rules(&["nopass"])).unwrap();
        assert!(default(&users).nopass && default(&users).passwords.is_empty());
        users.set_user(DEFAULT_USER, &rules(&[">x", "resetpass"])).unwrap();
        assert!(!default(&users).nopass && default(&users).passwords.is_empty());
    }

    #[test]
    fn a_bad_rule_changes_nothing() {
        let mut users = Users::default();
        let error = users.set_user(DEFAULT_USER, &rules(&[">pass", "bogus"])).unwrap_err();
        assert_eq!(error, "ERR Error in ACL SETUSER modifier 'bogus': Syntax error");
        assert!(default(&users).nopass && default(&users).passwords.is_empty());
        assert!(users.set_user(DEFAULT_USER, &rules(&["<missing"])).is_err());
        assert!(users.set_user(DEFAULT_USER, &rules(&["#abc"])).is_err());
        assert!(users.set_user(DEFAULT_USER, &rules(&[&format!("#{}", MYPASSWORD.to_uppercase())])).is_err());
    }

    #[test]
    fn checks_passwords() {
        let mut users = Users::default();
        assert!(users.check(DEFAULT_USER, b"anything"));
        users.set_user(DEFAULT_USER, &rules(&[">mypassword"])).unwrap();
        assert!(users.check(DEFAULT_USER, b"mypassword"));
        assert!(!users.check(DEFAULT_USER, b"wrongpassword"));
        assert!(!users.check(b"nobody", b"mypassword"));
    }

    #[test]
    fn creates_users() {
        let mut users = Users::default();
        users.set_user(b"alice", &rules(&[">secret"])).unwrap();
        let alice = users.get(b"alice").unwrap();
        assert!(!alice.nopass);
        assert_eq!(alice.passwords, [sha256_hex(b"secret")]);
        users.set_user(b"bob", &[]).unwrap();
        assert!(users.get(b"bob").unwrap().passwords.is_empty());
    }
}
