use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use crate::errors::{AppError, Result};

pub fn hash_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    BASE64.encode(result)
}

pub fn verify_signature(public_key_b64: &str, message: &[u8], signature_b64: &str) -> Result<bool> {
    let pubkey_bytes = BASE64.decode(public_key_b64)
        .map_err(|e| AppError::Crypto(format!("Invalid base64 public key: {}", e)))?;
    
    let key_bytes: [u8; 32] = pubkey_bytes.try_into()
        .map_err(|_| AppError::Crypto("Public key must be 32 bytes".into()))?;

    let verifying_key = VerifyingKey::from_bytes(&key_bytes)
        .map_err(|e| AppError::Crypto(format!("Invalid verifying key: {}", e)))?;

    let sig_bytes = BASE64.decode(signature_b64)
        .map_err(|e| AppError::Crypto(format!("Invalid base64 signature: {}", e)))?;

    let sig_arr: [u8; 64] = sig_bytes.try_into()
        .map_err(|_| AppError::Crypto("Signature must be 64 bytes".into()))?;

    let signature = Signature::from_bytes(&sig_arr);

    match verifying_key.verify_strict(message, &signature) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}
