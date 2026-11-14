//! Friendly messages for keyring (system secret storage) errors.
//!
//! The app keeps the Pal's pairing token and calendar sign-ins in the
//! operating system's keyring. On Linux that is a Secret Service such as
//! gnome-keyring or KWallet; minimal systems (ChromeOS Linux, some distros,
//! WSL) may have none installed, or one with no default keyring created yet.
//! The raw errors ("DBus error: the name org.freedesktop.secrets was not
//! provided by any .service files", "no result found") mean nothing to an
//! owner, so they are turned into a message that says what to do.

/// What to show when a secret can't be saved or read.
pub fn keyring_error(e: keyring::Error) -> String {
    match e {
        keyring::Error::PlatformFailure(_) | keyring::Error::NoStorageAccess(_) => {
            format!("{} (Details: {e})", no_keyring_advice())
        }
        other => other.to_string(),
    }
}

#[cfg(target_os = "linux")]
fn no_keyring_advice() -> &'static str {
    "CubiPal couldn't use your system keyring, where it keeps your pairing and \
     calendar sign-ins safe. Install a keyring (for example run \
     `sudo apt install gnome-keyring`), make sure it is unlocked, then restart \
     CubiPal and try again."
}

#[cfg(not(target_os = "linux"))]
fn no_keyring_advice() -> &'static str {
    "CubiPal couldn't use your system's secure storage, where it keeps your \
     pairing and calendar sign-ins. Make sure you are signed in to your \
     computer account, then restart CubiPal and try again."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_failure_gets_advice_and_keeps_details() {
        let msg = keyring_error(keyring::Error::PlatformFailure("no result found".into()));
        assert!(msg.contains("keyring") || msg.contains("secure storage"));
        assert!(msg.contains("no result found"));
    }

    #[test]
    fn other_errors_pass_through() {
        let msg = keyring_error(keyring::Error::NoEntry);
        assert_eq!(msg, keyring::Error::NoEntry.to_string());
    }
}
