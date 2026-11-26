use axum::{
    extract::{Path, Query, State, WebSocketUpgrade},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::{delete, get, post, put},
    Router,
};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::{net::SocketAddr, str::FromStr, sync::Arc, time::Duration};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use whisper_common::{
    AnonymousAccount, AvatarConfig, EncryptedEnvelope, House, HousePrivacyLevel, MessageStatus,
};
use whisper_friends::FriendsService;
use whisper_houses::HousesService;
use whisper_identity::IdentityService;
use whisper_messaging::MessagingService;
use whisper_multiplayer::SessionManager;
use whisper_privacy::PrivacyService;
use whisper_world::WorldManager;

#[derive(Clone)]
pub struct AppState {
    pub identity: Arc<IdentityService>,
    pub friends: Arc<FriendsService>,
    pub houses: Arc<HousesService>,
    pub world: Arc<WorldManager>,
    pub sessions: Arc<SessionManager>,
    pub messaging: Arc<MessagingService>,
    pub privacy: Arc<PrivacyService>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter("info,whisper_gateway=debug,whisper_messaging=debug")
        .init();

    dotenvy::dotenv().ok();

    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite://whisper_world.db?mode=rwc".to_string());

    tracing::info!("Connecting to database: {}", db_url);

    let connect_opts = SqliteConnectOptions::from_str(&db_url)?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

    let pool = SqlitePoolOptions::new()
        .max_connections(20)
        .connect_with(connect_opts)
        .await?;

    // Execute schema migrations
    let schema_sql = include_str!("../../../migrations/0001_initial_schema.sql");
    for statement in schema_sql.split(';') {
        let stmt = statement.trim();
        if !stmt.is_empty() {
            sqlx::query(stmt).execute(&pool).await?;
        }
    }
    tracing::info!("Database schema initialized successfully.");

    let identity = Arc::new(IdentityService::new(pool.clone()));
    let friends = Arc::new(FriendsService::new(pool.clone()));
    let houses = Arc::new(HousesService::new(pool.clone(), (*friends).clone()));
    let messaging = Arc::new(MessagingService::new(pool.clone(), (*friends).clone()));
    let privacy = Arc::new(PrivacyService::new(pool.clone()));
    let world = Arc::new(WorldManager::new());
    let sessions = Arc::new(SessionManager::new((*world).clone(), identity.clone()));

    let state = AppState {
        identity,
        friends,
        houses,
        world,
        sessions,
        messaging: messaging.clone(),
        privacy,
    };

    // Background task to purge expired pending messages
    let msg_service_bg = messaging.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if let Ok(count) = msg_service_bg.purge_expired().await {
                if count > 0 {
                    tracing::info!("Purged {} expired pending messages", count);
                }
            }
        }
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        // Health
        .route("/api/v1/health", get(health_check))
        // Auth / Identity
        .route("/api/v1/auth/register", post(register_account))
        .route("/api/v1/auth/:id", get(get_account))
        .route("/api/v1/auth/:id/avatar", put(update_avatar))
        .route("/api/v1/auth/:id/name", put(update_name))
        // Friends
        .route("/api/v1/friends/invite", post(create_invite))
        .route("/api/v1/friends/redeem", post(redeem_invite))
        .route("/api/v1/friends/:user_id", get(list_friends))
        .route("/api/v1/friends/:user_id/:friend_id", delete(remove_friend))
        .route("/api/v1/friends/:user_id/block/:target_id", post(block_user))
        // Houses
        .route("/api/v1/houses/:owner_id", get(get_house))
        .route("/api/v1/houses/:owner_id/can-visit", get(can_visit_house))
        .route("/api/v1/houses/:owner_id/privacy", put(update_house_privacy))
        .route("/api/v1/houses/:owner_id/customization", put(update_house_customization))
        // Messaging (E2E Encrypted Store-and-Forward + Atomic Burn)
        .route("/api/v1/messages/send", post(send_message))
        .route("/api/v1/messages/pending/:recipient_id", get(get_pending_messages))
        .route("/api/v1/messages/burn", post(burn_message))
        .route("/api/v1/messages/:id/status", get(get_message_status))
        // Safety / Abuse Reporting
        .route("/api/v1/safety/report", post(submit_abuse_report))
        // WebSocket Multiplayer Sync
        .route("/ws", get(ws_handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = SocketAddr::from(([0, 0, 0, 0], port.parse::<u16>()?));
    tracing::info!("Whisper World Server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "service": "whisper-world",
        "version": "1.0.0",
        "features": {
            "multiplayer": true,
            "e2ee_speech_balloon": true,
            "burn_after_reading": true
        }
    }))
}

#[derive(Deserialize)]
struct RegisterRequest {
    pub public_key: String,
    pub dh_public_key: String,
    pub display_name: String,
    pub avatar_config: AvatarConfig,
    pub recovery_hash: Option<String>,
}

async fn register_account(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> std::result::Result<Json<AnonymousAccount>, (StatusCode, String)> {
    state
        .identity
        .register(
            req.public_key,
            req.dh_public_key,
            req.display_name,
            req.avatar_config,
            req.recovery_hash,
        )
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn get_account(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> std::result::Result<Json<AnonymousAccount>, (StatusCode, String)> {
    state
        .identity
        .get_account(id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

#[derive(Deserialize)]
struct UpdateAvatarRequest {
    pub avatar_config: AvatarConfig,
}

async fn update_avatar(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateAvatarRequest>,
) -> std::result::Result<StatusCode, (StatusCode, String)> {
    state
        .identity
        .update_avatar(id, req.avatar_config)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Deserialize)]
struct UpdateNameRequest {
    pub display_name: String,
}

async fn update_name(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateNameRequest>,
) -> std::result::Result<StatusCode, (StatusCode, String)> {
    state
        .identity
        .update_display_name(id, &req.display_name)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Deserialize)]
struct InviteRequest {
    pub creator_id: Uuid,
    pub creator_public_key: String,
    pub signature: String,
}

async fn create_invite(
    State(state): State<AppState>,
    Json(req): Json<InviteRequest>,
) -> std::result::Result<Json<whisper_common::FriendInvitation>, (StatusCode, String)> {
    state
        .friends
        .create_invitation(req.creator_id, &req.creator_public_key, &req.signature)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Deserialize)]
struct RedeemInviteRequest {
    pub user_id: Uuid,
    pub code: String,
}

async fn redeem_invite(
    State(state): State<AppState>,
    Json(req): Json<RedeemInviteRequest>,
) -> std::result::Result<Json<AnonymousAccount>, (StatusCode, String)> {
    state
        .friends
        .redeem_invitation(req.user_id, &req.code)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn list_friends(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
) -> std::result::Result<Json<Vec<AnonymousAccount>>, (StatusCode, String)> {
    state
        .friends
        .list_friends(user_id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn remove_friend(
    State(state): State<AppState>,
    Path((user_id, friend_id)): Path<(Uuid, Uuid)>,
) -> std::result::Result<StatusCode, (StatusCode, String)> {
    state
        .friends
        .remove_friend(user_id, friend_id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn block_user(
    State(state): State<AppState>,
    Path((user_id, target_id)): Path<(Uuid, Uuid)>,
) -> std::result::Result<StatusCode, (StatusCode, String)> {
    state
        .friends
        .block_user(user_id, target_id)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

async fn get_house(
    State(state): State<AppState>,
    Path(owner_id): Path<Uuid>,
) -> std::result::Result<Json<House>, (StatusCode, String)> {
    state
        .houses
        .get_by_owner(owner_id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

#[derive(Deserialize)]
struct CanVisitQuery {
    pub visitor_id: Uuid,
}

async fn can_visit_house(
    State(state): State<AppState>,
    Path(owner_id): Path<Uuid>,
    Query(query): Query<CanVisitQuery>,
) -> std::result::Result<Json<serde_json::Value>, (StatusCode, String)> {
    let allowed = state
        .houses
        .can_visit(query.visitor_id, owner_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({ "allowed": allowed })))
}

#[derive(Deserialize)]
struct UpdatePrivacyRequest {
    pub privacy: HousePrivacyLevel,
}

async fn update_house_privacy(
    State(state): State<AppState>,
    Path(owner_id): Path<Uuid>,
    Json(req): Json<UpdatePrivacyRequest>,
) -> std::result::Result<StatusCode, (StatusCode, String)> {
    state
        .houses
        .update_privacy(owner_id, req.privacy)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Deserialize)]
struct UpdateCustomizationRequest {
    pub customization: serde_json::Value,
}

async fn update_house_customization(
    State(state): State<AppState>,
    Path(owner_id): Path<Uuid>,
    Json(req): Json<UpdateCustomizationRequest>,
) -> std::result::Result<StatusCode, (StatusCode, String)> {
    state
        .houses
        .update_customization(owner_id, req.customization)
        .await
        .map(|_| StatusCode::NO_CONTENT)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))
}

#[derive(Deserialize)]
struct SendMessageRequest {
    pub sender_id: Uuid,
    pub sender_public_key: String,
    pub recipient_id: Uuid,
    pub ephemeral_public_key: String,
    pub nonce: String,
    pub ciphertext: String,
    pub sender_signature: String,
    pub burn_token_hash: String,
}

async fn send_message(
    State(state): State<AppState>,
    Json(req): Json<SendMessageRequest>,
) -> std::result::Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Check rate limit
    state
        .privacy
        .check_rate_limit(req.sender_id, 30)
        .map_err(|e| (StatusCode::TOO_MANY_REQUESTS, e.to_string()))?;

    let message_id = state
        .messaging
        .submit_envelope(
            req.sender_id,
            &req.sender_public_key,
            req.recipient_id,
            req.ephemeral_public_key,
            req.nonce,
            req.ciphertext,
            req.sender_signature,
            req.burn_token_hash,
        )
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // If recipient is currently online in their house, send instant subtle notification!
    state.sessions.send_to_player(
        req.recipient_id,
        whisper_multiplayer::ServerWsMessage::Notification {
            text: "You have a new message.".into(),
        },
    );

    Ok(Json(serde_json::json!({
        "message_id": message_id,
        "status": "QUEUED"
    })))
}

async fn get_pending_messages(
    State(state): State<AppState>,
    Path(recipient_id): Path<Uuid>,
) -> std::result::Result<Json<Vec<EncryptedEnvelope>>, (StatusCode, String)> {
    state
        .messaging
        .get_pending_messages(recipient_id)
        .await
        .map(Json)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

#[derive(Deserialize)]
struct BurnRequest {
    pub recipient_id: Uuid,
    pub message_id: Uuid,
    pub burn_token: String,
}

async fn burn_message(
    State(state): State<AppState>,
    Json(req): Json<BurnRequest>,
) -> std::result::Result<Json<serde_json::Value>, (StatusCode, String)> {
    state
        .messaging
        .burn_message(req.recipient_id, req.message_id, &req.burn_token)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "status": "BURNED",
        "message": "Message read and permanently deleted."
    })))
}

async fn get_message_status(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> std::result::Result<Json<serde_json::Value>, (StatusCode, String)> {
    let status = state
        .messaging
        .get_status(id)
        .unwrap_or(MessageStatus::Burned);

    Ok(Json(serde_json::json!({
        "message_id": id,
        "status": status
    })))
}

#[derive(Deserialize)]
struct AbuseReportRequest {
    pub reporter_id: Uuid,
    pub reported_id: Uuid,
    pub reason: String,
    pub evidence_ciphertext: Option<String>,
}

async fn submit_abuse_report(
    State(state): State<AppState>,
    Json(req): Json<AbuseReportRequest>,
) -> std::result::Result<StatusCode, (StatusCode, String)> {
    state
        .privacy
        .submit_report(
            req.reporter_id,
            req.reported_id,
            &req.reason,
            req.evidence_ciphertext.as_deref(),
        )
        .await
        .map(|_| StatusCode::CREATED)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| state.sessions.handle_socket(socket))
}
