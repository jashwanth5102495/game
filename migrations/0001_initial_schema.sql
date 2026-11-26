-- Anonymous Accounts
CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    public_key TEXT NOT NULL UNIQUE,
    dh_public_key TEXT NOT NULL,
    display_name TEXT NOT NULL,
    avatar_config TEXT NOT NULL,
    recovery_hash TEXT,
    created_at TIMESTAMP NOT NULL,
    last_seen_at TIMESTAMP NOT NULL
);

-- Houses
CREATE TABLE IF NOT EXISTS houses (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL UNIQUE,
    privacy_level TEXT NOT NULL DEFAULT 'FRIENDS_ONLY',
    customization TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL,
    FOREIGN KEY(owner_id) REFERENCES accounts(id) ON DELETE CASCADE
);

-- Friend Relationships (Bidirectional)
CREATE TABLE IF NOT EXISTS friend_relationships (
    user_id TEXT NOT NULL,
    friend_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'PENDING',
    created_at TIMESTAMP NOT NULL,
    PRIMARY KEY(user_id, friend_id),
    FOREIGN KEY(user_id) REFERENCES accounts(id) ON DELETE CASCADE,
    FOREIGN KEY(friend_id) REFERENCES accounts(id) ON DELETE CASCADE
);

-- Friend Invitations (Ephemeral QR / One-time codes)
CREATE TABLE IF NOT EXISTS friend_invitations (
    code TEXT PRIMARY KEY,
    creator_id TEXT NOT NULL,
    expires_at TIMESTAMP NOT NULL,
    signature TEXT NOT NULL,
    FOREIGN KEY(creator_id) REFERENCES accounts(id) ON DELETE CASCADE
);

-- Block Relationships
CREATE TABLE IF NOT EXISTS block_relationships (
    user_id TEXT NOT NULL,
    blocked_id TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL,
    PRIMARY KEY(user_id, blocked_id),
    FOREIGN KEY(user_id) REFERENCES accounts(id) ON DELETE CASCADE,
    FOREIGN KEY(blocked_id) REFERENCES accounts(id) ON DELETE CASCADE
);

-- Ephemeral Pending Messages (End-to-End Encrypted Ciphertext ONLY)
-- Notice: NO readable plaintext or secret keys are EVER stored here.
-- As soon as the recipient acknowledges reading, this record is ATOMICALLY DELETED.
CREATE TABLE IF NOT EXISTS pending_messages (
    id TEXT PRIMARY KEY,
    sender_id TEXT NOT NULL,
    recipient_id TEXT NOT NULL,
    ephemeral_public_key TEXT NOT NULL,
    nonce TEXT NOT NULL,
    ciphertext TEXT NOT NULL,
    sender_signature TEXT NOT NULL,
    burn_token_hash TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL,
    expires_at TIMESTAMP NOT NULL,
    status TEXT NOT NULL DEFAULT 'QUEUED',
    FOREIGN KEY(sender_id) REFERENCES accounts(id) ON DELETE CASCADE,
    FOREIGN KEY(recipient_id) REFERENCES accounts(id) ON DELETE CASCADE
);

-- Safety & Abuse Reports (Consensual User Submissions)
CREATE TABLE IF NOT EXISTS abuse_reports (
    id TEXT PRIMARY KEY,
    reporter_id TEXT NOT NULL,
    reported_id TEXT NOT NULL,
    reason TEXT NOT NULL,
    evidence_ciphertext TEXT,
    created_at TIMESTAMP NOT NULL
);

-- Indexes for lightning fast lookups
CREATE INDEX IF NOT EXISTS idx_accounts_pubkey ON accounts(public_key);
CREATE INDEX IF NOT EXISTS idx_houses_owner ON houses(owner_id);
CREATE INDEX IF NOT EXISTS idx_friends_user ON friend_relationships(user_id, status);
CREATE INDEX IF NOT EXISTS idx_pending_recipient ON pending_messages(recipient_id, expires_at);
