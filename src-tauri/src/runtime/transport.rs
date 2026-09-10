use crate::{types::*, validation};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
    sync::{mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

const MAX_REQUEST: usize = 600_000;
const MAX_RESPONSE: usize = 400_000;
pub const LOCAL_BINDING: &str = "https://managed.forma.invalid";

pub struct Launch {
    pub python: PathBuf,
    pub controller: PathBuf,
    pub source: PathBuf,
    pub manifest: PathBuf,
    pub profile: PathBuf,
}
struct Call {
    frame: Value,
    reply: oneshot::Sender<AppResult<Value>>,
}
#[derive(Clone)]
pub struct Controller {
    tx: mpsc::Sender<Call>,
    alive: Arc<AtomicBool>,
    reaped: Arc<AtomicBool>,
    stop: CancellationToken,
}
#[derive(Default)]
pub struct ProcessOwner {
    inner: Mutex<OwnerState>,
}
#[derive(Default)]
struct OwnerState {
    stopping: bool,
    controller: Option<Controller>,
}
impl ProcessOwner {
    pub fn request_stop(&self) {
        let mut owner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        owner.stopping = true;
        if let Some(controller) = &owner.controller {
            controller.stop.cancel();
        }
    }
    pub async fn shutdown(&self) {
        let controller = {
            let mut owner = self
                .inner
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            owner.stopping = true;
            owner.controller.clone()
        };
        if let Some(controller) = controller {
            controller.shutdown().await;
        }
    }
}
impl Controller {
    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Acquire) && !self.stop.is_cancelled()
    }
    pub async fn launch(paths: Launch, owner: &ProcessOwner) -> AppResult<(Self, Value)> {
        let controller = {
            let mut slot = owner.inner.lock().map_err(|_| AppError::storage())?;
            if slot.stopping {
                return Err(not_ready());
            }
            let mut command = tokio::process::Command::new(&paths.python);
            command
                .args(["-I", "-B"])
                .arg(&paths.controller)
                .env_clear()
                .current_dir(&paths.profile)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            #[cfg(unix)]
            command.process_group(0);
            let mut child = command.spawn().map_err(|_| assets())?;
            let input = child.stdin.take().ok_or_else(network)?;
            let output = BufReader::new(child.stdout.take().ok_or_else(network)?);
            let (tx, rx) = mpsc::channel(32);
            let alive = Arc::new(AtomicBool::new(true));
            let reaped = Arc::new(AtomicBool::new(false));
            let stop = CancellationToken::new();
            let controller = Self {
                tx,
                alive: alive.clone(),
                reaped: reaped.clone(),
                stop: stop.clone(),
            };
            tokio::spawn(dispatch(child, input, output, rx, alive, reaped, stop));
            slot.controller = Some(controller.clone());
            controller
        };
        let boot = controller
            .control(json!({"op":"bootstrap", "profileDir":paths.profile,
            "sourceDir":paths.source,"pythonPath":paths.python,"manifestPath":paths.manifest}))
            .await;
        match boot {
            Ok(value) => Ok((controller, value)),
            Err(error) => {
                controller.shutdown().await;
                Err(error)
            }
        }
    }
    pub async fn control(&self, mut frame: Value) -> AppResult<Value> {
        if !self.alive() {
            return Err(network());
        }
        frame["id"] = json!(uuid::Uuid::new_v4().to_string());
        let (reply, result) = oneshot::channel();
        self.tx
            .try_send(Call { frame, reply })
            .map_err(|_| AppError::new("busy", "Hermes control queue is full or stopped."))?;
        result.await.map_err(|_| network())?
    }
    pub async fn shutdown(&self) {
        self.stop.cancel();
        while !self.reaped.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    #[cfg(test)]
    pub fn fixture() -> Self {
        let (tx, _) = mpsc::channel(1);
        Self {
            tx,
            alive: Arc::new(AtomicBool::new(true)),
            reaped: Arc::new(AtomicBool::new(true)),
            stop: CancellationToken::new(),
        }
    }
}
async fn dispatch(
    mut child: Child,
    mut input: ChildStdin,
    mut output: BufReader<ChildStdout>,
    mut rx: mpsc::Receiver<Call>,
    alive: Arc<AtomicBool>,
    reaped: Arc<AtomicBool>,
    stop: CancellationToken,
) {
    let mut secrets: Vec<Zeroizing<String>> = Vec::new();
    loop {
        let call = tokio::select! {
            biased;
            _ = stop.cancelled() => {
                let (reply, _) = oneshot::channel();
                Call { frame: json!({"id":uuid::Uuid::new_v4().to_string(),"op":"shutdown"}), reply }
            }
            call = rx.recv() => match call { Some(call) => call, None => break },
            _ = child.wait() => break,
        };
        let shutdown = call.frame["op"] == "shutdown";
        // A cancelled native future must not leave an undispatched admission in the queue.
        if !shutdown && call.reply.is_closed() {
            continue;
        }
        if let Some(secret) = call.frame["modelConfig"]["gatewayKey"]
            .as_str()
            .filter(|v| !v.is_empty())
        {
            if !secrets.iter().any(|s| s.as_str() == secret) {
                secrets.push(Zeroizing::new(secret.to_owned()));
            }
            if secrets.len() > 2 {
                secrets.remove(0);
            }
        }
        let exchange = tokio::time::timeout(
            Duration::from_secs(if shutdown { 8 } else { 30 }),
            exchange(&mut input, &mut output, call.frame),
        );
        let result = if shutdown {
            exchange.await.unwrap_or_else(|_| Err(network()))
        } else {
            tokio::select! { biased; _ = stop.cancelled() => Err(network()), result = exchange => result.unwrap_or_else(|_| Err(network())) }
        };
        let result = result.and_then(|value| {
            if secrets.iter().any(|s| contains_secret(&value, s)) {
                Err(protocol())
            } else {
                Ok(value)
            }
        });
        let broken = result.is_err();
        let _ = call.reply.send(result);
        if shutdown || broken {
            break;
        }
    }
    alive.store(false, Ordering::Release);
    drop(input);
    // EOF gives the controller a bounded chance to stop/reap its workers first.
    if tokio::time::timeout(Duration::from_secs(10), child.wait())
        .await
        .is_err()
    {
        #[cfg(unix)]
        if let Some(pid) = child.id() {
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    reaped.store(true, Ordering::Release);
    while let Ok(call) = rx.try_recv() {
        let _ = call.reply.send(Err(network()));
    }
}
async fn exchange(
    input: &mut ChildStdin,
    output: &mut BufReader<ChildStdout>,
    mut frame: Value,
) -> AppResult<Value> {
    let id = frame["id"].as_str().ok_or_else(protocol)?.to_owned();
    let mut bytes = Zeroizing::new(serde_json::to_vec(&frame).map_err(|_| protocol())?);
    if let Some(Value::String(secret)) = frame
        .get_mut("modelConfig")
        .and_then(|v| v.get_mut("gatewayKey"))
    {
        use zeroize::Zeroize;
        secret.zeroize();
    }
    if bytes.len() + 1 > MAX_REQUEST {
        return Err(protocol());
    }
    bytes.push(b'\n');
    input.write_all(&bytes).await.map_err(|_| network())?;
    input.flush().await.map_err(|_| network())?;
    let mut response = Zeroizing::new(Vec::new());
    loop {
        let buffer = output.fill_buf().await.map_err(|_| network())?;
        if buffer.is_empty() {
            return Err(network());
        }
        let end = buffer.iter().position(|b| *b == b'\n').map(|n| n + 1);
        let count = end.unwrap_or(buffer.len());
        if response.len() + count > MAX_RESPONSE {
            return Err(protocol());
        }
        response.extend_from_slice(&buffer[..count]);
        output.consume(count);
        if end.is_some() {
            break;
        }
    }
    decode_response(&response, &id)
}
fn decode_response(response: &[u8], id: &str) -> AppResult<Value> {
    if response.len() > MAX_RESPONSE {
        return Err(protocol());
    }
    let envelope: Value = serde_json::from_slice(response).map_err(|_| protocol())?;
    let object = envelope.as_object().ok_or_else(protocol)?;
    if object.len() != 3
        || !object.contains_key("id")
        || !object.contains_key("ok")
        || (!object.contains_key("result") && !object.contains_key("error"))
    {
        return Err(protocol());
    }
    if envelope["id"] != id {
        return Err(protocol());
    }
    if envelope["ok"] != true {
        return Err(AppError::new(
            "runtime_control",
            "Managed Hermes could not complete this operation. No fallback was used.",
        ));
    }
    envelope.get("result").cloned().ok_or_else(protocol)
}
#[derive(Clone)]
pub struct Session {
    pub endpoint: String,
    pub generation: u64,
    controller: Controller,
}
impl Session {
    pub fn managed(controller: Controller, generation: u64) -> AppResult<Self> {
        if !controller.alive() {
            return Err(not_ready());
        }
        Ok(Self {
            endpoint: LOCAL_BINDING.into(),
            generation,
            controller,
        })
    }
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> AppResult<T> {
        self.request("GET", path, None, None).await
    }
    pub async fn post<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        id: &str,
        body: &B,
    ) -> AppResult<T> {
        validation::id(id)?;
        self.request(
            "POST",
            path,
            Some(id),
            Some(serde_json::to_value(body).map_err(|_| AppError::invalid())?),
        )
        .await
    }
    async fn request<T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        id: Option<&str>,
        body: Option<Value>,
    ) -> AppResult<T> {
        if !path.starts_with("/v1/")
            || path.contains(['#', '%', '\\'])
            || path.contains("..")
            || path.contains("://")
        {
            return Err(AppError::invalid());
        }
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        let query: std::collections::BTreeMap<_, _> = query
            .split('&')
            .filter(|v| !v.is_empty())
            .map(|v| v.split_once('=').ok_or_else(AppError::invalid))
            .collect::<AppResult<_>>()?;
        let mut frame = json!({"op":"request","method":method,"path":path,"query":query});
        if let Some(id) = id {
            frame["idempotencyKey"] = json!(id);
        }
        if let Some(body) = body {
            frame["body"] = body;
        }
        let result = self.controller.control(frame).await?;
        match result["status"].as_u64().ok_or_else(protocol)? {
            200..=299 => (),
            401 | 403 => {
                return Err(AppError::new(
                    "runtime_auth",
                    "Hermes denied this device or operation. Check its grants.",
                ))
            }
            404 => {
                return Err(AppError::new(
                    "runtime_not_found",
                    "The Hermes object was not found.",
                ))
            }
            409 => {
                return Err(AppError::new(
                    "runtime_conflict",
                    "This Hermes object changed or has unresolved work.",
                ))
            }
            410 => {
                return Err(AppError::new(
                    "runtime_deleted",
                    "The Hermes object was deleted or expired.",
                ))
            }
            422 => {
                return Err(AppError::new(
                    "runtime_schema",
                    "Hermes rejected this request's contract.",
                ))
            }
            _ => return Err(network()),
        }
        serde_json::from_value(result.get("body").cloned().ok_or_else(protocol)?)
            .map_err(|_| protocol())
    }
}
pub fn contains_secret(value: &Value, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    match value {
        Value::String(s) => s.contains(token) || crate::hermes::escaped_contains(s, token),
        Value::Array(a) => a.iter().any(|v| contains_secret(v, token)),
        Value::Object(o) => o
            .iter()
            .any(|(k, v)| k.contains(token) || contains_secret(v, token)),
        _ => false,
    }
}
pub fn protocol() -> AppError {
    AppError::new(
        "runtime_protocol",
        "Managed Hermes returned an unsupported or unsafe response. No fallback was used.",
    )
}
pub fn network() -> AppError {
    AppError::new(
        "runtime_network",
        "Managed Hermes stopped or did not respond. Existing runs must be reconciled, not resent.",
    )
}
pub fn not_ready() -> AppError {
    AppError::new(
        "runtime_not_ready",
        "Hermes is not ready. Check your model configuration or retry starting Hermes.",
    )
}
pub fn assets() -> AppError {
    AppError::new("runtime_assets", "Bundled Hermes resources are missing or invalid. Rebuild or reinstall Forma; no system Python or direct fallback was used.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn explicit_quit_fences_a_late_start_before_any_process_spawn() {
        let owner = ProcessOwner::default();
        owner.request_stop();
        let missing = PathBuf::from("/nonexistent-forma-test-resource");
        let paths = Launch {
            python: missing.clone(),
            controller: missing.clone(),
            source: missing.clone(),
            manifest: missing.clone(),
            profile: missing,
        };
        let error = Controller::launch(paths, &owner).await.err().unwrap();
        assert_eq!(error.code, "runtime_not_ready");
        owner.shutdown().await;
    }
    #[test]
    fn private_control_envelope_is_correlated_bounded_and_not_an_http_response() {
        let id = uuid::Uuid::new_v4().to_string();
        let good = json!({"id":id,"ok":true,"result":{"status":200,"body":{"items":[]}}});
        assert_eq!(
            decode_response(&serde_json::to_vec(&good).unwrap(), &id).unwrap()["status"],
            200
        );
        for bad in [
            json!({"id":uuid::Uuid::new_v4().to_string(),"ok":true,"result":{}}),
            json!({"id":id,"ok":true,"result":{},"extra":"untrusted"}),
            json!({"status":200,"body":{}}),
        ] {
            assert!(decode_response(&serde_json::to_vec(&bad).unwrap(), &id).is_err());
        }
        assert!(decode_response(&vec![b' '; MAX_RESPONSE + 1], &id).is_err());
        let error = decode_response(
            &serde_json::to_vec(
                &json!({"id":id,"ok":false,"error":{"message":"PRIVATE_PATH_OR_KEY"}}),
            )
            .unwrap(),
            &id,
        )
        .unwrap_err();
        assert!(!error.message.contains("PRIVATE_PATH_OR_KEY"));
    }
    #[test]
    fn model_credentials_never_escape_as_decoded_or_escaped_resource_content() {
        for value in [
            json!({"text":"private-key"}),
            json!({"private-key":"hidden"}),
            json!({"spec":{"text":r"\u0070rivate-key"}}),
        ] {
            assert!(contains_secret(&value, "private-key"));
        }
        assert!(!contains_secret(&json!({"content":"safe"}), ""));
    }
}
