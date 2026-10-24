//! Device-local HMAC-SHA256 payload signing for Dosi-Tracker Linux.
//!
//! Provides cryptographic tamper detection for activity envelopes queued in the local outbox.
//! Mirrored from the Windows client signing implementation:
//! - HMAC-SHA256 calculated over canonical UTF-8 JSON bytes of the activity payload.
//! - Encoded as base64url without padding (`URL_SAFE_NO_PAD`).
//! - Computed before the `payloadSignature` field is added so verifying the signature matches
//!   the exact bytes originally committed.

use base64::Engine as _;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;

use crate::outbox::ActivityEnvelope;

type HmacSha256 = Hmac<Sha256>;

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

    proptest! {
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
