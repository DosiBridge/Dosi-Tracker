//! Device-local HMAC-SHA256 payload signing for Dosi-Tracker Linux.
//!
//! Provides cryptographic tamper detection for activity envelopes queued in the local outbox.
//! Mirrored from the Windows client signing implementation:
//! - HMAC-SHA256 calculated over canonical UTF-8 JSON bytes of the activity payload.
//! - Encoded as base64url without padding (`URL_SAFE_NO_PAD`).
//! - Computed before the `payloadSignature` field is added so verifying the signature matches
//!   the exact bytes originally committed.
//!
//! Outbox v2 rows ([`sign_v2`]) bind the payload bytes as stored and the SHA-256 digests of
//! the row's captures, so neither the payload nor a capture can be swapped without the row
//! failing its local integrity check. The tag is local tamper evidence only: the server
//! does not read `payloadSignature`.

use base64::Engine as _;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::outbox::ActivityEnvelope;

type HmacSha256 = Hmac<Sha256>;

/// SHA-256 digests of the captures queued with a v2 row, by kind. A kind the
/// row never had is `None`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureDigests {
    pub screen: Option<[u8; 32]>,
    pub thumb: Option<[u8; 32]>,
}

/// SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// The v2 signing input: the payload bytes, a `0x00` separator (JSON text
/// never contains a raw NUL), then one slot per capture kind in a fixed
/// order (screen, thumb): `0x01` followed by the 32-byte digest, or `0x00`
/// when the row has no capture of that kind. The slots keep "screen only"
/// and "thumb only" apart.
fn v2_mac(key: &[u8], payload: &[u8], digests: &CaptureDigests) -> HmacSha256 {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(payload);
    mac.update(&[0x00]);
    for slot in [digests.screen, digests.thumb] {
        match slot {
            Some(digest) => {
                mac.update(&[0x01]);
                mac.update(&digest);
            }
            None => mac.update(&[0x00]),
        }
    }
    mac
}

/// Sign a v2 outbox row: HMAC-SHA256 over the payload bytes as stored and
/// the digests of its captures, base64url without padding.
pub fn sign_v2(key: &[u8], payload: &[u8], digests: &CaptureDigests) -> String {
    let tag = v2_mac(key, payload, digests).finalize().into_bytes();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(tag)
}

/// Verify a v2 row's tag in constant time. A malformed tag never verifies.
pub fn verify_v2(key: &[u8], payload: &[u8], digests: &CaptureDigests, sig: &str) -> bool {
    let Ok(tag) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(sig) else {
        return false;
    };
    v2_mac(key, payload, digests).verify_slice(&tag).is_ok()
}

/// Generates a cryptographically secure 32-byte device signing key.
pub fn generate_signing_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    key
}

/// Sign `payload_json` (the canonical JSON bytes) using the device key and
/// return a base64url-encoded HMAC-SHA256 tag without padding.
pub fn sign_payload(key: &[u8], payload_json: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(payload_json);
    let tag = mac.finalize().into_bytes();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(tag)
}

/// Verify that `sig` is the valid HMAC-SHA256 base64url tag for `payload_json` under `key`.
pub fn verify_payload(key: &[u8], payload_json: &[u8], sig: &str) -> bool {
    let Ok(tag) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(sig) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(key) else {
        return false;
    };
    mac.update(payload_json);
    mac.verify_slice(&tag).is_ok()
}

/// Signs an [`ActivityEnvelope`] by computing the HMAC tag over its JSON representation
/// without the `payload_signature` field, and then stamping `payload_signature` onto the envelope.
pub fn sign_activity_envelope(
    key: &[u8],
    envelope: &mut ActivityEnvelope,
) -> Result<String, serde_json::Error> {
    envelope.payload_signature = None;
    let canonical_json = serde_json::to_vec(envelope)?;
    let tag = sign_payload(key, &canonical_json);
    envelope.payload_signature = Some(tag.clone());
    Ok(tag)
}

/// Verifies whether the envelope's stamped `payload_signature` matches the canonical JSON bytes.
pub fn verify_activity_envelope(key: &[u8], envelope: &ActivityEnvelope) -> bool {
    let Some(ref sig) = envelope.payload_signature else {
        return false;
    };
    let mut stripped = envelope.clone();
    stripped.payload_signature = None;
    let Ok(canonical_json) = serde_json::to_vec(&stripped) else {
        return false;
    };
    verify_payload(key, &canonical_json, sig)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use proptest::prelude::*;

    fn test_key() -> Vec<u8> {
        (0u8..32).collect()
    }

    #[test]
    fn test_sign_and_verify_roundtrip_windows_vector() {
        let key = test_key();
        let payload = br#"{"clientActivityId":"abc","projectId":"p1"}"#;
        let sig = sign_payload(&key, payload);
        assert!(verify_payload(&key, payload, &sig));
    }

    #[test]
    fn test_different_key_fails_verification() {
        let key = test_key();
        let other_key: Vec<u8> = (1u8..33).collect();
        let payload = br#"{"clientActivityId":"abc"}"#;
        let sig = sign_payload(&key, payload);
        assert!(!verify_payload(&other_key, payload, &sig));
    }

    #[test]
    fn test_modified_payload_fails_verification() {
        let key = test_key();
        let payload = br#"{"clientActivityId":"abc","mouseClicks":5}"#;
        let sig = sign_payload(&key, payload);
        let tampered = br#"{"clientActivityId":"abc","mouseClicks":999}"#;
        assert!(!verify_payload(&key, tampered, &sig));
    }

    #[test]
    fn test_garbage_and_empty_sig_fails_verification() {
        let key = test_key();
        let payload = br#"{"clientActivityId":"abc"}"#;
        assert!(!verify_payload(&key, payload, "not-a-valid-sig"));
        assert!(!verify_payload(&key, payload, ""));
        assert!(!verify_payload(&key, payload, "==bad-base64=="));
    }

    #[test]
    fn test_generate_signing_key_returns_32_distinct_bytes() {
        let k1 = generate_signing_key();
        let k2 = generate_signing_key();
        assert_eq!(k1.len(), 32);
        assert_eq!(k2.len(), 32);
        assert_ne!(k1, k2, "Generated keys must be distinct random sequences");
    }

    #[test]
    fn test_sign_and_verify_envelope_roundtrip() {
        let key = generate_signing_key();
        let mut envelope = ActivityEnvelope {
            format_version: ActivityEnvelope::CURRENT_FORMAT_VERSION,
            client_activity_id: "test-act-001".into(),
            project_id: Some("proj-alpha".into()),
            started_at: Utc::now(),
            ended_at: Utc::now(),
            productivity: Some(85),
            mouse_clicks: Some(120),
            keyboard_hits: Some(450),
            description: Some("Working on Phase 1 Linux parity".into()),
            device_id: Some("dev-linux-001".into()),
            platform: "linux".into(),
            agent_version: "0.1.0".into(),
            payload_signature: None,
        };

        let sig = sign_activity_envelope(&key, &mut envelope).expect("signing must succeed");
        assert_eq!(envelope.payload_signature.as_deref(), Some(sig.as_str()));
        assert!(verify_activity_envelope(&key, &envelope));

        // Tamper with clicks
        envelope.mouse_clicks = Some(9999);
        assert!(!verify_activity_envelope(&key, &envelope));
    }

    #[test]
    fn v2_signatures_bind_the_payload_and_each_capture_slot() {
        let key = test_key();
        let payload = br#"{"clientActivityId":"abc","projectId":"p1"}"#;
        let screen = sha256(b"screen-jpeg");
        let thumb = sha256(b"thumb-jpeg");
        let both = CaptureDigests {
            screen: Some(screen),
            thumb: Some(thumb),
        };
        let sig = sign_v2(&key, payload, &both);
        assert!(verify_v2(&key, payload, &both, &sig));

        // Another key, another payload, or any change to the captures fails.
        let other_key: Vec<u8> = (1u8..33).collect();
        assert!(!verify_v2(&other_key, payload, &both, &sig));
        assert!(!verify_v2(
            &key,
            br#"{"clientActivityId":"abd"}"#,
            &both,
            &sig
        ));
        let swapped = CaptureDigests {
            screen: Some(thumb),
            thumb: Some(screen),
        };
        assert!(!verify_v2(&key, payload, &swapped, &sig));
        let dropped = CaptureDigests {
            screen: Some(screen),
            thumb: None,
        };
        assert!(!verify_v2(&key, payload, &dropped, &sig));

        // "Screen only" and "thumb only" with the same digest are different inputs.
        let screen_only = CaptureDigests {
            screen: Some(screen),
            thumb: None,
        };
        let thumb_only = CaptureDigests {
            screen: None,
            thumb: Some(screen),
        };
        assert_ne!(
            sign_v2(&key, payload, &screen_only),
            sign_v2(&key, payload, &thumb_only)
        );

        // Without captures the tag differs from the v1 tag of the same bytes.
        let none = CaptureDigests::default();
        assert_ne!(sign_v2(&key, payload, &none), sign_payload(&key, payload));
        assert!(!verify_v2(&key, payload, &none, "not base64!"));
        assert!(!verify_v2(&key, payload, &none, ""));
    }

    #[test]
    fn sha256_matches_the_known_vector() {
        let digest = sha256(b"abc");
        assert_eq!(
            digest[..4],
            [0xba, 0x78, 0x16, 0xbf],
            "SHA-256(\"abc\") starts with ba7816bf"
        );
    }

    proptest! {
        #[test]
        fn prop_v2_sign_verify_arbitrary_payloads(
            key_bytes in prop::collection::vec(any::<u8>(), 16..64),
            payload in prop::collection::vec(any::<u8>(), 1..1024),
            screen in prop::option::of(any::<[u8; 32]>()),
            thumb in prop::option::of(any::<[u8; 32]>()),
        ) {
            let digests = CaptureDigests { screen, thumb };
            let sig = sign_v2(&key_bytes, &payload, &digests);
            prop_assert!(verify_v2(&key_bytes, &payload, &digests, &sig));
            let mut mutated = payload.clone();
            mutated[0] ^= 0xFF;
            prop_assert!(!verify_v2(&key_bytes, &mutated, &digests, &sig));
        }

        #[test]
        fn prop_sign_verify_arbitrary_payloads(
            key_bytes in prop::collection::vec(any::<u8>(), 16..64),
            payload in prop::collection::vec(any::<u8>(), 0..1024),
        ) {
            let sig = sign_payload(&key_bytes, &payload);
            prop_assert!(verify_payload(&key_bytes, &payload, &sig));
            if !payload.is_empty() {
                let mut mutated = payload.clone();
                mutated[0] ^= 0xFF;
                prop_assert!(!verify_payload(&key_bytes, &mutated, &sig));
            }
        }
    }
}
