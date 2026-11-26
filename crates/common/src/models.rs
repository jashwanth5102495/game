use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarConfig {
    pub skin_tone: u8,
    pub hair_style: u8,
    pub hair_color: u8,
    pub shirt_style: u8,
    pub shirt_color: u8,
    pub pants_style: u8,
    pub pants_color: u8,
    pub shoes_style: u8,
    pub shoes_color: u8,
    pub accessory: u8,
}

impl Default for AvatarConfig {
    fn default() -> Self {
        Self {
            skin_tone: 0,
            hair_style: 1,
            hair_color: 2,
            shirt_style: 1,
            shirt_color: 3,
            pants_style: 1,
            pants_color: 1,
            shoes_style: 0,
            shoes_color: 0,
            accessory: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnonymousAccount {
    pub id: Uuid,
    pub public_key: String,       // Ed25519 public key (Base64)
    pub dh_public_key: String,    // X25519 public key (Base64)
    pub display_name: String,
    pub avatar_config: AvatarConfig,
    pub recovery_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HousePrivacyLevel {
    Public,
    FriendsOnly,
    Private,
}

impl Default for HousePrivacyLevel {
    fn default() -> Self {
        Self::FriendsOnly
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct House {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub privacy_level: HousePrivacyLevel,
    pub customization: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FriendshipStatus {
    Pending,
    Accepted,
    Rejected,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendRelationship {
    pub user_id: Uuid,
    pub friend_id: Uuid,
    pub status: FriendshipStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendInvitation {
    pub code: String,
    pub creator_id: Uuid,
    pub expires_at: DateTime<Utc>,
    pub signature: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MessageStatus {
    Queued,
    Delivered,
    Opened,
    Burned,
    Expired,
}

/// Encrypted message envelope stored ephemerally on the server.
/// Notice: The server NEVER has access to the plaintext or decryption key!
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedEnvelope {
    pub message_id: Uuid,
    pub sender_id: Uuid,
    pub recipient_id: Uuid,
    pub ephemeral_public_key: String, // X25519 ephemeral key (Base64)
    pub nonce: String,                // 12-byte nonce (Base64)
    pub ciphertext: String,           // ChaCha20-Poly1305 ciphertext + tag (Base64)
    pub sender_signature: String,     // Ed25519 signature over (message_id + recipient_id + ciphertext)
    pub burn_token_hash: String,      // SHA-256 hash of recipient's one-time burn authorization token
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub status: MessageStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MovementPacket {
    pub player_id: Uuid,
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub facing: String, // "south", "north", "east", "west"
    pub state: String,  // "idle", "walk", "run"
    pub instance_id: String, // "town", "house:<owner_uuid>"
    pub timestamp: i64,
}
