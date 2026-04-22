use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

use crate::api::MisskeyClient;
use crate::db::Database;
use crate::error::NoteDeckError;
use crate::event_bus::{EventBus, SseEvent};
use crate::models::{
    ChatMessage, NormalizedNote, NormalizedNotification, RawNote, RawNotification, TimelineType,
    TimelineOptions,
};

/// Trait for emitting events to a frontend (e.g., Tauri WebView).
/// In CLI/daemon mode, use `NoopEmitter`.
pub trait FrontendEmitter: Send + Sync {
    fn emit(&self, event: &str, payload: Value);
}

/// No-op emitter for headless/CLI mode.
pub struct NoopEmitter;

impl FrontendEmitter for NoopEmitter {
    fn emit(&self, _event: &str, _payload: Value) {}
}

/// Emitter that forwards events to the EventBus for SSE delivery.
pub struct EventBusEmitter {
    event_bus: Arc<EventBus>,
}

impl EventBusEmitter {
    pub fn new(event_bus: Arc<EventBus>) -> Self {
        Self { event_bus }
    }
}

impl FrontendEmitter for EventBusEmitter {
    fn emit(&self, event: &str, payload: Value) {
        self.event_bus.send(SseEvent {
            event_type: event.to_string(),
            data: payload,
        });
    }
}

/// Emit to FrontendEmitter only (status events, polling, etc.)
macro_rules! emit_or_log {
    ($emitter:expr, $event:expr, $payload:expr) => {
        $emitter.emit(
            $event,
            serde_json::to_value(&$payload).unwrap_or_default(),
        );
    };
}

/// Serialize payload once and send to both EventBus and FrontendEmitter.
macro_rules! emit_event {
    ($emitter:expr, $event_bus:expr, $event_type:expr, $emitter_event:expr, $payload:expr) => {{
        let data = serde_json::to_value(&$payload).unwrap_or_default();
        $event_bus.send(SseEvent {
            event_type: $event_type.to_string(),
            data: data.clone(),
        });
        $emitter.emit($emitter_event, data);
    }};
}


const WS_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

fn ws_config() -> WebSocketConfig {
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(10 * 1024 * 1024); // 10 MB
    config.max_frame_size = Some(2 * 1024 * 1024); // 2 MB
    config
}

// --- Event payloads ---

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamNoteEvent {
    pub account_id: String,
    pub subscription_id: String,
    pub note: Arc<NormalizedNote>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamNotificationEvent {
    pub account_id: String,
    pub subscription_id: String,
    pub notification: NormalizedNotification,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamMentionEvent {
    pub account_id: String,
    pub subscription_id: String,
    pub note: Arc<NormalizedNote>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamChatMessageEvent {
    pub account_id: String,
    pub subscription_id: String,
    pub message: ChatMessage,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamChatMessageDeletedEvent {
    pub account_id: String,
    pub subscription_id: String,
    pub message_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamMainEvent {
    pub account_id: String,
    pub subscription_id: String,
    pub event_type: String,
    pub body: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamNoteUpdatedEvent {
    pub account_id: String,
    pub subscription_id: String,
    pub note_id: String,
    pub update_type: String,
    pub body: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamNoteCaptureEvent {
    pub account_id: String,
    pub note_id: String,
    pub update_type: String,
    pub body: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamStatusEvent {
    pub account_id: String,
    pub state: String,
}

// --- Internal commands sent to the WebSocket task ---

enum WsCommand {
    Subscribe { channel: String, id: String, params: Option<Value> },
    Unsubscribe { id: String },
    SubNote { id: String },
    UnsubNote { id: String },
    Shutdown,
}

struct ConnectionHandle {
    cmd_tx: mpsc::UnboundedSender<WsCommand>,
    task: tokio::task::JoinHandle<()>,
    host: String,
}

/// Handle for a polling task (non-WebSocket mode).
#[allow(dead_code)]
struct PollingHandle {
    task: tokio::task::JoinHandle<()>,
    cancel: tokio::sync::watch::Sender<bool>,
    host: String,
    token: String,
}

// --- Subscription tracking ---

#[derive(Debug, Clone)]
struct SubscriptionInfo {
    account_id: String,
    host: String,
    /// "timeline", "antenna", "channel", "main", or "chat"
    kind: String,
    /// The Misskey channel name (e.g. "homeTimeline", "main")
    channel: String,
    /// Original timeline type (e.g. "home", "local") for cache isolation
    timeline_type: String,
    /// Extra params for channel subscription (e.g. listId for userListTimeline)
    params: Option<Value>,
}

pub struct StreamingManager {
    connections: Arc<Mutex<HashMap<String, ConnectionHandle>>>,
    poll_connections: Arc<Mutex<HashMap<String, PollingHandle>>>,
    subscriptions: Arc<RwLock<HashMap<String, SubscriptionInfo>>>,
    /// Note IDs being captured per account (for polling mode note updates).
    captured_notes: Arc<RwLock<HashMap<String, HashSet<String>>>>,
    emitter: Arc<dyn FrontendEmitter>,
    event_bus: Arc<EventBus>,
    db: Arc<Database>,
    api_client: Arc<MisskeyClient>,
}

impl StreamingManager {
    pub fn new(
        emitter: Arc<dyn FrontendEmitter>,
        event_bus: Arc<EventBus>,
        db: Arc<Database>,
    ) -> Self {
        Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
            poll_connections: Arc::new(Mutex::new(HashMap::new())),
            subscriptions: Arc::new(RwLock::new(HashMap::new())),
            captured_notes: Arc::new(RwLock::new(HashMap::new())),
            emitter,
            event_bus,
            db,
            api_client: Arc::new(MisskeyClient::new().expect("failed to create HTTP client")),
        }
    }

    pub async fn connect(
        &self,
        account_id: &str,
        host: &str,
        token: &str,
    ) -> Result<(), NoteDeckError> {
        let mut conns = self.connections.lock().await;
        if conns.contains_key(account_id) {
            return Ok(());
        }

        let url = format!("wss://{host}/streaming?i={token}");

        // Verify initial connection (with timeout to prevent hang DoS)
        let (ws_stream, _) = tokio::time::timeout(
            WS_CONNECT_TIMEOUT,
            tokio_tungstenite::connect_async_with_config(&url, Some(ws_config()), false),
        )
        .await
        .map_err(|_| NoteDeckError::WebSocket("Connection timeout".to_string()))?
        .map_err(|e| NoteDeckError::WebSocket(e.to_string()))?;

        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();

        let account_id_owned = account_id.to_string();
        let url_owned = url.clone();
        let subscriptions = self.subscriptions.clone();
        let emitter = self.emitter.clone();
        let event_bus = self.event_bus.clone();
        let db = self.db.clone();

        let task = tokio::spawn(async move {
            connection_task(
                emitter,
                event_bus,
                db,
                account_id_owned,
                url_owned,
                ws_stream,
                cmd_rx,
                subscriptions,
            )
            .await;
        });

        conns.insert(
            account_id.to_string(),
            ConnectionHandle {
                cmd_tx,
                task,
                host: host.to_string(),
            },
        );

        emit_or_log!(self.emitter, "stream-status", StreamStatusEvent {
            account_id: account_id.to_string(),
            state: "connected".to_string(),
        });

        Ok(())
    }

    pub async fn disconnect(&self, account_id: &str) {
        // Stop WebSocket connection if any
        let mut conns = self.connections.lock().await;
        if let Some(handle) = conns.remove(account_id) {
            if let Err(e) = handle.cmd_tx.send(WsCommand::Shutdown) {
                tracing::warn!(account_id, error = %e, "failed to send shutdown");
            }
            if let Err(e) = handle.task.await {
                tracing::error!(account_id, error = %e, "task join error");
            }
        }
        drop(conns);

        // Stop polling task if any
        let mut polls = self.poll_connections.lock().await;
        if let Some(handle) = polls.remove(account_id) {
            let _ = handle.cancel.send(true);
            handle.task.abort();
        }
        drop(polls);

        // Remove all subscriptions for this account
        let mut subs = self.subscriptions.write().await;
        subs.retain(|_, info| info.account_id != account_id);

        emit_or_log!(self.emitter, "stream-status", StreamStatusEvent {
            account_id: account_id.to_string(),
            state: "disconnected".to_string(),
        });
    }

    /// Switch between realtime (WebSocket) and polling (HTTP) mode.
    /// Subscriptions are preserved across the switch.
    pub async fn set_mode(
        &self,
        account_id: &str,
        host: &str,
        token: &str,
        mode: &str,
        interval_ms: Option<u64>,
    ) -> Result<(), NoteDeckError> {
        match mode {
            "realtime" => {
                // Stop polling if active
                {
                    let mut polls = self.poll_connections.lock().await;
                    if let Some(handle) = polls.remove(account_id) {
                        let _ = handle.cancel.send(true);
                        handle.task.abort();
                    }
                }
                // Start WebSocket (connect is idempotent)
                self.connect(account_id, host, token).await?;
            }
            "polling" => {
                // Stop WebSocket if active (but keep subscriptions)
                {
                    let mut conns = self.connections.lock().await;
                    if let Some(handle) = conns.remove(account_id) {
                        let _ = handle.cmd_tx.send(WsCommand::Shutdown);
                        let _ = handle.task.await;
                    }
                }
                // Start polling task
                let interval = Duration::from_millis(interval_ms.unwrap_or(15_000));
                self.start_polling(account_id, host, token, interval).await;

                emit_or_log!(self.emitter, "stream-status", StreamStatusEvent {
                    account_id: account_id.to_string(),
                    state: "connected".to_string(),
                });
            }
            _ => {
                return Err(NoteDeckError::InvalidInput(format!("unknown mode: {mode}")));
            }
        }
        Ok(())
    }

    async fn start_polling(
        &self,
        account_id: &str,
        host: &str,
        token: &str,
        interval: Duration,
    ) {
        let mut polls = self.poll_connections.lock().await;

        // Stop existing polling task
        if let Some(handle) = polls.remove(account_id) {
            let _ = handle.cancel.send(true);
            handle.task.abort();
        }

        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

        let account_id_owned = account_id.to_string();
        let host_owned = host.to_string();
        let token_owned = token.to_string();
        let subscriptions = self.subscriptions.clone();
        let emitter = self.emitter.clone();
        let event_bus = self.event_bus.clone();
        let db = self.db.clone();
        let api_client = self.api_client.clone();
        let captured_notes = self.captured_notes.clone();

        let task = tokio::spawn(async move {
            polling_loop(
                api_client,
                emitter,
                event_bus,
                db,
                account_id_owned,
                host_owned,
                token_owned,
                interval,
                subscriptions,
                captured_notes,
                cancel_rx,
            )
            .await;
        });

        polls.insert(
            account_id.to_string(),
            PollingHandle {
                task,
                cancel: cancel_tx,
                host: host.to_string(),
                token: token.to_string(),
            },
        );
    }

    pub async fn subscribe_timeline(
        &self,
        account_id: &str,
        timeline_type: TimelineType,
        list_id: Option<String>,
    ) -> Result<String, NoteDeckError> {
        let sub_id = uuid::Uuid::new_v4().to_string();
        let channel = timeline_type.ws_channel();
        let params = list_id.as_ref().map(|id| json!({ "listId": id }));

        let host = self.get_host(account_id).await?;
        self.send_subscribe(account_id, &channel, &sub_id, params.clone()).await?;

        let mut subs = self.subscriptions.write().await;
        subs.insert(
            sub_id.clone(),
            SubscriptionInfo {
                account_id: account_id.to_string(),
                host,
                kind: "timeline".to_string(),
                channel: channel.clone(),
                timeline_type: timeline_type.as_str().to_string(),
                params,
            },
        );

        Ok(sub_id)
    }

    pub async fn subscribe_antenna(
        &self,
        account_id: &str,
        antenna_id: &str,
    ) -> Result<String, NoteDeckError> {
        let sub_id = uuid::Uuid::new_v4().to_string();
        let params = Some(json!({ "antennaId": antenna_id }));

        let host = self.get_host(account_id).await?;
        self.send_subscribe(account_id, "antenna", &sub_id, params.clone()).await?;

        let mut subs = self.subscriptions.write().await;
        subs.insert(
            sub_id.clone(),
            SubscriptionInfo {
                account_id: account_id.to_string(),
                host,
                kind: "antenna".to_string(),
                channel: "antenna".to_string(),
                timeline_type: String::new(),
                params,
            },
        );

        Ok(sub_id)
    }

    pub async fn subscribe_channel(
        &self,
        account_id: &str,
        channel_id: &str,
    ) -> Result<String, NoteDeckError> {
        let sub_id = uuid::Uuid::new_v4().to_string();
        let params = Some(json!({ "channelId": channel_id }));

        let host = self.get_host(account_id).await?;
        self.send_subscribe(account_id, "channel", &sub_id, params.clone()).await?;

        let mut subs = self.subscriptions.write().await;
        subs.insert(
            sub_id.clone(),
            SubscriptionInfo {
                account_id: account_id.to_string(),
                host,
                kind: "channel".to_string(),
                channel: "channel".to_string(),
                timeline_type: String::new(),
                params,
            },
        );

        Ok(sub_id)
    }

    pub async fn subscribe_role(
        &self,
        account_id: &str,
        role_id: &str,
    ) -> Result<String, NoteDeckError> {
        let sub_id = uuid::Uuid::new_v4().to_string();
        let params = Some(json!({ "roleId": role_id }));

        let host = self.get_host(account_id).await?;
        self.send_subscribe(account_id, "roleTimeline", &sub_id, params.clone()).await?;

        let mut subs = self.subscriptions.write().await;
        subs.insert(
            sub_id.clone(),
            SubscriptionInfo {
                account_id: account_id.to_string(),
                host,
                kind: "role".to_string(),
                channel: "roleTimeline".to_string(),
                timeline_type: String::new(),
                params,
            },
        );

        Ok(sub_id)
    }

    pub async fn subscribe_chat_user(
        &self,
        account_id: &str,
        other_id: &str,
    ) -> Result<String, NoteDeckError> {
        let sub_id = uuid::Uuid::new_v4().to_string();
        let params = Some(json!({ "otherId": other_id }));

        let host = self.get_host(account_id).await?;
        self.send_subscribe(account_id, "chatUser", &sub_id, params.clone()).await?;

        let mut subs = self.subscriptions.write().await;
        subs.insert(
            sub_id.clone(),
            SubscriptionInfo {
                account_id: account_id.to_string(),
                host,
                kind: "chat".to_string(),
                channel: "chatUser".to_string(),
                timeline_type: String::new(),
                params,
            },
        );

        Ok(sub_id)
    }

    pub async fn subscribe_chat_room(
        &self,
        account_id: &str,
        room_id: &str,
    ) -> Result<String, NoteDeckError> {
        let sub_id = uuid::Uuid::new_v4().to_string();
        let params = Some(json!({ "roomId": room_id }));

        let host = self.get_host(account_id).await?;
        self.send_subscribe(account_id, "chatRoom", &sub_id, params.clone()).await?;

        let mut subs = self.subscriptions.write().await;
        subs.insert(
            sub_id.clone(),
            SubscriptionInfo {
                account_id: account_id.to_string(),
                host,
                kind: "chat".to_string(),
                channel: "chatRoom".to_string(),
                timeline_type: String::new(),
                params,
            },
        );

        Ok(sub_id)
    }

    pub async fn subscribe_main(&self, account_id: &str) -> Result<String, NoteDeckError> {
        let sub_id = uuid::Uuid::new_v4().to_string();

        let host = self.get_host(account_id).await?;
        self.send_subscribe(account_id, "main", &sub_id, None).await?;

        let mut subs = self.subscriptions.write().await;
        subs.insert(
            sub_id.clone(),
            SubscriptionInfo {
                account_id: account_id.to_string(),
                host,
                kind: "main".to_string(),
                channel: "main".to_string(),
                timeline_type: String::new(),
                params: None,
            },
        );

        Ok(sub_id)
    }

    pub async fn unsubscribe(&self, account_id: &str, subscription_id: &str) -> Result<(), NoteDeckError> {
        // Send unsubscribe to WebSocket if in realtime mode
        let conns = self.connections.lock().await;
        if let Some(handle) = conns.get(account_id) {
            let _ = handle.cmd_tx.send(WsCommand::Unsubscribe {
                id: subscription_id.to_string(),
            });
        }
        drop(conns);

        // Always remove from subscriptions map (both modes use it)
        let mut subs = self.subscriptions.write().await;
        subs.remove(subscription_id);

        Ok(())
    }

    pub async fn sub_note(&self, account_id: &str, note_id: &str) -> Result<(), NoteDeckError> {
        // WebSocket mode: send subNote command
        let conns = self.connections.lock().await;
        if let Some(handle) = conns.get(account_id) {
            return handle
                .cmd_tx
                .send(WsCommand::SubNote { id: note_id.to_string() })
                .map_err(|_| NoteDeckError::ConnectionClosed);
        }
        drop(conns);

        // Polling mode: add to captured_notes set for batch polling
        let mut captured = self.captured_notes.write().await;
        captured
            .entry(account_id.to_string())
            .or_default()
            .insert(note_id.to_string());
        Ok(())
    }

    pub async fn unsub_note(&self, account_id: &str, note_id: &str) -> Result<(), NoteDeckError> {
        // WebSocket mode
        let conns = self.connections.lock().await;
        if let Some(handle) = conns.get(account_id) {
            return handle
                .cmd_tx
                .send(WsCommand::UnsubNote { id: note_id.to_string() })
                .map_err(|_| NoteDeckError::ConnectionClosed);
        }
        drop(conns);

        // Polling mode: remove from captured_notes
        let mut captured = self.captured_notes.write().await;
        if let Some(set) = captured.get_mut(account_id) {
            set.remove(note_id);
        }
        Ok(())
    }

    async fn get_host(&self, account_id: &str) -> Result<String, NoteDeckError> {
        // Check WebSocket connections first
        let conns = self.connections.lock().await;
        if let Some(h) = conns.get(account_id) {
            return Ok(h.host.clone());
        }
        drop(conns);

        // Check polling connections
        let polls = self.poll_connections.lock().await;
        if let Some(h) = polls.get(account_id) {
            return Ok(h.host.clone());
        }

        Err(NoteDeckError::NoConnection(account_id.to_string()))
    }

    async fn send_subscribe(
        &self,
        account_id: &str,
        channel: &str,
        sub_id: &str,
        params: Option<Value>,
    ) -> Result<(), NoteDeckError> {
        let conns = self.connections.lock().await;
        if let Some(handle) = conns.get(account_id) {
            // WebSocket mode: send subscribe command
            return handle
                .cmd_tx
                .send(WsCommand::Subscribe {
                    channel: channel.to_string(),
                    id: sub_id.to_string(),
                    params,
                })
                .map_err(|_| NoteDeckError::ConnectionClosed);
        }
        drop(conns);

        // Polling mode: subscription is tracked in the subscriptions map
        // (no WebSocket command needed — polling loop reads from subscriptions directly)
        let polls = self.poll_connections.lock().await;
        if polls.contains_key(account_id) {
            return Ok(());
        }

        Err(NoteDeckError::NoConnection(account_id.to_string()))
    }
}

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;
type WsRead = futures_util::stream::SplitStream<WsStream>;
type WsWrite = Arc<
    Mutex<
        futures_util::stream::SplitSink<WsStream, Message>,
    >,
>;

enum WsExitReason {
    Disconnected,
    Shutdown,
}

const MAX_BACKOFF_SECS: u64 = 30;

/// Top-level task that handles reconnection with exponential backoff.
#[allow(clippy::too_many_arguments)]
async fn connection_task(
    emitter: Arc<dyn FrontendEmitter>,
    event_bus: Arc<EventBus>,
    db: Arc<Database>,
    account_id: String,
    url: String,
    initial_ws: WsStream,
    mut cmd_rx: mpsc::UnboundedReceiver<WsCommand>,
    subscriptions: Arc<RwLock<HashMap<String, SubscriptionInfo>>>,
) {
    let mut backoff_secs: u64 = 1;

    // Run the first session with the already-connected WebSocket
    let reason = run_ws_session(&emitter, &event_bus, &db, &account_id, initial_ws, &mut cmd_rx, &subscriptions).await;
    if matches!(reason, WsExitReason::Shutdown) {
        return;
    }

    // Reconnection loop
    loop {
        emit_or_log!(emitter, "stream-status", StreamStatusEvent {
            account_id: account_id.clone(),
            state: "reconnecting".to_string(),
        });

        // Wait with backoff, but listen for Shutdown during the wait
        let sleep = tokio::time::sleep(Duration::from_secs(backoff_secs));
        tokio::pin!(sleep);

        let shutdown_during_wait = loop {
            tokio::select! {
                _ = &mut sleep => break false,
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(WsCommand::Shutdown) | None => break true,
                        // Subscribe/Unsubscribe: safe to drop here because the
                        // subscriptions table is already updated by the caller.
                        // run_ws_session will re-subscribe from that table.
                        _ => {}
                    }
                }
            }
        };

        if shutdown_during_wait {
            return;
        }

        // Attempt reconnection (with timeout)
        let ws_result = tokio::time::timeout(
            WS_CONNECT_TIMEOUT,
            tokio_tungstenite::connect_async_with_config(&url, Some(ws_config()), false),
        )
        .await;
        match ws_result {
            Ok(Ok((ws_stream, _))) => {
                backoff_secs = 1; // Reset backoff on success

                emit_or_log!(emitter, "stream-status", StreamStatusEvent {
                    account_id: account_id.clone(),
                    state: "connected".to_string(),
                });

                let reason = run_ws_session(
                    &emitter,
                    &event_bus,
                    &db,
                    &account_id,
                    ws_stream,
                    &mut cmd_rx,
                    &subscriptions,
                )
                .await;

                if matches!(reason, WsExitReason::Shutdown) {
                    return;
                }
            }
            Ok(Err(e)) => {
                tracing::warn!(account_id = %account_id, error = %e, backoff_secs, "reconnect failed");
                backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
            }
            Err(_) => {
                tracing::warn!(account_id = %account_id, backoff_secs, "reconnect timeout");
                backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
            }
        }
    }
}

/// Run a single WebSocket session. Re-subscribes existing channels, then enters the message loop.
async fn run_ws_session(
    emitter: &Arc<dyn FrontendEmitter>,
    event_bus: &Arc<EventBus>,
    db: &Arc<Database>,
    account_id: &str,
    ws_stream: WsStream,
    cmd_rx: &mut mpsc::UnboundedReceiver<WsCommand>,
    subscriptions: &Arc<RwLock<HashMap<String, SubscriptionInfo>>>,
) -> WsExitReason {
    let (write, read) = ws_stream.split();
    let write = Arc::new(Mutex::new(write));

    // Collect subscriptions to replay, then drop the lock before doing I/O
    let to_resub: Vec<(String, String, Option<Value>)> = {
        let subs = subscriptions.read().await;
        subs.iter()
            .filter(|(_, info)| info.account_id == account_id)
            .map(|(sub_id, info)| (sub_id.clone(), info.channel.clone(), info.params.clone()))
            .collect()
    };

    if !to_resub.is_empty() {
        let mut w = write.lock().await;
        for (sub_id, channel, params) in &to_resub {
            let mut body = json!({ "channel": channel, "id": sub_id });
            if let Some(p) = params {
                body["params"] = p.clone();
            }
            let msg = json!({ "type": "connect", "body": body });
            if let Err(e) = w.send(Message::Text(msg.to_string().into())).await {
                tracing::warn!(error = %e, "re-subscribe send failed");
                break;
            }
        }
    }

    ws_loop(emitter, event_bus, db, account_id, read, write, cmd_rx, subscriptions).await
}

#[allow(clippy::too_many_arguments)]
async fn ws_loop(
    emitter: &Arc<dyn FrontendEmitter>,
    event_bus: &Arc<EventBus>,
    db: &Arc<Database>,
    account_id: &str,
    mut read: WsRead,
    write: WsWrite,
    cmd_rx: &mut mpsc::UnboundedReceiver<WsCommand>,
    subscriptions: &Arc<RwLock<HashMap<String, SubscriptionInfo>>>,
) -> WsExitReason {
    let mut ping_interval = tokio::time::interval(Duration::from_secs(30));
    ping_interval.tick().await; // skip the first immediate tick

    loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        let emitter = emitter.clone();
                        let event_bus = event_bus.clone();
                        let db = db.clone();
                        let account_id = account_id.to_string();
                        let subscriptions = subscriptions.clone();
                        tokio::spawn(async move {
                            handle_ws_message(&*emitter, &event_bus, &db, &account_id, &text, &subscriptions).await;
                        });
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let mut w = write.lock().await;
                        if let Err(e) = w.send(Message::Pong(data)).await {
                            tracing::warn!(error = %e, "pong send failed");
                            return WsExitReason::Disconnected;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => {
                        return WsExitReason::Disconnected;
                    }
                    _ => {}
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(WsCommand::Subscribe { channel, id, params }) => {
                        let mut body = json!({ "channel": channel, "id": id });
                        if let Some(p) = params {
                            body["params"] = p;
                        }
                        let msg = json!({ "type": "connect", "body": body });
                        let mut w = write.lock().await;
                        if let Err(e) = w.send(Message::Text(msg.to_string().into())).await {
                            tracing::warn!(error = %e, "subscribe send failed");
                        }
                    }
                    Some(WsCommand::Unsubscribe { id }) => {
                        let msg = json!({
                            "type": "disconnect",
                            "body": { "id": id }
                        });
                        let mut w = write.lock().await;
                        if let Err(e) = w.send(Message::Text(msg.to_string().into())).await {
                            tracing::warn!(error = %e, "unsubscribe send failed");
                        }
                    }
                    Some(WsCommand::SubNote { id }) => {
                        let msg = json!({ "type": "subNote", "body": { "id": id } });
                        let mut w = write.lock().await;
                        if let Err(e) = w.send(Message::Text(msg.to_string().into())).await {
                            tracing::warn!(error = %e, "subNote send failed");
                        }
                    }
                    Some(WsCommand::UnsubNote { id }) => {
                        let msg = json!({ "type": "unsubNote", "body": { "id": id } });
                        let mut w = write.lock().await;
                        if let Err(e) = w.send(Message::Text(msg.to_string().into())).await {
                            tracing::warn!(error = %e, "unsubNote send failed");
                        }
                    }
                    Some(WsCommand::Shutdown) | None => {
                        let mut w = write.lock().await;
                        let _ = w.close().await;
                        return WsExitReason::Shutdown;
                    }
                }
            }
            _ = ping_interval.tick() => {
                let mut w = write.lock().await;
                if let Err(e) = w.send(Message::Ping(vec![].into())).await {
                    tracing::warn!(account_id, error = %e, "keepalive ping failed");
                    return WsExitReason::Disconnected;
                }
            }
        }
    }
}

async fn handle_ws_message(
    emitter: &dyn FrontendEmitter,
    event_bus: &EventBus,
    db: &Arc<Database>,
    account_id: &str,
    text: &str,
    subscriptions: &Arc<RwLock<HashMap<String, SubscriptionInfo>>>,
) {
    let mut msg: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return,
    };

    let msg_type = match msg.get("type").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return,
    };

    // Note Capture: { "type": "noteUpdated", "body": { "id": "...", "type": "...", "body": ... } }
    if msg_type == "noteUpdated" {
        if let Some(mut body) = msg.get_mut("body").map(Value::take) {
            let note_id = body.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_owned();
            let update_type = body.get("type").and_then(|v| v.as_str()).unwrap_or_default().to_owned();
            let update_body = body.get_mut("body").map(Value::take).unwrap_or_default();
            let payload = StreamNoteCaptureEvent {
                account_id: account_id.to_string(),
                note_id,
                update_type,
                body: update_body,
            };
            emit_event!(emitter, event_bus, "note-capture-updated", "stream-note-capture-updated", payload);
        }
        return;
    }

    // Misskey streaming: { "type": "channel", "body": { "id": "...", "type": "...", "body": ... } }
    if msg_type != "channel" {
        return;
    }

    let mut body = match msg.get_mut("body").map(Value::take) {
        Some(b) if !b.is_null() => b,
        _ => return,
    };

    let sub_id = match body.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_owned(),
        None => return,
    };

    let event_type = match body.get("type").and_then(|v| v.as_str()) {
        Some(t) => t.to_owned(),
        None => return,
    };

    let event_body = match body.get_mut("body").map(Value::take) {
        Some(b) if !b.is_null() => b,
        _ => return,
    };

    let (kind, host, timeline_type) = {
        let subs = subscriptions.read().await;
        match subs.get(&sub_id) {
            Some(i) => (i.kind.clone(), i.host.clone(), i.timeline_type.clone()),
            None => return,
        }
    };

    let is_note_channel = matches!(kind.as_str(), "timeline" | "antenna" | "channel" | "role");

    if is_note_channel && event_type == "note" {
        if let Ok(raw) = serde_json::from_value::<RawNote>(event_body) {
            let note = Arc::new(raw.normalize(account_id, &host));
            let db = db.clone();
            let note_for_cache = Arc::clone(&note);
            tokio::task::spawn_blocking(move || {
                if let Err(e) = db.cache_note(&note_for_cache, &timeline_type) {
                    tracing::warn!(error = %e, "failed to cache streamed note");
                }
            });
            let payload = StreamNoteEvent {
                account_id: account_id.to_string(),
                subscription_id: sub_id,
                note,
            };
            emit_event!(emitter, event_bus, "note", "stream-note", payload);
        }
    } else if is_note_channel && event_type == "noteUpdated" {
        let note_id = event_body.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_owned();
        let update_type = event_body.get("type").and_then(|v| v.as_str()).unwrap_or_default().to_owned();
        let mut event_body = event_body;
        let update_body = event_body.get_mut("body").map(Value::take).unwrap_or_default();
        let payload = StreamNoteUpdatedEvent {
            account_id: account_id.to_string(),
            subscription_id: sub_id,
            note_id,
            update_type,
            body: update_body,
        };
        emit_event!(emitter, event_bus, "note-updated", "stream-note-updated", payload);
    } else if kind == "main" {
        if event_type == "notification" {
            if let Ok(raw) = serde_json::from_value::<RawNotification>(event_body) {
                let notification = raw.normalize(account_id, &host);
                let payload = StreamNotificationEvent {
                    account_id: account_id.to_string(),
                    subscription_id: sub_id,
                    notification,
                };
                emit_event!(emitter, event_bus, "notification", "stream-notification", payload);
            }
        } else if event_type == "mention" || event_type == "reply" {
            // Serialize event_body as main-event first, then try parsing as mention
            let main_data = serde_json::to_value(&StreamMainEvent {
                account_id: account_id.to_string(),
                subscription_id: sub_id.clone(),
                event_type,
                body: event_body.clone(),
            }).unwrap_or_default();
            emitter.emit("stream-main-event", main_data);

            if let Ok(raw) = serde_json::from_value::<RawNote>(event_body) {
                let note = Arc::new(raw.normalize(account_id, &host));
                let payload = StreamMentionEvent {
                    account_id: account_id.to_string(),
                    subscription_id: sub_id,
                    note,
                };
                emit_event!(emitter, event_bus, "mention", "stream-mention", payload);
            }
        } else {
            let main_event_type = format!("main-{event_type}");
            let payload = StreamMainEvent {
                account_id: account_id.to_string(),
                subscription_id: sub_id,
                event_type,
                body: event_body,
            };
            emit_event!(emitter, event_bus, main_event_type, "stream-main-event", payload);
        }
    } else if kind == "chat" {
        if event_type == "message" {
            if let Ok(msg) = serde_json::from_value::<ChatMessage>(event_body) {
                let payload = StreamChatMessageEvent {
                    account_id: account_id.to_string(),
                    subscription_id: sub_id,
                    message: msg,
                };
                emit_event!(emitter, event_bus, "chat", "stream-chat-message", payload);
            }
        } else if event_type == "deleted" {
            if let Some(id) = event_body.as_str() {
                let payload = StreamChatMessageDeletedEvent {
                    account_id: account_id.to_string(),
                    subscription_id: sub_id,
                    message_id: id.to_string(),
                };
                emit_event!(emitter, event_bus, "chat-deleted", "stream-chat-message-deleted", payload);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Polling mode
// ---------------------------------------------------------------------------

const MAX_POLL_BACKOFF_SECS: u64 = 60;

/// Per-subscription state for polling (tracks last seen note ID).
struct PollSubState {
    since_id: Option<String>,
}

/// Top-level polling task. Periodically fetches notes for all active subscriptions.
#[allow(clippy::too_many_arguments)]
async fn polling_loop(
    api_client: Arc<MisskeyClient>,
    emitter: Arc<dyn FrontendEmitter>,
    event_bus: Arc<EventBus>,
    db: Arc<Database>,
    account_id: String,
    host: String,
    token: String,
    interval: Duration,
    subscriptions: Arc<RwLock<HashMap<String, SubscriptionInfo>>>,
    captured_notes: Arc<RwLock<HashMap<String, HashSet<String>>>>,
    mut cancel_rx: tokio::sync::watch::Receiver<bool>,
) {
    let mut sub_states: HashMap<String, PollSubState> = HashMap::new();
    // Cached reaction counts for captured notes (for diff detection).
    let mut note_reaction_cache: HashMap<String, HashMap<String, i64>> = HashMap::new();
    let mut consecutive_failures: u64 = 0;
    let mut poll_count: u64 = 0;

    loop {
        // Check cancellation
        if *cancel_rx.borrow() {
            return;
        }

        // Collect timeline subscriptions for this account
        let subs_snapshot: Vec<(String, SubscriptionInfo)> = {
            let subs = subscriptions.read().await;
            subs.iter()
                .filter(|(_, info)| info.account_id == account_id && info.kind == "timeline")
                .map(|(id, info)| (id.clone(), info.clone()))
                .collect()
        };

        let mut poll_failed = false;

        for (sub_id, info) in &subs_snapshot {
            let state = sub_states.entry(sub_id.clone()).or_insert(PollSubState {
                since_id: None,
            });

            let tl_type = TimelineType::new(&info.timeline_type);
            let mut options = TimelineOptions::new(
                30,
                state.since_id.clone(),
                None,
            );
            options.list_id = info.params.as_ref().and_then(|p| {
                p.get("listId").and_then(|v| v.as_str()).map(|s| s.to_string())
            });

            match api_client
                .get_timeline(&host, &token, &account_id, tl_type, options)
                .await
            {
                Ok(notes) if !notes.is_empty() => {
                    // Update since_id to newest note
                    state.since_id = Some(notes[0].id.clone());

                    // Emit notes in chronological order (oldest first)
                    for note in notes.into_iter().rev() {
                        let note = Arc::new(note);
                        // Cache to DB
                        let db = db.clone();
                        let note_for_cache = Arc::clone(&note);
                        let timeline_type = info.timeline_type.clone();
                        tokio::task::spawn_blocking(move || {
                            if let Err(e) = db.cache_note(&note_for_cache, &timeline_type) {
                                tracing::warn!(error = %e, "failed to cache polled note");
                            }
                        });

                        let payload = StreamNoteEvent {
                            account_id: account_id.clone(),
                            subscription_id: sub_id.clone(),
                            note,
                        };
                        emit_event!(emitter, event_bus, "note", "stream-note", payload);
                    }

                    consecutive_failures = 0;
                }
                Ok(_) => {
                    // No new notes — success, reset backoff
                    consecutive_failures = 0;
                }
                Err(e) => {
                    tracing::warn!(
                        account_id = %account_id,
                        subscription_id = %sub_id,
                        error = %e,
                        "polling fetch failed"
                    );
                    poll_failed = true;
                }
            }
        }

        // Note capture: poll every 2nd cycle (2x interval)
        poll_count += 1;
        if poll_count % 2 == 0 {
            let note_ids: Vec<String> = {
                let captured = captured_notes.read().await;
                captured
                    .get(&account_id)
                    .map(|s| s.iter().cloned().collect())
                    .unwrap_or_default()
            };

            // Fetch in batches of 8 to balance throughput vs server load
            for chunk in note_ids.chunks(8) {
                let futures: Vec<_> = chunk
                    .iter()
                    .map(|note_id| {
                        let api = api_client.clone();
                        let host = host.clone();
                        let token = token.clone();
                        let account_id = account_id.clone();
                        let note_id = note_id.clone();
                        async move {
                            let result = api
                                .get_note(&host, &token, &account_id, &note_id)
                                .await;
                            (note_id, result)
                        }
                    })
                    .collect();

                let results = futures_util::future::join_all(futures).await;

                for (note_id, result) in results {
                    if let Ok(note) = result {
                        // Diff reactions against cache
                        let new_reactions = note.reactions.clone();

                        let old_reactions = note_reaction_cache
                            .get(&note_id)
                            .cloned()
                            .unwrap_or_default();

                        // Find new/increased reactions
                        for (reaction, &new_count) in &new_reactions {
                            let old_count = old_reactions.get(reaction).copied().unwrap_or(0);
                            if new_count > old_count {
                                let payload = StreamNoteCaptureEvent {
                                    account_id: account_id.clone(),
                                    note_id: note_id.clone(),
                                    update_type: "reacted".to_string(),
                                    body: json!({
                                        "reaction": reaction,
                                        "emoji": null,
                                        "userId": null,
                                    }),
                                };
                                emit_or_log!(
                                    emitter,
                                    "stream-note-capture-updated",
                                    payload
                                );
                            }
                        }

                        // Find removed reactions
                        for (reaction, _) in &old_reactions {
                            if !new_reactions.contains_key(reaction) {
                                let payload = StreamNoteCaptureEvent {
                                    account_id: account_id.clone(),
                                    note_id: note_id.clone(),
                                    update_type: "unreacted".to_string(),
                                    body: json!({
                                        "reaction": reaction,
                                        "userId": null,
                                    }),
                                };
                                emit_or_log!(
                                    emitter,
                                    "stream-note-capture-updated",
                                    payload
                                );
                            }
                        }

                        note_reaction_cache.insert(note_id, new_reactions);
                    }
                }
            }
        }

        // Determine sleep duration
        let sleep_duration = if poll_failed {
            consecutive_failures += 1;
            let backoff = if consecutive_failures > 10 {
                MAX_POLL_BACKOFF_SECS
            } else {
                (1u64 << consecutive_failures.min(5)).min(MAX_POLL_BACKOFF_SECS)
            };

            emit_or_log!(emitter, "stream-status", StreamStatusEvent {
                account_id: account_id.clone(),
                state: "reconnecting".to_string(),
            });

            Duration::from_secs(backoff)
        } else {
            interval
        };

        // Sleep with cancellation check
        tokio::select! {
            _ = tokio::time::sleep(sleep_duration) => {}
            _ = cancel_rx.changed() => {
                return;
            }
        }
    }
}
