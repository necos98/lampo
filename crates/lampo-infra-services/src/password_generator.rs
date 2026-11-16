//! New MySQL account passwords from the system RNG: [`PASSWORD_LEN`] letters and digits, so they
//! need no quoting in an option file or an SQL string and survive copying into a `.env` file.

use lampo_domain::mysql::credentials::MysqlPassword;
use lampo_domain::ports::MysqlPasswordGenerator;
use lampo_domain::{PortError, PortResult};

use crate::win::random;

/// 32 characters of a 62-letter alphabet: about 190 bits.
pub const PASSWORD_LEN: usize = 32;
const ALPHABET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
/// The largest multiple of the alphabet's size that fits a byte: bytes from here on are
/// discarded, so every character is equally likely (no modulo bias).
const UNBIASED_LIMIT: u8 = (256 / ALPHABET.len() * ALPHABET.len()) as u8;
/// Random bytes drawn per round: enough for a password in almost every round.
const BATCH: usize = 64;

/// [`MysqlPasswordGenerator`] over `BCryptGenRandom`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemRandomPasswords;

impl MysqlPasswordGenerator for SystemRandomPasswords {
    fn generate(&self) -> PortResult<MysqlPassword> {
        let mut password = String::with_capacity(PASSWORD_LEN);
        let mut batch = [0u8; BATCH];
        while password.len() < PASSWORD_LEN {
            random::fill(&mut batch).map_err(|e| PortError::io("generating a MySQL password", e))?;
            let usable = batch.iter().filter(|&&b| b < UNBIASED_LIMIT).map(|&b| char::from(ALPHABET[usize::from(b) % ALPHABET.len()]));
            password.extend(usable.take(PASSWORD_LEN - password.len()));
        }
        Ok(MysqlPassword::new(password).expect("letters and digits are a valid password"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_are_long_alphanumeric_and_distinct() {
        let a = SystemRandomPasswords.generate().unwrap();
        let b = SystemRandomPasswords.generate().unwrap();
        assert_eq!(a.expose().len(), PASSWORD_LEN);
        assert!(a.expose().bytes().all(|c| c.is_ascii_alphanumeric()), "{}", a.expose());
        assert_ne!(a, b);
    }

    #[test]
    fn the_unbiased_limit_is_a_whole_number_of_alphabets() {
        assert_eq!(UNBIASED_LIMIT, 248);
        assert_eq!(usize::from(UNBIASED_LIMIT) % ALPHABET.len(), 0);
    }
}
