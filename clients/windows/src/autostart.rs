//! "Start with Windows" via the per-user Run key.
//!
//! HKCU\Software\Microsoft\Windows\CurrentVersion\Run is used (not HKLM) so the
//! agent installs and toggles without administrator rights, and runs in the
//! signed-in user's session — which is required for screen/input capture.

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const VALUE_NAME: &str = "DosiTracker";

/// Is the agent currently registered to start at login?
#[cfg(windows)]
pub fn is_enabled() -> bool {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    };

    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            0,
            KEY_READ,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return false;
        }

        let mut size: u32 = 0;
        let status = RegQueryValueExW(
            key,
            &HSTRING::from(VALUE_NAME),
            None,
            None,
            None,
            Some(&mut size),
        );
        let _ = RegCloseKey(key);
        status == ERROR_SUCCESS
    }
}

/// Register or unregister the agent for start-at-login.
#[cfg(windows)]
pub fn set_enabled(enabled: bool) -> anyhow::Result<()> {
    use anyhow::anyhow;
    use windows::core::HSTRING;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_WRITE, REG_SZ,
    };

    // Quote the path so a Program Files install (spaces) still launches correctly.
    let exe = std::env::current_exe()?;
    let command = format!("\"{}\"", exe.display());

    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            0,
            KEY_WRITE,
            &mut key,
        ) != ERROR_SUCCESS
        {
            return Err(anyhow!("could not open the Run registry key for writing"));
        }

        let status = if enabled {
            // REG_SZ payload must be a NUL-terminated UTF-16 byte blob.
            let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2);
            RegSetValueExW(key, &HSTRING::from(VALUE_NAME), 0, REG_SZ, Some(bytes))
        } else {
            let status = RegDeleteValueW(key, &HSTRING::from(VALUE_NAME));
            // Deleting something that was never there is success for our purposes.
            if status == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND {
                ERROR_SUCCESS
            } else {
                status
            }
        };

        let _ = RegCloseKey(key);

        if status != ERROR_SUCCESS {
            return Err(anyhow!("failed to update the Run registry key: {status:?}"));
        }
    }

    Ok(())
}

#[cfg(not(windows))]
pub fn is_enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn set_enabled(_enabled: bool) -> anyhow::Result<()> {
    Ok(())
}
