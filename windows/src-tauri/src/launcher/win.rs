//! The Win32 of the launcher: the registry lookup of URL protocols.

use ::windows::core::HSTRING;
use ::windows::Win32::Foundation::ERROR_SUCCESS;
use ::windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CLASSES_ROOT, KEY_READ,
};

/// True when `scheme` is a registered URL protocol: `HKEY_CLASSES_ROOT\<scheme>` (HKCU and HKLM
/// classes merged) has a `URL Protocol` value. Only the presence of that value is read.
pub fn protocol_registered(scheme: &str) -> bool {
    if scheme.is_empty() || scheme.contains(['\\', '/']) {
        return false;
    }
    let mut key = HKEY::default();
    unsafe {
        if RegOpenKeyExW(HKEY_CLASSES_ROOT, &HSTRING::from(scheme), Some(0), KEY_READ, &mut key) != ERROR_SUCCESS {
            return false;
        }
        let found = RegQueryValueExW(key, &HSTRING::from("URL Protocol"), None, None, None, None) == ERROR_SUCCESS;
        let _ = RegCloseKey(key);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registered_protocol_is_found_and_an_unknown_one_is_not() {
        // `http` is registered on every Windows; the second name exists nowhere.
        assert!(protocol_registered("http"));
        assert!(!protocol_registered("mayak-no-such-protocol"));
        assert!(!protocol_registered(""), "the root of the classes is no protocol");
        assert!(!protocol_registered(r"http\shell"), "a path is no scheme");
    }
}
