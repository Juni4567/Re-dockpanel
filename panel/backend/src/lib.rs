pub mod auth;
pub mod config;
pub mod error;
pub mod helpers;
pub mod models;
pub mod routes;
pub mod safe_cmd;
pub mod services;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::RwLock;

use config::Config;
use services::agent::{AgentClient, AgentRegistry};

#[derive(Clone)]
pub struct AppState {
    pub db: sqlx::PgPool,
    pub config: Arc<Config>,
    /// Legacy single-agent accessor (routes being migrated will use `agents` instead).
    pub agent: AgentClient,
    /// Multi-server agent registry: dispatches to local or remote agents by server_id.
    pub agents: AgentRegistry,
    pub login_attempts: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
    /// Blacklisted JWT JTIs (for logout). Entries expire naturally after 2h.
    pub token_blacklist: Arc<RwLock<HashSet<String>>>,
    /// Rate limiter for 2FA verification attempts: user_id -> (count, window_start)
    pub twofa_attempts: Arc<Mutex<HashMap<uuid::Uuid, (u32, Instant)>>>,
    /// Rate limiter for deploy webhooks: site_id -> (failed_count, window_start)
    pub webhook_attempts: Arc<Mutex<HashMap<uuid::Uuid, (u32, Instant)>>>,
    /// Rate limiter for agent endpoints: server_id -> (count, window_start)
    pub agent_rate_limits: Arc<Mutex<HashMap<uuid::Uuid, (u32, Instant)>>>,
    /// Provisioning log channels: site_id -> (step history, broadcast sender)
    pub provision_logs: Arc<Mutex<HashMap<uuid::Uuid, (Vec<routes::sites::ProvisionStep>, tokio::sync::broadcast::Sender<routes::sites::ProvisionStep>, Instant)>>>,
    /// OAuth CSRF state tokens: state_string -> (provider_name, created_at)
    pub oauth_states: Arc<Mutex<HashMap<String, (String, Instant)>>>,
    /// Broadcast channel for real-time panel notification delivery (SSE)
    pub notif_tx: tokio::sync::broadcast::Sender<(uuid::Uuid, String)>,
    /// Cached `sessions_revoked_at` timestamp (epoch seconds).
    /// Auth middleware rejects tokens with `iat` before this value.
    /// Updated when admin calls revoke-all; avoids a DB query per request.
    pub sessions_revoked_at: Arc<RwLock<Option<i64>>>,
    /// Deploy ownership map: deploy_id -> user_id (for SSE log access control).
    pub deploy_owners: Arc<Mutex<HashMap<uuid::Uuid, uuid::Uuid>>>,
    /// Shutdown broadcast, mirroring the one `spawn_supervised` already hands
    /// every background service. Route handlers that hold a connection open
    /// past a single request/response (the metrics websocket, every SSE
    /// stream below) subscribe to this so `shutdown_signal`'s graceful drain
    /// isn't left waiting on a client that never disconnects on its own —
    /// see `helpers::shutdown_signal_fut`.
    pub shutdown_tx: tokio::sync::broadcast::Sender<()>,
    /// WebAuthn/Passkey challenge store (in-memory, 5-minute TTL).
    pub passkey_challenges: routes::passkeys::ChallengeStore,
    /// Phase 4 W4: panel self-update orchestrator state. Read by
    /// `/api/update/status`; written by `start_panel_update`. In-process
    /// only — DB rows in `panel_snapshots` are the durable cross-restart
    /// signal (the api process dies mid-binary-swap).
    pub panel_update_state: services::panel_update::UpdateStateHandle,
}
