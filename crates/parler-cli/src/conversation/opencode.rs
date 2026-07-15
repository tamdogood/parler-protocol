use super::*;
use bytes::{Bytes, BytesMut};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming as HyperBody;
use hyper::{Method, Request, StatusCode};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

const API_RESPONSE_LIMIT: usize = 8 * 1024 * 1024;
const EVENT_BUFFER_LIMIT: usize = 8 * 1024 * 1024;
const API_START_TIMEOUT: Duration = Duration::from_secs(10);
const API_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MESSAGE_TAIL_LIMIT: u32 = 256;
const SEEN_ASSISTANT_LIMIT: usize = 1_024;

type ApiClient = Client<HttpConnector, Full<Bytes>>;

pub(super) async fn run(context: AdapterContext) -> Result<()> {
    let AdapterContext {
        options,
        identity,
        cwd,
        hub_override,
        mut sender,
    } = context;
    let mut host = OpenCodeHost::start(&cwd, options.resume.as_deref(), &identity, &sender).await?;
    let entry = enter_conversation(
        &mut sender,
        &options,
        &host.transcript,
        Host::Opencode.display(),
    )
    .await?;
    let ConversationEntry { room, initial, created, share } = entry;
    crate::save_active_session(&room)?;

    let mut tui = host.launch_tui(&cwd, &identity, &sender).await?;
    tokio::time::sleep(TUI_ATTACH_GRACE).await;
    if let Some(status) = tui.try_wait()? {
        bail!("interactive OpenCode exited before attaching to the live conversation ({status})");
    }

    let backlog = prepare_backlog(&mut sender, &room, &initial, created, Host::Opencode).await?;
    if let Some(prompt) = backlog.prompt {
        host.add_context(&prompt).await?;
        let cursor = backlog
            .commit_cursor
            .ok_or_else(|| anyhow!("prepared OpenCode backlog is missing its commit cursor"))?;
        sender.commit_reads_through(&room, cursor).await?;
    }

    announce_arrival(&mut sender, &room, created).await?;

    let receiver = crate::connect_with_hub(hub_override.as_deref()).await?;
    let (incoming_tx, incoming_rx) = mpsc::channel(1);
    let receive_task = tokio::spawn(receive_loop(receiver, room.clone(), incoming_tx));

    print_connected(&sender, Host::Opencode, share);

    let outcome = coordinate(&mut host, &mut tui, &mut sender, &room, incoming_rx).await;
    receive_task.abort();
    outcome
}

struct OpenCodeHost {
    server: Child,
    api: OpenCodeApi,
    events: OpenCodeEvents,
    session_id: String,
    transcript: String,
    known_assistants: RecentIds,
    pending: HashMap<String, Incoming>,
    next_message: u64,
}

impl OpenCodeHost {
    async fn start(
        cwd: &Path,
        resume: Option<&str>,
        identity: &TuiIdentity,
        agent: &MeshAgent,
    ) -> Result<OpenCodeHost> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);
        let base = format!("http://127.0.0.1:{port}");
        let inline = opencode_inline_config(
            std::env::var("OPENCODE_CONFIG_CONTENT").ok().as_deref(),
            identity,
            agent,
        )?;
        let authorization = basic_authorization(
            std::env::var("OPENCODE_SERVER_PASSWORD").ok().as_deref(),
            std::env::var("OPENCODE_SERVER_USERNAME").ok().as_deref(),
        );
        let mut command = Command::new("opencode");
        command
            .args(["serve", "--hostname", "127.0.0.1", "--port"])
            .arg(port.to_string())
            .current_dir(cwd)
            .env("OPENCODE_CONFIG_CONTENT", serde_json::to_string(&inline)?)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        configure_host_process(&mut command, identity, agent);
        let mut server = command
            .spawn()
            .context("failed to launch `opencode serve`; update OpenCode and retry")?;
        let api = OpenCodeApi::new(base, authorization);
        wait_for_api(&api, &mut server).await?;

        let session_id = match resume {
            Some(value) if value.eq_ignore_ascii_case("last") => api.latest_session(cwd).await?,
            Some(value) => {
                validate_session_id(value)?;
                api.require_session(value, cwd).await?;
                value.to_string()
            }
            None => api.create_session().await?,
        };
        validate_session_id(&session_id)?;
        // Subscribe before taking the canonical snapshot. A resumed turn that completes during
        // startup is therefore represented either in the snapshot or by a buffered status event.
        let events = api.events().await?;
        let messages = api.messages(&session_id).await?;
        let mut known_assistants = RecentIds::new(SEEN_ASSISTANT_LIMIT);
        known_assistants.extend(assistant_ids(&messages));
        let transcript = transcript_from_messages(&messages);
        Ok(OpenCodeHost {
            server,
            api,
            events,
            session_id,
            transcript,
            known_assistants,
            pending: HashMap::new(),
            next_message: 1,
        })
    }

    async fn launch_tui(&self, cwd: &Path, identity: &TuiIdentity, agent: &MeshAgent) -> Result<Child> {
        let mut command = Command::new("opencode");
        command
            .arg("attach")
            .arg(&self.api.base)
            .arg("--dir")
            .arg(cwd)
            .arg("--session")
            .arg(&self.session_id)
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        configure_host_process(&mut command, identity, agent);
        command.spawn().context("failed to launch the interactive OpenCode TUI")
    }

    fn message_id(&mut self) -> String {
        let id = format!("msg_parler_{}_{}", std::process::id(), self.next_message);
        self.next_message += 1;
        id
    }

    async fn add_context(&mut self, prompt: &str) -> Result<()> {
        let message_id = self.message_id();
        self.api
            .send_message(&self.session_id, &message_id, prompt, true)
            .await
            .context("OpenCode rejected the conversation catch-up context")
    }

    async fn start_turn(&mut self, incoming: Incoming, prompt: &str) -> Result<()> {
        let message_id = self.message_id();
        self.api
            .send_message(&self.session_id, &message_id, prompt, false)
            .await
            .context("OpenCode rejected a signed peer turn")?;
        self.pending.insert(message_id, incoming);
        Ok(())
    }

    async fn is_busy(&self) -> Result<bool> {
        self.api.is_busy(&self.session_id).await
    }

    async fn next_event(&mut self) -> Result<Value> {
        self.events.next().await
    }

    async fn completed(&mut self) -> Result<Vec<OpenCodeOutcome>> {
        let messages = self.api.messages(&self.session_id).await?;
        Ok(collect_completed_outcomes(&messages, &mut self.known_assistants))
    }
}

impl Drop for OpenCodeHost {
    fn drop(&mut self) {
        let _ = self.server.start_kill();
    }
}

struct OpenCodeOutcome {
    parent_id: String,
    answer: String,
    local_text: String,
    error: Option<String>,
}

fn collect_completed_outcomes(
    messages: &[Value],
    known_assistants: &mut RecentIds,
) -> Vec<OpenCodeOutcome> {
    let mut outcomes: Vec<OpenCodeOutcome> = Vec::new();
    let mut parent_indexes = HashMap::new();
    for message in messages {
        if message["info"]["role"] != "assistant" {
            continue;
        }
        let Some(id) = message["info"]["id"].as_str() else { continue };
        if known_assistants.contains(id) || message["info"]["time"]["completed"].is_null() {
            continue;
        }
        known_assistants.insert(id.to_string());
        let parent_id = message["info"]["parentID"].as_str().unwrap_or(id).to_string();
        let answer = text_parts(message, false);
        let local_text = user_message(messages, &parent_id)
            .and_then(|user| visible_user_transcript(&text_parts(user, true)))
            .map(|(speaker, prompt)| local_turn(speaker, &prompt, &answer))
            .unwrap_or_else(|| answer.clone());
        let error = message["info"].get("error").filter(|value| !value.is_null()).map(error_text);
        let outcome = OpenCodeOutcome { parent_id: parent_id.clone(), answer, local_text, error };
        if let Some(index) = parent_indexes.get(&parent_id).copied() {
            outcomes[index] = outcome;
        } else {
            parent_indexes.insert(parent_id, outcomes.len());
            outcomes.push(outcome);
        }
    }
    outcomes
}

struct OpenCodeEvents {
    body: HyperBody,
    buffer: BytesMut,
    scan_from: usize,
}

impl OpenCodeEvents {
    async fn next(&mut self) -> Result<Value> {
        loop {
            if let Some(event) = take_sse_event(&mut self.buffer, &mut self.scan_from)? {
                if !event.is_null() {
                    return Ok(event);
                }
                continue;
            }
            let frame = self
                .body
                .frame()
                .await
                .ok_or_else(|| anyhow!("OpenCode local event stream closed"))?
                .context("OpenCode local event stream failed")?;
            let Ok(data) = frame.into_data() else { continue };
            if self.buffer.len().saturating_add(data.len()) > EVENT_BUFFER_LIMIT {
                bail!("OpenCode local event exceeded the 8 MiB stream buffer limit");
            }
            self.buffer.extend_from_slice(&data);
        }
    }
}

fn take_sse_event(buffer: &mut BytesMut, scan_from: &mut usize) -> Result<Option<Value>> {
    let start = scan_from.saturating_sub(3);
    let mut boundary = None;
    for index in start..buffer.len() {
        if buffer[index..].starts_with(b"\r\n\r\n") {
            boundary = Some((index, 4));
            break;
        }
        if buffer[index..].starts_with(b"\n\n") {
            boundary = Some((index, 2));
            break;
        }
    }
    let Some((payload_len, delimiter_len)) = boundary else {
        *scan_from = buffer.len();
        return Ok(None);
    };
    let frame = buffer.split_to(payload_len + delimiter_len);
    *scan_from = 0;
    let text = std::str::from_utf8(&frame[..payload_len])
        .context("OpenCode local event stream returned non-UTF-8 data")?;
    let data = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|value| value.strip_prefix(' ').unwrap_or(value))
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() {
        return Ok(Some(Value::Null));
    }
    serde_json::from_str(&data)
        .map(Some)
        .context("OpenCode local event stream returned invalid JSON")
}

fn session_busy_event(event: &Value, session_id: &str) -> Option<bool> {
    if event["type"] != "session.status"
        || event["properties"]["sessionID"].as_str() != Some(session_id)
    {
        return None;
    }
    event["properties"]["status"]["type"]
        .as_str()
        .map(|status| status != "idle")
}

async fn coordinate(
    host: &mut OpenCodeHost,
    tui: &mut Child,
    sender: &mut MeshAgent,
    room: &str,
    mut incoming_rx: mpsc::Receiver<Incoming>,
) -> Result<()> {
    let mut queued: VecDeque<Incoming> = VecDeque::new();
    let mut busy = host.is_busy().await?;
    let mut presence_state = if busy { "working" } else { "waiting" };
    let mut heartbeat = tokio::time::interval(PRESENCE_HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;

    loop {
        if !busy && host.pending.is_empty() && !queued.is_empty() {
            // A human can submit from the attached TUI between event deliveries. Re-read canonical
            // status immediately before injection so a peer turn never races a local visible turn.
            busy = host.is_busy().await?;
            if !busy {
                let incoming = queued
                    .pop_front()
                    .ok_or_else(|| anyhow!("queued OpenCode turn disappeared"))?;
                let prompt = live_turn_prompt(sender, room, &incoming.message, Host::Opencode.display()).await;
                host.start_turn(incoming, &prompt).await?;
                busy = true;
            }
        }

        tokio::select! {
            status = tui.wait() => {
                let status = status?;
                if !status.success() {
                    bail!("interactive OpenCode exited with {status}");
                }
                return Ok(());
            }
            maybe = incoming_rx.recv() => {
                match maybe {
                    Some(incoming) => queued.push_back(incoming),
                    None => bail!("conversation receive loop stopped"),
                }
            }
            _ = heartbeat.tick() => {
                let _ = sender.presence(presence_state, Some(format!("live conversation '{room}'"))).await;
            }
            event = host.next_event() => {
                let event = event?;
                if let Some(event_busy) = session_busy_event(&event, &host.session_id) {
                    busy = event_busy;
                    if !busy {
                        for outcome in host.completed().await? {
                            if let Some(incoming) = host.pending.remove(&outcome.parent_id) {
                                if let Some(error) = outcome.error {
                                    drop(incoming);
                                    bail!("OpenCode could not finish an injected peer turn: {error}");
                                }
                                publish_turn(
                                    sender,
                                    room,
                                    TurnCapture { incoming: Some(incoming), text: outcome.answer },
                                )
                                .await?;
                            } else if outcome.error.is_none() && !outcome.local_text.trim().is_empty() {
                                publish_turn(
                                    sender,
                                    room,
                                    TurnCapture { incoming: None, text: outcome.local_text },
                                )
                                .await?;
                            }
                        }
                        // A new local prompt can race the idle notification. Reconcile once before
                        // allowing queued peer work into the native session.
                        busy = host.is_busy().await?;
                    }
                    let next_state = if busy { "working" } else { "waiting" };
                    if next_state != presence_state {
                        let _ = sender.presence(next_state, Some(format!("live conversation '{room}'"))).await;
                        presence_state = next_state;
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
struct OpenCodeApi {
    client: ApiClient,
    base: String,
    authorization: Option<String>,
}

impl OpenCodeApi {
    fn new(base: String, authorization: Option<String>) -> OpenCodeApi {
        let connector = HttpConnector::new();
        let client = Client::builder(TokioExecutor::new()).build(connector);
        OpenCodeApi { client, base, authorization }
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
        expected: &[StatusCode],
    ) -> Result<Value> {
        let bytes = match body {
            Some(value) => serde_json::to_vec(&value)?,
            None => Vec::new(),
        };
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.base))
            .header("content-type", "application/json")
            .header("accept", "application/json");
        if let Some(authorization) = &self.authorization {
            builder = builder.header("authorization", authorization);
        }
        let request = builder.body(Full::new(Bytes::from(bytes)))?;
        let response = tokio::time::timeout(API_REQUEST_TIMEOUT, self.client.request(request))
            .await
            .context("OpenCode local API request timed out")?
            .context("OpenCode local API request failed")?;
        let status = response.status();
        let body = tokio::time::timeout(
            API_REQUEST_TIMEOUT,
            Limited::new(response.into_body(), API_RESPONSE_LIMIT).collect(),
        )
            .await
            .context("OpenCode local API response timed out")?
            .map_err(|error| anyhow!("OpenCode local API response failed or exceeded 8 MiB: {error}"))?
            .to_bytes();
        if !expected.contains(&status) {
            let detail = String::from_utf8_lossy(&body);
            bail!("OpenCode local API returned {status}: {}", detail.trim());
        }
        if body.is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&body).context("OpenCode local API returned invalid JSON")
    }

    async fn health(&self) -> Result<()> {
        self.request(Method::GET, "/global/health", None, &[StatusCode::OK])
            .await?;
        Ok(())
    }

    async fn events(&self) -> Result<OpenCodeEvents> {
        let mut builder = Request::builder()
            .method(Method::GET)
            .uri(format!("{}/event", self.base))
            .header("accept", "text/event-stream");
        if let Some(authorization) = &self.authorization {
            builder = builder.header("authorization", authorization);
        }
        let request = builder.body(Full::new(Bytes::new()))?;
        let response = tokio::time::timeout(API_REQUEST_TIMEOUT, self.client.request(request))
            .await
            .context("OpenCode local event subscription timed out")?
            .context("OpenCode local event subscription failed")?;
        if response.status() != StatusCode::OK {
            bail!("OpenCode local event subscription returned {}", response.status());
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !content_type.starts_with("text/event-stream") {
            bail!("OpenCode local event subscription returned unexpected content type '{content_type}'");
        }
        Ok(OpenCodeEvents {
            body: response.into_body(),
            buffer: BytesMut::with_capacity(4 * 1024),
            scan_from: 0,
        })
    }

    async fn create_session(&self) -> Result<String> {
        let value = self
            .request(
                Method::POST,
                "/session",
                Some(json!({ "title": "Parler Protocol live conversation" })),
                &[StatusCode::OK],
            )
            .await?;
        value["id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow!("OpenCode created a session without an id"))
    }

    async fn latest_session(&self, cwd: &Path) -> Result<String> {
        let value = self
            .request(Method::GET, "/session?limit=50", None, &[StatusCode::OK])
            .await?;
        let expected = cwd.to_string_lossy();
        value
            .as_array()
            .into_iter()
            .flatten()
            .find(|session| session["directory"].as_str() == Some(expected.as_ref()))
            .and_then(|session| session["id"].as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("no resumable OpenCode session found in {}; omit --resume to start a new one", cwd.display()))
    }

    async fn require_session(&self, session_id: &str, cwd: &Path) -> Result<()> {
        let value = self
            .request(
                Method::GET,
                &format!("/session/{session_id}"),
                None,
                &[StatusCode::OK],
            )
            .await
            .with_context(|| format!("could not resume OpenCode session '{session_id}'"))?;
        if value["directory"].as_str() != Some(cwd.to_string_lossy().as_ref()) {
            bail!("OpenCode session '{session_id}' belongs to a different workspace");
        }
        Ok(())
    }

    async fn messages(&self, session_id: &str) -> Result<Vec<Value>> {
        let value = self
            .request(
                Method::GET,
                &message_tail_path(session_id),
                None,
                &[StatusCode::OK],
            )
            .await?;
        value
            .as_array()
            .cloned()
            .ok_or_else(|| anyhow!("OpenCode returned a non-array session transcript"))
    }

    async fn send_message(
        &self,
        session_id: &str,
        message_id: &str,
        prompt: &str,
        no_reply: bool,
    ) -> Result<()> {
        let path = if no_reply {
            format!("/session/{session_id}/message")
        } else {
            format!("/session/{session_id}/prompt_async")
        };
        let expected = if no_reply { StatusCode::OK } else { StatusCode::NO_CONTENT };
        self.request(
            Method::POST,
            &path,
            Some(json!({
                "messageID": message_id,
                "noReply": no_reply,
                "parts": [{ "type": "text", "text": prompt }]
            })),
            &[expected],
        )
        .await?;
        Ok(())
    }

    async fn is_busy(&self, session_id: &str) -> Result<bool> {
        let value = self
            .request(Method::GET, "/session/status", None, &[StatusCode::OK])
            .await?;
        Ok(value[session_id]["type"].as_str().is_some_and(|status| status != "idle"))
    }
}

fn message_tail_path(session_id: &str) -> String {
    format!("/session/{session_id}/message?limit={MESSAGE_TAIL_LIMIT}")
}

async fn wait_for_api(api: &OpenCodeApi, server: &mut Child) -> Result<()> {
    let deadline = tokio::time::Instant::now() + API_START_TIMEOUT;
    loop {
        if let Some(status) = server.try_wait()? {
            bail!("`opencode serve` exited before accepting the visible session ({status}); update OpenCode and retry");
        }
        match api.health().await {
            Ok(()) => return Ok(()),
            Err(error) if tokio::time::Instant::now() < deadline => {
                let _ = error;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => {
                return Err(error).context("OpenCode local API did not become ready; update OpenCode and retry");
            }
        }
    }
}

fn opencode_inline_config(
    existing: Option<&str>,
    identity: &TuiIdentity,
    agent: &MeshAgent,
) -> Result<Value> {
    let mut root = match existing.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => serde_json::from_str::<Value>(value)
            .context("OPENCODE_CONFIG_CONTENT is not valid JSON")?,
        None => json!({}),
    };
    let root = root
        .as_object_mut()
        .ok_or_else(|| anyhow!("OPENCODE_CONFIG_CONTENT must contain a JSON object"))?;
    let mcp = root.entry("mcp").or_insert_with(|| json!({}));
    let mcp = mcp
        .as_object_mut()
        .ok_or_else(|| anyhow!("OPENCODE_CONFIG_CONTENT.mcp must be a JSON object"))?;
    let executable = std::env::current_exe().context("could not locate the running parler binary")?;
    let environment = managed_host_environment(identity, agent);
    mcp.insert(
        "parler".into(),
        json!({
            "type": "local",
            "command": [executable, "mcp"],
            "enabled": true,
            "environment": environment,
        }),
    );
    Ok(Value::Object(root.clone()))
}

fn basic_authorization(password: Option<&str>, username: Option<&str>) -> Option<String> {
    let password = password.filter(|value| !value.is_empty())?;
    let username = username.filter(|value| !value.is_empty()).unwrap_or("opencode");
    Some(format!(
        "Basic {}",
        data_encoding::BASE64.encode(format!("{username}:{password}").as_bytes())
    ))
}

fn validate_session_id(value: &str) -> Result<()> {
    if value.starts_with("ses_")
        && value.len() <= 128
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Ok(());
    }
    bail!("invalid OpenCode session id '{value}'")
}

fn assistant_ids(messages: &[Value]) -> Vec<String> {
    messages
        .iter()
        .filter(|message| message["info"]["role"] == "assistant")
        // A resumed local turn may still be running when the adapter attaches. Keep it unseen so
        // its eventual terminal result is mirrored into the live conversation exactly once.
        .filter(|message| !message["info"]["time"]["completed"].is_null())
        .filter_map(|message| message["info"]["id"].as_str().map(str::to_string))
        .collect()
}

fn transcript_from_messages(messages: &[Value]) -> String {
    let mut lines = Vec::new();
    for message in messages {
        match message["info"]["role"].as_str() {
            Some("user") => {
                if let Some((speaker, text)) = visible_user_transcript(&text_parts(message, true)) {
                    lines.push(format!("{speaker}: {text}"));
                }
            }
            Some("assistant") if message["info"].get("error").is_none_or(Value::is_null) => {
                let text = text_parts(message, false);
                if !text.is_empty() {
                    lines.push(format!("Agent: {text}"));
                }
            }
            _ => {}
        }
    }
    clip_tail(&lines.join("\n\n"), MAX_CONTEXT_CHARS)
}

fn text_parts(message: &Value, user: bool) -> String {
    message["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|part| part["type"] == "text")
        .filter(|part| !part["ignored"].as_bool().unwrap_or(false))
        .filter(|part| !user || !part["synthetic"].as_bool().unwrap_or(false))
        .filter_map(|part| part["text"].as_str().map(str::trim))
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn user_message<'a>(messages: &'a [Value], id: &str) -> Option<&'a Value> {
    messages.iter().find(|message| {
        message["info"]["role"] == "user" && message["info"]["id"].as_str() == Some(id)
    })
}

fn local_turn(speaker: &str, prompt: &str, answer: &str) -> String {
    match (prompt.trim().is_empty(), answer.trim().is_empty()) {
        (true, _) => answer.trim().to_string(),
        (false, true) => format!("{speaker}: {}", prompt.trim()),
        (false, false) => format!("{speaker}: {}\n\nAgent: {}", prompt.trim(), answer.trim()),
    }
}

fn error_text(value: &Value) -> String {
    value["data"]["message"]
        .as_str()
        .or_else(|| value["message"].as_str())
        .unwrap_or("OpenCode turn failed")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use parler_connector::Config;

    fn identity() -> TuiIdentity {
        TuiIdentity { base_home: PathBuf::from("/tmp/parler"), terminal_session: "tty-1".into() }
    }

    fn agent() -> MeshAgent {
        let config = Config::create("ws://127.0.0.1:1", "open-agent", None).unwrap();
        MeshAgent::with_transport(
            Box::new(NoTransport),
            config.identity.id,
            config.name,
            config.role,
            config.hub_url,
        )
    }

    struct NoTransport;

    #[async_trait::async_trait]
    impl parler_connector::MeshTransport for NoTransport {
        async fn request(&mut self, _frame: parler_protocol::ClientFrame) -> Result<parler_protocol::ServerFrame> {
            bail!("not used")
        }
    }

    #[test]
    fn inline_config_preserves_user_values_and_overlays_exact_parler_identity() {
        let config = opencode_inline_config(
            Some(r#"{"model":"anthropic/sonnet","mcp":{"other":{"type":"remote","url":"https://m"}}}"#),
            &identity(),
            &agent(),
        )
        .unwrap();
        assert_eq!(config["model"], "anthropic/sonnet");
        assert_eq!(config["mcp"]["other"]["url"], "https://m");
        assert_eq!(config["mcp"]["parler"]["type"], "local");
        assert_eq!(
            config["mcp"]["parler"]["environment"]["PARLER_AGENT_SESSION"],
            "tty-1"
        );
        assert_eq!(config["mcp"]["parler"]["environment"]["PARLER_HUB"], "ws://127.0.0.1:1");
    }

    #[test]
    fn message_parser_separates_local_and_injected_turns() {
        let messages = json!([
            {
                "info":{"id":"msg_user","role":"user"},
                "parts":[{"type":"text","text":"Please inspect it."}]
            },
            {
                "info":{"id":"msg_answer","role":"assistant","parentID":"msg_user","time":{"completed":2}},
                "parts":[{"type":"text","text":"Inspection complete."}]
            },
            {
                "info":{"id":"msg_peer","role":"user"},
                "parts":[{"type":"text","text":"A cryptographically signed peer message arrived in your live Parler conversation 'r'.\nPEER MESSAGE:\n[2] peer: Fix it"}]
            }
        ]);
        let messages = messages.as_array().unwrap();
        assert_eq!(
            transcript_from_messages(messages),
            "User: Please inspect it.\n\nAgent: Inspection complete.\n\nPeer: [2] peer: Fix it"
        );
        assert_eq!(
            local_turn("User", "Please inspect it.", "Inspection complete."),
            "User: Please inspect it.\n\nAgent: Inspection complete."
        );
    }

    #[test]
    fn reconciliation_keeps_only_the_final_assistant_record_for_one_native_turn() {
        let messages = json!([
            {
                "info":{"id":"msg_user","role":"user"},
                "parts":[{"type":"text","text":"Please inspect it."}]
            },
            {
                "info":{"id":"msg_step","role":"assistant","parentID":"msg_user","time":{"completed":2}},
                "parts":[{"type":"text","text":"Intermediate answer."}]
            },
            {
                "info":{"id":"msg_final","role":"assistant","parentID":"msg_user","time":{"completed":3}},
                "parts":[{"type":"text","text":"Final answer."}]
            }
        ]);
        let mut known = RecentIds::new(SEEN_ASSISTANT_LIMIT);
        let outcomes = collect_completed_outcomes(messages.as_array().unwrap(), &mut known);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].answer, "Final answer.");
        assert_eq!(
            outcomes[0].local_text,
            "User: Please inspect it.\n\nAgent: Final answer."
        );
        assert!(known.contains("msg_step"));
        assert!(known.contains("msg_final"));
    }

    #[test]
    fn resumed_running_assistant_remains_unseen_until_completion() {
        let messages = json!([
            {"info":{"id":"done","role":"assistant","time":{"completed":2}}},
            {"info":{"id":"running","role":"assistant","time":{"completed":null}}}
        ]);
        let known = assistant_ids(messages.as_array().unwrap());
        assert!(known.iter().any(|id| id == "done"));
        assert!(!known.iter().any(|id| id == "running"));
    }

    #[test]
    fn transcript_reads_use_a_bounded_tail() {
        assert_eq!(
            message_tail_path("ses_abc"),
            format!("/session/ses_abc/message?limit={MESSAGE_TAIL_LIMIT}")
        );
    }

    #[test]
    fn sse_parser_handles_fragmented_crlf_frames_and_multiple_events() {
        let mut buffer = BytesMut::new();
        let mut scan_from = 0;
        buffer.extend_from_slice(
            b"data: {\"type\":\"server.connected\",\"properties\":{}}\r\n",
        );
        assert!(take_sse_event(&mut buffer, &mut scan_from).unwrap().is_none());
        buffer.extend_from_slice(
            b"\r\ndata: {\"type\":\"session.status\",\"properties\":{\"sessionID\":\"ses_1\",\"status\":{\"type\":\"idle\"}}}\n\n",
        );

        let connected = take_sse_event(&mut buffer, &mut scan_from).unwrap().unwrap();
        assert_eq!(connected["type"], "server.connected");
        let idle = take_sse_event(&mut buffer, &mut scan_from).unwrap().unwrap();
        assert_eq!(idle["properties"]["status"]["type"], "idle");
        assert!(buffer.is_empty());
    }

    #[test]
    fn session_status_events_are_filtered_and_conservative() {
        let idle = json!({
            "type": "session.status",
            "properties": {"sessionID": "ses_1", "status": {"type": "idle"}}
        });
        let retry = json!({
            "type": "session.status",
            "properties": {"sessionID": "ses_1", "status": {"type": "retry"}}
        });
        assert_eq!(session_busy_event(&idle, "ses_1"), Some(false));
        assert_eq!(session_busy_event(&retry, "ses_1"), Some(true));
        assert_eq!(session_busy_event(&idle, "ses_other"), None);
        assert_eq!(session_busy_event(&json!({"type": "server.heartbeat"}), "ses_1"), None);
    }

    #[test]
    fn resume_id_cannot_escape_the_local_api_path() {
        assert!(validate_session_id("ses_abc-123").is_ok());
        assert!(validate_session_id("../../config").is_err());
        assert!(validate_session_id("ses_%2Fadmin").is_err());
    }

    #[test]
    fn local_api_auth_preserves_the_opencode_server_policy() {
        assert_eq!(
            basic_authorization(Some("secret"), None).as_deref(),
            Some("Basic b3BlbmNvZGU6c2VjcmV0")
        );
        assert_eq!(
            basic_authorization(Some("secret"), Some("alice")).as_deref(),
            Some("Basic YWxpY2U6c2VjcmV0")
        );
        assert!(basic_authorization(None, Some("alice")).is_none());
    }
}
