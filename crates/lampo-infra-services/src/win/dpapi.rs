//! DPAPI for the current Windows user (`CryptProtectData` / `CryptUnprotectData`): what one user
//! encrypts only that user, logged on to this machine, can decrypt.

use std::io;
use std::ptr::{null, null_mut};
use std::sync::atomic::{Ordering, compiler_fence};

use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData};

use super::LocalMem;

fn blob(data: &[u8]) -> io::Result<CRYPT_INTEGER_BLOB> {
    let len = u32::try_from(data.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "data too large for DPAPI"))?;
    // DPAPI never writes through an input blob.
    Ok(CRYPT_INTEGER_BLOB { cbData: len, pbData: data.as_ptr().cast_mut() })
}

/// Takes the system-allocated output of a DPAPI call: copied out, wiped, freed.
fn take_output(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let _free = LocalMem(out.pbData.cast());
    if out.pbData.is_null() {
        return Vec::new();
    }
    // SAFETY: DPAPI returned `cbData` initialised bytes at `pbData`, ours until LocalFree.
    unsafe {
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        std::ptr::write_bytes(out.pbData, 0, out.cbData as usize);
        compiler_fence(Ordering::SeqCst);
        bytes
    }
}

/// Encrypts `plain` for the current user. `entropy` binds the result to one purpose: decrypting
/// needs the same bytes.
pub(crate) fn protect(plain: &[u8], entropy: &[u8]) -> io::Result<Vec<u8>> {
    let (input, entropy) = (blob(plain)?, blob(entropy)?);
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
    // SAFETY: valid input blobs borrowed for the call; the output is freed by `take_output`.
    if unsafe { CryptProtectData(&input, null(), &entropy, null(), null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(take_output(out))
}

/// Decrypts what [`protect`] encrypted for this user with the same `entropy`.
pub(crate) fn unprotect(sealed: &[u8], entropy: &[u8]) -> io::Result<Vec<u8>> {
    let (input, entropy) = (blob(sealed)?, blob(entropy)?);
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
    // SAFETY: as in `protect`.
    if unsafe { CryptUnprotectData(&input, null_mut(), &entropy, null(), null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(take_output(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_needs_the_same_entropy() {
        let sealed = protect(b"s3cret", b"purpose-a").unwrap();
        assert!(!sealed.windows(6).any(|w| w == b"s3cret"), "encrypted");
        assert_eq!(unprotect(&sealed, b"purpose-a").unwrap(), b"s3cret");
        assert!(unprotect(&sealed, b"purpose-b").is_err());
        assert!(unprotect(b"not a dpapi blob", b"purpose-a").is_err());
    }
}
