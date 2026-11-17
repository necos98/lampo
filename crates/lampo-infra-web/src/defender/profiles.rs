//! The user profiles registered on this machine, from
//! `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList` (writable by administrators
//! only, readable by everyone): the same list on both sides of the UAC prompt, whoever approves it.

use std::path::PathBuf;

/// The folders of the user profiles of this machine.
pub trait UserProfiles: Send + Sync {
    fn profile_dirs(&self) -> Vec<PathBuf>;
}

/// `ProfileImagePath` of every user account profile (local, domain and Microsoft Entra
/// accounts); the built-in service profiles and temporary `.bak` copies are skipped.
pub struct RegistryProfiles;

/// SID prefixes of user accounts: local and domain (`S-1-5-21-`), Microsoft Entra (`S-1-12-1-`).
const USER_SID_PREFIXES: [&str; 2] = ["S-1-5-21-", "S-1-12-1-"];

fn is_user_profile_key(name: &str) -> bool {
    USER_SID_PREFIXES.iter().any(|prefix| name.starts_with(prefix)) && !name.contains('.')
}

#[cfg(windows)]
impl UserProfiles for RegistryProfiles {
    fn profile_dirs(&self) -> Vec<PathBuf> {
        use winreg::RegKey;
        use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ};
        const PROFILE_LIST: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList";
        const IMAGE_PATH: &str = "ProfileImagePath";
        let Ok(list) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(PROFILE_LIST, KEY_READ) else {
            tracing::warn!("cannot open the profile list");
            return Vec::new();
        };
        list.enum_keys()
            .filter_map(Result::ok)
            .filter(|sid| is_user_profile_key(sid))
            .filter_map(|sid| list.open_subkey_with_flags(&sid, KEY_READ).ok())
            .filter_map(|profile| profile.get_value::<String, _>(IMAGE_PATH).ok())
            .map(PathBuf::from)
            .collect()
    }
}

#[cfg(not(windows))]
impl UserProfiles for RegistryProfiles {
    fn profile_dirs(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_user_account_profiles_are_kept() {
        for user in ["S-1-5-21-2777171597-854322427-1938791069-1000", "S-1-12-1-1234-5678-91011-1213"] {
            assert!(is_user_profile_key(user), "{user}");
        }
        for other in ["S-1-5-18", "S-1-5-19", "S-1-5-20", "S-1-5-21-1-2-3-1000.bak", "S-1-5-80-1"] {
            assert!(!is_user_profile_key(other), "{other}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn this_users_profile_is_registered() {
        let home = std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap();
        let profiles = RegistryProfiles.profile_dirs();
        assert!(profiles.iter().any(|p| p.as_os_str().eq_ignore_ascii_case(home.as_os_str())), "{home:?} not in {profiles:?}");
    }
}
