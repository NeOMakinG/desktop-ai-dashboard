//! Native CDP client for the embedded real-Chrome surface.
//!
//! Founder directive 2026-09-13: the real Chrome browser is experienced
//! ENTIRELY INSIDE the Forma window. The Chrome child still runs headful with
//! its normal window (never `--headless` — the whole point is a real-browser
//! fingerprint), but that window sits far offscreen while Forma renders the
//! page through `Page.startScreencast` and forwards the user's mouse and
//! keyboard through `Input.dispatchMouseEvent` / `Input.dispatchKeyEvent`.
//!
//! Anti-ban posture, on purpose: CDP `Input.dispatch*` events traverse
//! Chrome's REAL input pipeline, so pages observe `isTrusted` events with
//! ordinary key/mouse timing — indistinguishable from typing into the popped
//! out window. No emulation overrides, no headless flags, no proxy.
//!
//! Transport is a single loopback WebSocket (`ws://127.0.0.1:<port>/...`),
//! multiplexing per-tab sessions via `Target.attachToTarget { flatten: true }`.
use crate::types::{AppError, AppResult};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

pub(super) const CDP_FAILED: &str = "The embedded Chrome view lost its control channel.";
pub(super) const NOT_EMBEDDED: &str = "Open the Chrome browser first.";

fn cdp_error() -> AppError {
    AppError::new("browser_cdp_failed", CDP_FAILED)
}

// ---------------------------------------------------------------------------
// Pure protocol layer — testable without any socket.
// ---------------------------------------------------------------------------

/// Serialize one outgoing CDP command. `sessionId` is attached only for
/// per-tab commands; browser-level commands (Target.*, Browser.*) omit it.
pub(super) fn command_json(id: u64, session: Option<&str>, method: &str, params: Value) -> String {
    let mut message = json!({ "id": id, "method": method, "params": params });
    if let Some(session) = session {
        message["sessionId"] = Value::String(session.to_string());
    }
    message.to_string()
}

#[derive(Debug, PartialEq)]
pub(super) enum Incoming {
    Response {
        id: u64,
        result: Result<Value, String>,
    },
    Event {
        method: String,
        session: Option<String>,
        params: Value,
    },
}

/// Classify one incoming CDP text frame. Unknown shapes are dropped (None),
/// never panicked on — the peer is a real browser, not a trusted service.
pub(super) fn classify(text: &str) -> Option<Incoming> {
    let value: Value = serde_json::from_str(text).ok()?;
    let session = value["sessionId"].as_str().map(str::to_string);
    if let Some(id) = value["id"].as_u64() {
        let result = if value.get("error").is_some() {
            Err(value["error"]["message"].as_str().unwrap_or("error").to_string())
        } else {
            Ok(value.get("result").cloned().unwrap_or(Value::Null))
        };
        return Some(Incoming::Response { id, result });
    }
    let method = value["method"].as_str()?.to_string();
    Some(Incoming::Event {
        method,
        session,
        params: value.get("params").cloned().unwrap_or(Value::Null),
    })
}

/// One CDP event forwarded to the embedded-browser pump.
#[derive(Debug)]
pub(crate) struct CdpEvent {
    pub method: String,
    pub session: Option<String>,
    pub params: Value,
}

// ---------------------------------------------------------------------------
// Client: pending-response map + writer channel. The reader task feeds
// `handle_incoming`, which is also driven directly by the unit tests
// (mock-websocket plumbing without a socket).
// ---------------------------------------------------------------------------

pub(crate) struct CdpClient {
    tx: mpsc::UnboundedSender<String>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>,
    next: AtomicU64,
}

impl CdpClient {
    fn new(tx: mpsc::UnboundedSender<String>) -> Self {
        Self {
            tx,
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(0),
        }
    }

    #[cfg(test)]
    pub(super) fn new_for_test() -> (Arc<Self>, mpsc::UnboundedReceiver<String>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Arc::new(Self::new(tx)), rx)
    }

    /// Route one incoming frame: responses resolve their pending caller;
    /// events go to the pump. A response for an unknown id is dropped.
    pub(super) fn handle_incoming(&self, incoming: Incoming, events: &mpsc::UnboundedSender<CdpEvent>) {
        match incoming {
            Incoming::Response { id, result } => {
                let waiter = self.pending.lock().ok().and_then(|mut map| map.remove(&id));
                if let Some(waiter) = waiter {
                    let _ = waiter.send(result);
                }
            }
            Incoming::Event { method, session, params } => {
                let _ = events.send(CdpEvent { method, session, params });
            }
        }
    }

    /// The socket died: every in-flight call fails now instead of timing out.
    fn fail_all_pending(&self) {
        if let Ok(mut map) = self.pending.lock() {
            for (_, waiter) in map.drain() {
                let _ = waiter.send(Err(CDP_FAILED.to_string()));
            }
        }
    }

    /// Send a command and await its response (bounded).
    pub(crate) async fn call(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
    ) -> AppResult<Value> {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .map_err(|_| cdp_error())?
            .insert(id, tx);
        if self.tx.send(command_json(id, session, method, params)).is_err() {
            let _ = self.pending.lock().map(|mut map| map.remove(&id));
            return Err(cdp_error());
        }
        match tokio::time::timeout(Duration::from_secs(15), rx).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(_))) | Ok(Err(_)) => Err(cdp_error()),
            Err(_) => {
                let _ = self.pending.lock().map(|mut map| map.remove(&id));
                Err(cdp_error())
            }
        }
    }

    /// Fire-and-forget (input forwarding, frame acks): no response awaited,
    /// so a 15–25fps stream never queues behind round-trips.
    pub(crate) fn fire(&self, session: Option<&str>, method: &str, params: Value) {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let _ = self.tx.send(command_json(id, session, method, params));
    }
}

/// Connect to the browser-level CDP websocket for the given loopback port.
/// Returns the client plus the event stream; the stream closing means the
/// browser (or the socket) is gone.
pub(crate) async fn connect(
    port: u16,
) -> AppResult<(Arc<CdpClient>, mpsc::UnboundedReceiver<CdpEvent>)> {
    let version: Value = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| cdp_error())?
        .get(format!("http://127.0.0.1:{port}/json/version"))
        .send()
        .await
        .map_err(|_| cdp_error())?
        .json()
        .await
        .map_err(|_| cdp_error())?;
    let ws_url = version["webSocketDebuggerUrl"]
        .as_str()
        .filter(|url| url.starts_with("ws://127.0.0.1:"))
        .ok_or_else(cdp_error)?
        .to_string();
    let (stream, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|_| cdp_error())?;
    use tokio_tungstenite::tungstenite::Message;
    let (mut sink, mut source) = stream.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let (event_tx, event_rx) = mpsc::unbounded_channel::<CdpEvent>();
    // Control frames the reader must get onto the wire (Pong replies). Chrome's
    // CDP endpoint pings the client; if we never Pong, it closes the socket —
    // which previously surfaced as a spurious "lost control channel". The
    // writer answers Pings so the connection stays up.
    let (ctrl_tx, mut ctrl_rx) = mpsc::unbounded_channel::<Message>();
    let client = Arc::new(CdpClient::new(out_tx));

    tauri::async_runtime::spawn(async move {
        loop {
            let message = tokio::select! {
                biased;
                control = ctrl_rx.recv() => match control {
                    Some(message) => message,
                    None => break,
                },
                text = out_rx.recv() => match text {
                    Some(text) => Message::Text(text.into()),
                    None => break,
                },
            };
            if sink.send(message).await.is_err() {
                break;
            }
        }
    });
    let reader_client = client.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(message) = source.next().await {
            match message {
                Ok(Message::Text(text)) => {
                    if let Some(incoming) = classify(text.as_str()) {
                        reader_client.handle_incoming(incoming, &event_tx);
                    }
                }
                // Answer the server ping so it does not drop us for silence.
                Ok(Message::Ping(payload)) => {
                    let _ = ctrl_tx.send(Message::Pong(payload));
                }
                Ok(Message::Close(_)) | Err(_) => break,
                Ok(_) => {}
            }
        }
        reader_client.fail_all_pending();
        // event_tx drops here: the pump observes the channel closing.
    });
    Ok((client, event_rx))
}

// ---------------------------------------------------------------------------
// Tab state reducer — pure and unit-tested. Only real page targets are
// tracked; devtools/extension/service-worker targets never become tabs.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EmbeddedTab {
    pub id: String,
    pub url: String,
    pub title: String,
    pub session: Option<String>,
    pub loading: bool,
}

#[derive(Default, Debug)]
pub(crate) struct Tabs {
    list: Vec<EmbeddedTab>,
    active: Option<String>,
}

fn is_page_target(info: &Value) -> bool {
    info["type"] == "page"
        && !info["url"]
            .as_str()
            .map(|url| url.starts_with("devtools://") || url.starts_with("chrome-extension://"))
            .unwrap_or(true)
}

impl Tabs {
    /// Apply Target.targetCreated / Target.targetInfoChanged. Returns true
    /// when this is a page target new to the list (caller should attach).
    pub(super) fn upsert(&mut self, info: &Value) -> bool {
        if !is_page_target(info) {
            return false;
        }
        let Some(id) = info["targetId"].as_str() else { return false };
        let url = info["url"].as_str().unwrap_or("").to_string();
        let title = info["title"].as_str().unwrap_or("").to_string();
        if let Some(tab) = self.list.iter_mut().find(|tab| tab.id == id) {
            tab.url = url;
            tab.title = title;
            false
        } else {
            self.list.push(EmbeddedTab {
                id: id.to_string(),
                url,
                title,
                session: None,
                loading: false,
            });
            true
        }
    }

    /// Remove a destroyed target. Returns true when it was the active tab.
    pub(super) fn remove(&mut self, id: &str) -> bool {
        let was_active = self.active.as_deref() == Some(id);
        self.list.retain(|tab| tab.id != id);
        if was_active {
            self.active = None;
        }
        was_active
    }

    pub(super) fn attach(&mut self, id: &str, session: &str) {
        if let Some(tab) = self.list.iter_mut().find(|tab| tab.id == id) {
            tab.session = Some(session.to_string());
        }
    }

    pub(super) fn activate(&mut self, id: &str) -> bool {
        if self.list.iter().any(|tab| tab.id == id) {
            self.active = Some(id.to_string());
            true
        } else {
            false
        }
    }

    pub(super) fn navigated(&mut self, session: &str, url: &str) -> bool {
        if let Some(tab) = self
            .list
            .iter_mut()
            .find(|tab| tab.session.as_deref() == Some(session))
        {
            tab.url = url.to_string();
            true
        } else {
            false
        }
    }

    pub(super) fn set_loading(&mut self, session: &str, loading: bool) -> bool {
        if let Some(tab) = self
            .list
            .iter_mut()
            .find(|tab| tab.session.as_deref() == Some(session))
        {
            tab.loading = loading;
            true
        } else {
            false
        }
    }

    pub(super) fn session_of(&self, id: &str) -> Option<String> {
        self.list
            .iter()
            .find(|tab| tab.id == id)
            .and_then(|tab| tab.session.clone())
    }

    pub(super) fn active_id(&self) -> Option<String> {
        self.active.clone()
    }

    pub(super) fn active_session(&self) -> Option<String> {
        self.active
            .as_deref()
            .and_then(|id| self.session_of(id))
    }

    pub(super) fn tab_id_by_session(&self, session: &str) -> Option<String> {
        self.list
            .iter()
            .find(|tab| tab.session.as_deref() == Some(session))
            .map(|tab| tab.id.clone())
    }

    pub(super) fn first_id(&self) -> Option<String> {
        self.list.first().map(|tab| tab.id.clone())
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.list.len()
    }

    /// Wire snapshot for the renderer. Never includes session ids: those are
    /// a host-only capability handle.
    pub(super) fn snapshot(&self) -> Value {
        json!({
            "tabs": self.list.iter().map(|tab| json!({
                "id": tab.id,
                "url": tab.url,
                "title": tab.title,
                "loading": tab.loading,
                "active": self.active.as_deref() == Some(tab.id.as_str()),
            })).collect::<Vec<_>>(),
            "activeId": self.active,
        })
    }
}

// ---------------------------------------------------------------------------
// Input forwarding — whitelisted event shapes only. Anything outside the
// whitelist is dropped, never "best effort" forwarded.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum EmbeddedInput {
    #[serde(rename_all = "camelCase")]
    Mouse {
        r#type: String,
        x: f64,
        y: f64,
        #[serde(default)]
        button: Option<String>,
        #[serde(default)]
        click_count: Option<u32>,
        #[serde(default)]
        delta_x: Option<f64>,
        #[serde(default)]
        delta_y: Option<f64>,
        #[serde(default)]
        modifiers: Option<u32>,
    },
    #[serde(rename_all = "camelCase")]
    Key {
        r#type: String,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        code: Option<String>,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        windows_virtual_key_code: Option<u32>,
        #[serde(default)]
        modifiers: Option<u32>,
        #[serde(default)]
        auto_repeat: Option<bool>,
    },
    /// Clipboard paste: inserted as text, never replayed as key events.
    #[serde(rename_all = "camelCase")]
    Paste { text: String },
}

fn clamp_coord(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 32_000.0)
    } else {
        0.0
    }
}

/// Map a whitelisted input event to its CDP (method, params). These traverse
/// Chrome's real input pipeline, so the page sees trusted events — the same
/// anti-ban posture as typing into the popped-out window.
pub(super) fn input_command(input: &EmbeddedInput) -> Option<(&'static str, Value)> {
    match input {
        EmbeddedInput::Mouse {
            r#type,
            x,
            y,
            button,
            click_count,
            delta_x,
            delta_y,
            modifiers,
        } => {
            let kind = match r#type.as_str() {
                "mousePressed" | "mouseReleased" | "mouseMoved" | "mouseWheel" => r#type.as_str(),
                _ => return None,
            };
            let button = match button.as_deref() {
                None => "none",
                Some(b @ ("left" | "middle" | "right" | "none")) => b,
                Some(_) => return None,
            };
            let mut params = json!({
                "type": kind,
                "x": clamp_coord(*x),
                "y": clamp_coord(*y),
                "button": button,
                "modifiers": modifiers.unwrap_or(0).min(15),
            });
            if kind == "mousePressed" || kind == "mouseReleased" {
                params["clickCount"] = json!(click_count.unwrap_or(1).clamp(1, 3));
            }
            if kind == "mouseWheel" {
                params["deltaX"] = json!(clamp_coord(delta_x.unwrap_or(0.0).abs()) * delta_x.unwrap_or(0.0).signum());
                params["deltaY"] = json!(clamp_coord(delta_y.unwrap_or(0.0).abs()) * delta_y.unwrap_or(0.0).signum());
            }
            Some(("Input.dispatchMouseEvent", params))
        }
        EmbeddedInput::Key {
            r#type,
            key,
            code,
            text,
            windows_virtual_key_code,
            modifiers,
            auto_repeat,
        } => {
            let kind = match r#type.as_str() {
                "keyDown" | "keyUp" | "char" => r#type.as_str(),
                _ => return None,
            };
            if key.as_deref().map(|k| k.len() > 32).unwrap_or(false)
                || code.as_deref().map(|c| c.len() > 32).unwrap_or(false)
                || text.as_deref().map(|t| t.len() > 16).unwrap_or(false)
            {
                return None;
            }
            let mut params = json!({
                "type": kind,
                "modifiers": modifiers.unwrap_or(0).min(15),
                "autoRepeat": auto_repeat.unwrap_or(false),
            });
            if let Some(key) = key {
                params["key"] = json!(key);
            }
            if let Some(code) = code {
                params["code"] = json!(code);
            }
            if let Some(text) = text {
                params["text"] = json!(text);
                params["unmodifiedText"] = json!(text);
            }
            if let Some(vk) = windows_virtual_key_code {
                let vk = (*vk).min(255);
                params["windowsVirtualKeyCode"] = json!(vk);
                params["nativeVirtualKeyCode"] = json!(vk);
            }
            Some(("Input.dispatchKeyEvent", params))
        }
        EmbeddedInput::Paste { text } => {
            if text.is_empty() || text.len() > 16_384 {
                return None;
            }
            Some(("Input.insertText", json!({ "text": text })))
        }
    }
}

// ---------------------------------------------------------------------------
// Viewport → screencast constraints.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Viewport {
    pub max_width: u32,
    pub max_height: u32,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            max_width: 1440,
            max_height: 900,
        }
    }
}

/// Device-scaled screencast bounds: CSS size × devicePixelRatio, clamped so a
/// hostile renderer cannot request absurd frame sizes.
pub(super) fn viewport_for(css_width: u32, css_height: u32, scale: f64) -> Viewport {
    let scale = if scale.is_finite() { scale.clamp(1.0, 3.0) } else { 1.0 };
    let scaled = |v: u32, min: f64| -> u32 { ((v as f64) * scale).round().clamp(min, 3840.0) as u32 };
    Viewport {
        max_width: scaled(css_width.clamp(320, 3840), 320.0),
        max_height: scaled(css_height.clamp(240, 3840), 240.0),
    }
}

fn screencast_params(viewport: Viewport) -> Value {
    json!({
        "format": "jpeg",
        "quality": 70,
        "maxWidth": viewport.max_width,
        "maxHeight": viewport.max_height,
        "everyNthFrame": 1,
    })
}

// ---------------------------------------------------------------------------
// Embedded runtime handle, owned by BrowserState.
// ---------------------------------------------------------------------------

pub(crate) struct Embedded {
    pub client: Arc<CdpClient>,
    pub tabs: Mutex<Tabs>,
    pub viewport: Mutex<Viewport>,
    pub popped_out: Mutex<bool>,
    /// RealChromium generation this surface belongs to; a stale surface can
    /// never clear a newer one.
    pub generation: u64,
}

impl Embedded {
    pub(super) fn new(client: Arc<CdpClient>, generation: u64) -> Self {
        Self {
            client,
            tabs: Mutex::new(Tabs::default()),
            viewport: Mutex::new(Viewport::default()),
            popped_out: Mutex::new(false),
            generation,
        }
    }

    pub(crate) fn active_session(&self) -> AppResult<String> {
        self.tabs
            .lock()
            .ok()
            .and_then(|tabs| tabs.active_session())
            .ok_or_else(cdp_error)
    }

    pub(super) fn state_payload(&self) -> Value {
        let mut payload = self
            .tabs
            .lock()
            .map(|tabs| tabs.snapshot())
            .unwrap_or_else(|_| json!({ "tabs": [], "activeId": null }));
        payload["running"] = json!(true);
        payload["connected"] = json!(true);
        payload["poppedOut"] = json!(self.popped_out.lock().map(|p| *p).unwrap_or(false));
        payload
    }

    /// Same tab snapshot, but flagged as a transient disconnect: the browser
    /// is still running, the control socket dropped and is reconnecting. The
    /// renderer keeps the last frame and shows a slim "reconnecting" note
    /// rather than a hard error.
    pub(super) fn reconnecting_payload(&self) -> Value {
        let mut payload = self.state_payload();
        payload["connected"] = json!(false);
        payload
    }

    /// Stop then start the stream so Chrome emits a fresh keyframe now. Used
    /// after a navigation completes while the window is hidden (frames are
    /// otherwise coalesced and the canvas would stay stale/blank).
    pub(super) async fn restart_screencast(&self, session: &str) {
        self.client
            .fire(Some(session), "Page.stopScreencast", json!({}));
        self.start_screencast(session).await;
    }

    pub(super) async fn start_screencast(&self, session: &str) {
        let viewport = self.viewport.lock().map(|v| *v).unwrap_or_default();
        let _ = self
            .client
            .call(Some(session), "Page.startScreencast", screencast_params(viewport))
            .await;
    }

    pub(super) async fn activate(&self, target_id: &str) -> AppResult<()> {
        let (previous, next) = {
            let mut tabs = self.tabs.lock().map_err(|_| cdp_error())?;
            let previous = tabs.active_session();
            if !tabs.activate(target_id) {
                return Err(cdp_error());
            }
            (previous, tabs.session_of(target_id))
        };
        if let Some(previous) = previous.as_deref() {
            if Some(previous) != next.as_deref() {
                self.client
                    .fire(Some(previous), "Page.stopScreencast", json!({}));
            }
        }
        self.client
            .call(None, "Target.activateTarget", json!({ "targetId": target_id }))
            .await?;
        if let Some(session) = next.as_deref() {
            self.start_screencast(session).await;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests — protocol plumbing, tab reducer, input whitelist. No sockets.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn page(id: &str, url: &str, title: &str) -> Value {
        json!({ "targetId": id, "type": "page", "url": url, "title": title })
    }

    #[test]
    fn command_json_attaches_session_only_when_present() {
        let flat: Value =
            serde_json::from_str(&command_json(7, Some("S1"), "Page.navigate", json!({"url": "https://example.com/"}))).unwrap();
        assert_eq!(flat["id"], 7);
        assert_eq!(flat["method"], "Page.navigate");
        assert_eq!(flat["sessionId"], "S1");
        assert_eq!(flat["params"]["url"], "https://example.com/");
        let browser_level: Value =
            serde_json::from_str(&command_json(8, None, "Target.getTargets", json!({}))).unwrap();
        assert!(browser_level.get("sessionId").is_none());
    }

    #[test]
    fn classify_covers_responses_errors_events_and_garbage() {
        assert_eq!(
            classify(r#"{"id":3,"result":{"ok":true}}"#),
            Some(Incoming::Response { id: 3, result: Ok(json!({"ok": true})) })
        );
        assert_eq!(
            classify(r#"{"id":4,"error":{"message":"nope"}}"#),
            Some(Incoming::Response { id: 4, result: Err("nope".into()) })
        );
        assert_eq!(
            classify(r#"{"method":"Page.screencastFrame","sessionId":"S","params":{"sessionId":9}}"#),
            Some(Incoming::Event {
                method: "Page.screencastFrame".into(),
                session: Some("S".into()),
                params: json!({"sessionId": 9}),
            })
        );
        assert_eq!(classify("not json"), None);
        assert_eq!(classify(r#"{"neither":"nor"}"#), None);
    }

    /// Mock-websocket plumbing: a call is answered by routing the peer's
    /// response through `handle_incoming`; events go to the pump channel;
    /// unknown response ids are dropped without disturbing pending calls.
    #[tokio::test]
    async fn call_resolves_through_incoming_router() {
        let (client, mut wire) = CdpClient::new_for_test();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let caller = client.clone();
        let call = tokio::spawn(async move {
            caller.call(Some("S1"), "Page.navigate", json!({"url": "https://example.com/"})).await
        });
        let sent: Value = serde_json::from_str(&wire.recv().await.unwrap()).unwrap();
        let id = sent["id"].as_u64().unwrap();
        assert_eq!(sent["sessionId"], "S1");
        // A stray response for an unknown id must not resolve the call.
        client.handle_incoming(Incoming::Response { id: id + 999, result: Ok(json!("stray")) }, &event_tx);
        client.handle_incoming(
            Incoming::Event { method: "Target.targetCreated".into(), session: None, params: json!({}) },
            &event_tx,
        );
        client.handle_incoming(Incoming::Response { id, result: Ok(json!({"frameId": "F"})) }, &event_tx);
        let result = call.await.unwrap().unwrap();
        assert_eq!(result["frameId"], "F");
        assert_eq!(event_rx.recv().await.unwrap().method, "Target.targetCreated");
    }

    #[tokio::test]
    async fn call_fails_when_the_peer_reports_an_error_or_socket_dies() {
        let (client, mut wire) = CdpClient::new_for_test();
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let caller = client.clone();
        let call = tokio::spawn(async move { caller.call(None, "Target.getTargets", json!({})).await });
        let sent: Value = serde_json::from_str(&wire.recv().await.unwrap()).unwrap();
        client.handle_incoming(
            Incoming::Response { id: sent["id"].as_u64().unwrap(), result: Err("denied".into()) },
            &event_tx,
        );
        assert!(call.await.unwrap().is_err());
        // Socket death fails in-flight calls promptly.
        let caller = client.clone();
        let call = tokio::spawn(async move { caller.call(None, "Target.getTargets", json!({})).await });
        let _ = wire.recv().await.unwrap();
        client.fail_all_pending();
        assert!(call.await.unwrap().is_err());
    }

    #[test]
    fn tab_reducer_tracks_lifecycle_and_ignores_non_pages() {
        let mut tabs = Tabs::default();
        assert!(tabs.upsert(&page("T1", "https://example.com/", "Example")));
        assert!(!tabs.upsert(&page("T1", "https://example.com/a", "Example A")), "update, not new");
        assert!(!tabs.upsert(&json!({"targetId": "D", "type": "page", "url": "devtools://x"})));
        assert!(!tabs.upsert(&json!({"targetId": "W", "type": "service_worker", "url": "https://x/"})));
        assert!(!tabs.upsert(&json!({"targetId": "E", "type": "page", "url": "chrome-extension://x"})));
        assert!(tabs.upsert(&page("T2", "about:blank", "")));
        assert_eq!(tabs.len(), 2);
        tabs.attach("T1", "S1");
        tabs.attach("T2", "S2");
        assert!(tabs.activate("T1"));
        assert!(!tabs.activate("missing"));
        assert_eq!(tabs.active_session().as_deref(), Some("S1"));
        assert!(tabs.navigated("S2", "https://two.example/"));
        assert!(!tabs.navigated("S9", "https://nine.example/"));
        assert!(tabs.set_loading("S2", true));
        assert_eq!(tabs.tab_id_by_session("S2").as_deref(), Some("T2"));
        // Destroying the active tab clears activation; the reducer reports it
        // so the pump can promote the next tab.
        assert!(tabs.remove("T1"));
        assert_eq!(tabs.active_id(), None);
        assert_eq!(tabs.first_id().as_deref(), Some("T2"));
        assert!(!tabs.remove("T1"), "double remove is inert");
    }

    #[test]
    fn tab_snapshot_is_renderer_safe() {
        let mut tabs = Tabs::default();
        tabs.upsert(&page("T1", "https://example.com/", "Example"));
        tabs.attach("T1", "SECRET-SESSION");
        tabs.activate("T1");
        tabs.set_loading("SECRET-SESSION", true);
        let snapshot = tabs.snapshot();
        assert_eq!(snapshot["activeId"], "T1");
        assert_eq!(snapshot["tabs"][0]["id"], "T1");
        assert_eq!(snapshot["tabs"][0]["active"], true);
        assert_eq!(snapshot["tabs"][0]["loading"], true);
        // Session ids are a host-only capability handle: never serialized.
        assert!(!snapshot.to_string().contains("SECRET-SESSION"));
    }

    #[test]
    fn input_whitelist_maps_and_rejects() {
        let (method, params) = input_command(&EmbeddedInput::Mouse {
            r#type: "mousePressed".into(),
            x: 10.5,
            y: 20.0,
            button: Some("left".into()),
            click_count: Some(2),
            delta_x: None,
            delta_y: None,
            modifiers: Some(8),
        })
        .unwrap();
        assert_eq!(method, "Input.dispatchMouseEvent");
        assert_eq!(params["clickCount"], 2);
        assert_eq!(params["modifiers"], 8);
        let (_, wheel) = input_command(&EmbeddedInput::Mouse {
            r#type: "mouseWheel".into(),
            x: 5.0,
            y: 5.0,
            button: None,
            click_count: None,
            delta_x: Some(0.0),
            delta_y: Some(-120.0),
            modifiers: None,
        })
        .unwrap();
        assert_eq!(wheel["deltaY"], -120.0);
        assert_eq!(wheel["button"], "none");
        let (method, key) = input_command(&EmbeddedInput::Key {
            r#type: "keyDown".into(),
            key: Some("a".into()),
            code: Some("KeyA".into()),
            text: Some("a".into()),
            windows_virtual_key_code: Some(65),
            modifiers: Some(0),
            auto_repeat: None,
        })
        .unwrap();
        assert_eq!(method, "Input.dispatchKeyEvent");
        assert_eq!(key["windowsVirtualKeyCode"], 65);
        assert_eq!(key["unmodifiedText"], "a");
        let (method, paste) = input_command(&EmbeddedInput::Paste { text: "hello".into() }).unwrap();
        assert_eq!(method, "Input.insertText");
        assert_eq!(paste["text"], "hello");
        // Rejections: unknown types, oversized text, bogus buttons.
        assert!(input_command(&EmbeddedInput::Mouse {
            r#type: "mouseHover".into(), x: 0.0, y: 0.0, button: None,
            click_count: None, delta_x: None, delta_y: None, modifiers: None,
        }).is_none());
        assert!(input_command(&EmbeddedInput::Mouse {
            r#type: "mousePressed".into(), x: 0.0, y: 0.0, button: Some("back".into()),
            click_count: None, delta_x: None, delta_y: None, modifiers: None,
        }).is_none());
        assert!(input_command(&EmbeddedInput::Key {
            r#type: "keyDown".into(), key: None, code: None,
            text: Some("x".repeat(64)), windows_virtual_key_code: None, modifiers: None, auto_repeat: None,
        }).is_none());
        assert!(input_command(&EmbeddedInput::Paste { text: String::new() }).is_none());
        assert!(input_command(&EmbeddedInput::Paste { text: "x".repeat(20_000) }).is_none());
    }

    #[test]
    fn input_clamps_hostile_coordinates_and_modifiers() {
        let (_, params) = input_command(&EmbeddedInput::Mouse {
            r#type: "mouseMoved".into(),
            x: f64::NAN,
            y: 9e9,
            button: None,
            click_count: None,
            delta_x: None,
            delta_y: None,
            modifiers: Some(9_999),
        })
        .unwrap();
        assert_eq!(params["x"], 0.0);
        assert_eq!(params["y"], 32_000.0);
        assert_eq!(params["modifiers"], 15);
        let (_, pressed) = input_command(&EmbeddedInput::Mouse {
            r#type: "mousePressed".into(),
            x: 1.0,
            y: 1.0,
            button: Some("left".into()),
            click_count: Some(99),
            delta_x: None,
            delta_y: None,
            modifiers: None,
        })
        .unwrap();
        assert_eq!(pressed["clickCount"], 3);
    }

    #[test]
    fn viewport_scales_and_clamps() {
        assert_eq!(
            viewport_for(1200, 800, 2.0),
            Viewport { max_width: 2400, max_height: 1600 }
        );
        // Hostile values clamp instead of exploding.
        assert_eq!(
            viewport_for(20, 20, f64::INFINITY),
            Viewport { max_width: 320, max_height: 240 }
        );
        assert_eq!(
            viewport_for(100_000, 100_000, 3.0),
            Viewport { max_width: 3840, max_height: 3840 }
        );
        let params = screencast_params(Viewport::default());
        assert_eq!(params["format"], "jpeg");
        assert_eq!(params["quality"], 70);
        assert_eq!(params["maxWidth"], 1440);
    }

    #[test]
    fn embedded_state_payload_reports_running_and_pop_state() {
        let (client, _wire) = CdpClient::new_for_test();
        let embedded = Embedded::new(client, 3);
        embedded.tabs.lock().unwrap().upsert(&page("T1", "https://example.com/", "Example"));
        let payload = embedded.state_payload();
        assert_eq!(payload["running"], true);
        assert_eq!(payload["poppedOut"], false);
        assert_eq!(payload["tabs"][0]["id"], "T1");
        *embedded.popped_out.lock().unwrap() = true;
        assert_eq!(embedded.state_payload()["poppedOut"], true);
        assert_eq!(embedded.generation, 3);
    }

}
