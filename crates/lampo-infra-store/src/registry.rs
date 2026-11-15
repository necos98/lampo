//! Thin registry access for the `Path` environment values. No logic lives here (that is in
//! [`crate::path_list`]); this module only reads/writes the raw value and broadcasts the change.

use lampo_domain::PortResult;

/// Which `Path` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hive {
    /// `HKCU\Environment`.
    User,
    /// `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment`.
    Machine,
}

/// A raw `Path` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathValue {
    pub text: String,
    /// `REG_EXPAND_SZ` (true) or `REG_SZ`; preserved when rewriting.
    pub expandable: bool,
}

#[cfg(windows)]
mod imp {
    use super::{Hive, PathValue};
    use lampo_domain::{PortError, PortResult};
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, REG_EXPAND_SZ, REG_SZ};
    use winreg::{RegKey, RegValue};

    const USER_KEY: &str = "Environment";
    const MACHINE_KEY: &str = "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment";
    const VALUE: &str = "Path";

    fn is_not_found(e: &std::io::Error) -> bool {
        e.kind() == std::io::ErrorKind::NotFound
    }

    pub fn read_path(hive: Hive) -> PortResult<Option<PathValue>> {
        let (root, sub) = match hive {
            Hive::User => (HKEY_CURRENT_USER, USER_KEY),
            Hive::Machine => (HKEY_LOCAL_MACHINE, MACHINE_KEY),
        };
        let context = format!("reading the {hive:?} PATH from the registry");
        let key = match RegKey::predef(root).open_subkey_with_flags(sub, KEY_READ) {
            Ok(k) => k,
            Err(e) if is_not_found(&e) => return Ok(None),
            Err(e) => return Err(PortError::io(context, e)),
        };
        let raw = match key.get_raw_value(VALUE) {
            Ok(v) => v,
            Err(e) if is_not_found(&e) => return Ok(None),
            Err(e) => return Err(PortError::io(context, e)),
        };
        let expandable = raw.vtype == REG_EXPAND_SZ;
        let text = <String as winreg::types::FromRegValue>::from_reg_value(&raw).map_err(|e| PortError::io(context, e))?;
        Ok(Some(PathValue { text, expandable }))
    }

    pub fn write_user_path(value: &PathValue) -> PortResult<()> {
        let context = "writing the user PATH to the registry";
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(USER_KEY, KEY_READ | KEY_SET_VALUE).map_err(|e| PortError::io(context, e))?;
        let mut bytes: Vec<u8> = value.text.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
        bytes.shrink_to_fit();
        let raw = RegValue { bytes, vtype: if value.expandable { REG_EXPAND_SZ } else { REG_SZ } };
        key.set_raw_value(VALUE, &raw).map_err(|e| PortError::io(context, e))
    }

    /// Tells running programs (Explorer, new terminals) that the environment changed.
    pub fn broadcast_environment_change() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE};
        let param: Vec<u16> = "Environment".encode_utf16().chain(std::iter::once(0)).collect();
        let mut result: usize = 0;
        // SAFETY: `param` is a NUL-terminated UTF-16 string that outlives the call; the timeout
        // bounds the time spent waiting for unresponsive windows.
        unsafe {
            SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, 0, param.as_ptr() as isize, SMTO_ABORTIFHUNG, 3000, &mut result);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Hive, PathValue};
    use lampo_domain::{PortError, PortResult};

    pub fn read_path(_hive: Hive) -> PortResult<Option<PathValue>> {
        Err(PortError::Unsupported("the Windows registry is not available on this platform".into()))
    }

    pub fn write_user_path(_value: &PathValue) -> PortResult<()> {
        Err(PortError::Unsupported("the Windows registry is not available on this platform".into()))
    }

    pub fn broadcast_environment_change() {}
}

/// Reads the raw `Path` of a hive (`None` when the value does not exist).
pub fn read_path(hive: Hive) -> PortResult<Option<PathValue>> {
    imp::read_path(hive)
}

pub fn write_user_path(value: &PathValue) -> PortResult<()> {
    imp::write_user_path(value)
}

pub fn broadcast_environment_change() {
    imp::broadcast_environment_change()
}
