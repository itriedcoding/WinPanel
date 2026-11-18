//! WebSocket streams: live server console and global status events.

use crate::auth::{load_server, AuthUser};
use crate::error::{AppError, Result};
use crate::events::EventBus;
use crate::process::Supervisor;
use crate::state::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ClientMessage {
    Input { command: String },
    Clear,
    Ping,
}

pub async fn console_socket(
    State(state): State<Arc<AppState>>,
    auth: AuthUser,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    load_server(&state, &auth, &id, "console")?;
    let supervisor = state.supervisor.clone();
    let events = state.events.clone();
    Ok(ws
        .max_message_size(64 * 1024)
        .on_upgrade(move |socket| console_loop(socket, supervisor, events, id)))
}

async fn console_loop(
    socket: WebSocket,
    supervisor: Arc<Supervisor>,
    events: EventBus,
    server_id: String,
) {
    let (mut sink, mut stream) = socket.split();
    let mut rx = events.subscribe();

    for line in supervisor.console_snapshot(&server_id, 500) {
        let payload = json!({ "type": "console", "line": line }).to_string();
        if sink.send(Message::Text(payload)).await.is_err() {
            return;
        }
    }
    let runtime = supervisor.runtime(&server_id);
    let _ = sink
        .send(Message::Text(
            json!({
                "type": "status",
                "status": runtime.status,
                "pid": runtime.pid,
            })
            .to_string(),
        ))
        .await;

    let mut keepalive = tokio::time::interval(Duration::from_secs(25));
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Ok(event) => {
                        if event.server_id.as_deref() != Some(server_id.as_str()) {
                            continue;
                        }
                        let Ok(text) = serde_json::to_string(&event) else { continue };
                        if sink.send(Message::Text(text)).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        let notice = json!({
                            "type": "notice",
                            "message": format!("{skipped} console lines were dropped because the client could not keep up"),
                        });
                        let _ = sink.send(Message::Text(notice.to_string())).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(ClientMessage::Input { command }) => {
                                if let Err(err) = supervisor.write_input(&server_id, command.trim()).await {
                                    let _ = sink
                                        .send(Message::Text(
                                            json!({ "type": "notice", "level": "error", "message": err.message() })
                                                .to_string(),
                                        ))
                                        .await;
                                }
                            }
                            Ok(ClientMessage::Clear) => supervisor.clear_console(&server_id),
                            Ok(ClientMessage::Ping) => {
                                let _ = sink.send(Message::Text("{\"type\":\"pong\"}".into())).await;
                            }
                            Err(err) => {
                                let _ = sink
                                    .send(Message::Text(
                                        json!({ "type": "notice", "level": "error", "message": format!("Invalid message: {err}") })
                                            .to_string(),
                                    ))
                                    .await;
                            }
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = sink.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Err(err)) => {
                        tracing::debug!("console socket closed: {err}");
                        return;
                    }
                    _ => {}
                }
            }
            _ = keepalive.tick() => {
                if sink.send(Message::Ping(Vec::new())).await.is_err() {
                    return;
                }
            }
        }
    }
}

pub async fn events_socket(
    State(state): State<Arc<AppState>>,
    _auth: AuthUser,
    ws: WebSocketUpgrade,
) -> Result<Response> {
    let events = state.events.clone();
    Ok(ws.on_upgrade(move |socket| events_loop(socket, events)))
}

async fn events_loop(socket: WebSocket, events: EventBus) {
    let (mut sink, mut stream) = socket.split();
    let mut rx = events.subscribe();
    let mut keepalive = tokio::time::interval(Duration::from_secs(30));
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            event = rx.recv() => {
                match event {
                    Ok(event) => {
                        // Console output has its own per-server socket.
                        if event.kind == "console" {
                            continue;
                        }
                        let Ok(text) = serde_json::to_string(&event) else { continue };
                        if sink.send(Message::Text(text)).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Err(_)) => return,
                    _ => {}
                }
            }
            _ = keepalive.tick() => {
                if sink.send(Message::Ping(Vec::new())).await.is_err() {
                    return;
                }
            }
        }
    }
}

pub fn _ensure_error_used(_: AppError) {}
