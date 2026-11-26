use dashmap::DashMap;
use std::sync::Arc;
use uuid::Uuid;
use whisper_common::{MovementPacket, Result};

pub const TOWN_INSTANCE: &str = "town";

#[derive(Clone)]
pub struct WorldManager {
    // Maps player_id -> current MovementPacket
    players: Arc<DashMap<Uuid, MovementPacket>>,
    // Maps instance_id -> Set of player_ids
    instances: Arc<DashMap<String, Vec<Uuid>>>,
}

impl WorldManager {
    pub fn new() -> Self {
        Self {
            players: Arc::new(DashMap::new()),
            instances: Arc::new(DashMap::new()),
        }
    }

    pub fn player_enter_instance(&self, player_id: Uuid, instance_id: &str, initial_pos: (f32, f32)) {
        self.player_leave(player_id);

        let packet = MovementPacket {
            player_id,
            x: initial_pos.0,
            y: initial_pos.1,
            vx: 0.0,
            vy: 0.0,
            facing: "south".into(),
            state: "idle".into(),
            instance_id: instance_id.to_string(),
            timestamp: chrono::Utc::now().timestamp_millis(),
        };

        self.players.insert(player_id, packet);
        let mut entry = self.instances.entry(instance_id.to_string()).or_default();
        if !entry.contains(&player_id) {
            entry.push(player_id);
        }
    }

    pub fn player_leave(&self, player_id: Uuid) {
        if let Some((_, packet)) = self.players.remove(&player_id) {
            if let Some(mut set) = self.instances.get_mut(&packet.instance_id) {
                set.retain(|&id| id != player_id);
            }
        }
    }

    pub fn update_movement(&self, packet: MovementPacket) -> Result<()> {
        let player_id = packet.player_id;
        self.players.insert(player_id, packet);
        Ok(())
    }

    pub fn get_players_in_instance(&self, instance_id: &str) -> Vec<MovementPacket> {
        let mut result = Vec::new();
        if let Some(player_ids) = self.instances.get(instance_id) {
            for id in player_ids.iter() {
                if let Some(p) = self.players.get(id) {
                    result.push(p.clone());
                }
            }
        }
        result
    }

    pub fn get_player(&self, player_id: Uuid) -> Option<MovementPacket> {
        self.players.get(&player_id).map(|p| p.clone())
    }
}
