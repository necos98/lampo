//! Credentials Lampo's own MySQL clients authenticate with (the administrative account of each
//! local instance). A password never reaches a command line or a log: the clients read it from an
//! option file ([`MysqlPassword::client_option_file`]) the infrastructure writes for the length of
//! one run.

use std::fmt;

use crate::error::DomainError;

/// The administrative account of every local instance (`mysqld --initialize-insecure` creates it
/// with no password), which Lampo's own clients connect as.
pub const ROOT_ACCOUNT: &str = "root";

/// Longest password accepted, in bytes (MySQL sets no limit; this bounds what is stored and read).
pub const MAX_PASSWORD_BYTES: usize = 256;

/// The password of a MySQL account.
///
/// Never printed: `Debug` redacts it and there is no `Display`. Restricted to what an option file
/// carries verbatim between double quotes: no control characters (a line break would end the
/// value and start an option of its own) and no `"` (the option-file reader cannot escape it).
#[derive(Clone, PartialEq, Eq)]
pub struct MysqlPassword(String);

impl MysqlPassword {
    pub fn new(password: impl Into<String>) -> Result<Self, DomainError> {
        let password = password.into();
        let invalid = |reason: &str| Err(DomainError::InvalidMysqlPassword(reason.to_string()));
        if password.is_empty() {
            invalid("it is empty (an account without a password needs no stored password)")
        } else if password.len() > MAX_PASSWORD_BYTES {
            invalid(&format!("it is longer than {MAX_PASSWORD_BYTES} bytes"))
        } else if password.chars().any(char::is_control) {
            invalid("it contains a control character (line breaks, tabs, …)")
        } else if password.contains('"') {
            invalid("it contains a double quote")
        } else {
            Ok(Self(password))
        }
    }

    /// The password itself, for the adapters that store it or hand it to a client.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// An option file giving this password to every MySQL client (`mysql`, `mysqldump`,
    /// `mysqladmin` read the `[client]` group), for `--defaults-file` / `--defaults-extra-file`.
    ///
    /// The value is double-quoted, so `#`, `'`, `;` and leading or trailing spaces are kept
    /// literally; `\` is the reader's escape character and is doubled.
    pub fn client_option_file(&self) -> String {
        format!("[client]\npassword=\"{}\"\n", self.0.replace('\\', "\\\\"))
    }
}

impl fmt::Debug for MysqlPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MysqlPassword(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_printable_passwords_and_never_prints_them() {
        let longest = "x".repeat(MAX_PASSWORD_BYTES);
        for p in ["s3cret", "with spaces ", "#hash;semi'quote", "àèì 😀", longest.as_str()] {
            let password = MysqlPassword::new(p).unwrap();
            assert_eq!(password.expose(), p);
            assert_eq!(format!("{password:?}"), "MysqlPassword(<redacted>)");
        }
    }

    #[test]
    fn refuses_what_an_option_file_cannot_carry_without_echoing_it() {
        let too_long = "x".repeat(MAX_PASSWORD_BYTES + 1);
        for p in ["", "line\nbreak", "tab\there", "nul\0", "quote\"d", too_long.as_str()] {
            let err = MysqlPassword::new(p).unwrap_err();
            assert!(matches!(err, DomainError::InvalidMysqlPassword(_)), "{p:?}");
            if !p.is_empty() && p.len() < 20 {
                assert!(!err.to_string().contains(p), "the message must not carry the password: {err}");
            }
        }
    }

    #[test]
    fn option_file_quotes_the_value_and_doubles_backslashes() {
        let file = MysqlPassword::new(r"a\b #c 'd'").unwrap().client_option_file();
        assert_eq!(file, "[client]\npassword=\"a\\\\b #c 'd'\"\n");
    }
}
