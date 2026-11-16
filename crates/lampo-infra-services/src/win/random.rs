//! The system's cryptographically secure random number generator (`BCryptGenRandom` with the
//! system-preferred RNG).

use std::io;
use std::ptr::null_mut;

use windows_sys::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};

/// Fills `buffer` with random bytes.
pub(crate) fn fill(buffer: &mut [u8]) -> io::Result<()> {
    let len = u32::try_from(buffer.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "random buffer too large"))?;
    // SAFETY: `buffer` is valid for `len` writable bytes; no algorithm handle with this flag.
    let status = unsafe { BCryptGenRandom(null_mut(), buffer.as_mut_ptr(), len, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if status == 0 { Ok(()) } else { Err(io::Error::other(format!("BCryptGenRandom failed (NTSTATUS {status:#010x})"))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_with_different_bytes_each_time() {
        let (mut a, mut b) = ([0u8; 32], [0u8; 32]);
        fill(&mut a).unwrap();
        fill(&mut b).unwrap();
        assert_ne!(a, b);
        assert_ne!(a, [0u8; 32]);
    }
}
