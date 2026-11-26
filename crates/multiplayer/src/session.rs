use axum::extract::ws::{Message, WebSocket};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;
use whisper_common::{AnonymousAccount, MovementPacket};
use whisper_identity::IdentityService;
use whisper_world::WorldManager;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ClientWsMessage {
    Join {
        player_id: Uuid,
        instance_id: String,
        x: f32,
        y: f32,
    },
    Move {
        x: f32,
        y: f32,
        vx: f32,
        vy: f32,
        facing: String,
        state: String,
    },
    Leave,
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum ServerWsMessage {
    Welcome {
        current_players: Vec<MovementPacket>,
        accounts: Vec<AnonymousAccount>,
    },
    PlayerJoined {
        packet: MovementPacket,
        account: AnonymousAccount,
    },
    PlayerMoved {
        packet: MovementPacket,
    },
    PlayerLeft {
        player_id: Uuid,
    },
    Pong,
    Notification {
        text: String,
    },
}

#[derive(Clone)]
pub struct SessionManager {
    world: WorldManager,
    identity: Arc<IdentityService>,
    // Player ID -> Sender channel to their WS
    sessions: Arc<DashMap<Uuid, mpsc::UnboundedSender<ServerWsMessage>>>,
    // Player ID -> current instance_id
    player_instances: Arc<DashMap<Uuid, String>>,
}

impl SessionManager {
    pub fn new(world: WorldManager, identity: Arc<IdentityService>) -> Self {
        Self {
            world,
            identity,
            sessions: Arc::new(DashMap::new()),
            player_instances: Arc::new(DashMap::new()),
        }
    }

    pub fn broadcast_to_instance(&self, instance_id: &str, msg: ServerWsMessage, exclude: Option<Uuid>) {
        let players = self.world.get_players_in_instance(instance_id);
        for p in players {
            if Some(p.player_id) == exclude {
                continue;
            }
            if let Some(tx) = self.sessions.get(&p.player_id) {
                let _ = tx.send(msg.clone());
            }
        }
    }

    pub fn send_to_player(&self, player_id: Uuid, msg: ServerWsMessage) {
        if let Some(tx) = self.sessions.get(&player_id) {
            let _ = tx.send(msg);
        }
    }

    pub async fn handle_socket(self: Arc<Self>, mut socket: WebSocket) {
        let (mut ws_sender, mut ws_receiver) = socket.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<ServerWsMessage>();

        // Outbound task
        let send_task = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Ok(json) = serde_json::to_string(&msg) {
                    if ws_sender.send(Message::Text(json)).await.is_err() {
                        break;
                    }
                }
            }
        });

        let mut current_player_id: Option<Uuid> = None;
        let mut current_instance = "town".to_string();

        while let Some(Ok(msg)) = ws_receiver.next().await {
            match msg {
                Message::Text(text) => {
                    let parsed: std::result::Result<ClientWsMessage, _> = serde_json::from_str(&text);
                    match parsed {
                        Ok(ClientWsMessage::Join { player_id, instance_id, x, y }) => {
                            current_player_id = Some(player_id);
                            current_instance = instance_id.clone();
                            self.sessions.insert(player_id, tx.clone());
                            self.player_instances.insert(player_id, instance_id.clone());

                            self.world.player_enter_instance(player_id, &instance_id, (x, y));

                            // Gather existing players in instance
                            let existing_packets = self.world.get_players_in_instance(&instance_id);
                            let mut accounts = Vec::new();
                            for ep in &existing_packets {
                                if let Ok(acc) = self.identity.get_account(ep.player_id).await {
                                    accounts.push(acc);
                                }
                            }

                            // Send Welcome to joining player
                            let _ = tx.send(ServerWsMessage::Welcome {
                                current_players: existing_packets.clone(),
                                accounts: accounts.clone(),
                            });

                            // Broadcast to others
                            if let Ok(joining_acc) = self.identity.get_account(player_id).await {
                                let initial_packet = MovementPacket {
                                    player_id,
                                    x,
                                    y,
                                    vx: 0.0,
                                    vy: 0.0,
                                    facing: "south".into(),
                                    state: "idle".into(),
                                    instance_id: instance_id.clone(),
                                    timestamp: chrono::Utc::now().timestamp_millis(),
                                };
                                self.broadcast_to_instance(
                                    &instance_id,
                                    ServerWsMessage::PlayerJoined {
                                        packet: initial_packet,
                                        account: joining_acc,
                                    },
                                    Some(player_id),
                                );
                            }
                        }
                        Ok(ClientWsMessage::Move { x, y, vx, vy, facing, state }) => {
                            if let Some(pid) = current_player_id {
                                let packet = MovementPacket {
                                    player_id: pid,
                                    x,
                                    y,
                                    vx,
                                    vy,
                                    facing,
                                    state,
                                    instance_id: current_instance.clone(),
                                    timestamp: chrono::Utc::now().timestamp_millis(),
                                };
                                let _ = self.world.update_movement(packet.clone());
                                self.broadcast_to_instance(
                                    &current_instance,
                                    ServerWsMessage::PlayerMoved { packet },
                                    Some(pid),
                                );
                            }
                        }
                        Ok(ClientWsMessage::Ping) => {
                            let _ = tx.send(ServerWsMessage::Pong);
                        }
                        Ok(ClientWsMessage::Leave) => {
                            break;
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse WS client message: {}", e);
                        }
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }

        // Cleanup on disconnect
        if let Some(pid) = current_player_id {
            self.sessions.remove(&pid);
            self.player_instances.remove(&pid);
            self.world.player_leave(pid);
            self.broadcast_to_instance(
                &current_instance,
                ServerWsMessage::PlayerLeft { player_id: pid },
                None,
            );
        }

        let _ = send_task.abort();
    }
}
