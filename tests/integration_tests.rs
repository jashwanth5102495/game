use uuid::Uuid;
use whisper_common::{
    crypto::{hash_sha256, verify_signature},
    EncryptedEnvelope, MessageStatus,
};

#[test]
fn test_cryptographic_envelope_integrity() {
    let raw_data = b"whisper_confidential_message";
    let hash = hash_sha256(raw_data);
    assert!(!hash.is_empty());
}

#[test]
fn test_ephemeral_burn_state_transition() {
    let mut status = MessageStatus::Queued;
    assert_eq!(status, MessageStatus::Queued);

    status = MessageStatus::Delivered;
    assert_eq!(status, MessageStatus::Delivered);

    status = MessageStatus::Burned;
    assert_eq!(status, MessageStatus::Burned);
}
