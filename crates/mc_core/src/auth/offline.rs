//! Offline / local account helpers.

use md5::{Digest, Md5};

/// Reproduce Java's `UUID.nameUUIDFromBytes` (a version-3, MD5-based UUID).
///
/// Mojang computes offline UUIDs as `nameUUIDFromBytes("OfflinePlayer:" + name)`,
/// so matching this exactly lets offline profiles keep a stable identity that
/// is compatible with vanilla servers running in `online-mode=false`.
pub fn offline_uuid(username: &str) -> String {
    let input = format!("OfflinePlayer:{username}");
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    let mut bytes: [u8; 16] = hasher.finalize().into();

    // Set version (3 => MD5) and IETF variant bits, mirroring the JDK.
    bytes[6] = (bytes[6] & 0x0f) | 0x30;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

/// Validate that a username is acceptable for an offline account.
///
/// Mirrors Minecraft's 3–16 character, `[A-Za-z0-9_]` rule.
pub fn valid_username(username: &str) -> bool {
    let len = username.chars().count();
    (3..=16).contains(&len)
        && username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_matches_known_java_value() {
        // Reference value produced by the JDK for "OfflinePlayer:Notch".
        assert_eq!(
            offline_uuid("Notch"),
            "b50ad385-829d-3141-a216-7e7d7539ba7f"
        );
    }

    #[test]
    fn uuid_is_deterministic() {
        assert_eq!(offline_uuid("Steve"), offline_uuid("Steve"));
        assert_ne!(offline_uuid("Steve"), offline_uuid("Alex"));
    }

    #[test]
    fn version_and_variant_bits() {
        let uuid = offline_uuid("Player");
        let chars: Vec<char> = uuid.chars().collect();
        // Third group starts with '3' for version 3.
        assert_eq!(chars[14], '3');
        // Variant nibble must be 8, 9, a or b.
        assert!(matches!(chars[19], '8' | '9' | 'a' | 'b'));
    }

    #[test]
    fn username_validation() {
        assert!(valid_username("Steve"));
        assert!(valid_username("Player_1"));
        assert!(!valid_username("ab"));
        assert!(!valid_username("this_name_is_far_too_long"));
        assert!(!valid_username("bad name"));
    }
}
