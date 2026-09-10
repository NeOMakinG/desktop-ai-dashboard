//! Fixed, bounded Google projections. No bodies, snippets, attachments or writes.
use super::*;
use grants::{timestamp, ReadOperation, RuntimeReadContext};
use std::{collections::HashSet, future::Future, pin::Pin};
use tokio_util::sync::CancellationToken;

const MAX_BODY_BYTES: usize = 262_144;
const MAX_ITEMS: usize = 100;
const MAX_PAGES: usize = 3;
const WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1000;
const GMAIL_LIST: &str = "https://gmail.googleapis.com/gmail/v1/users/me/messages";
const CALENDAR_LIST: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";
const GMAIL_LIST_FIELDS: &str = "messages(id),nextPageToken";
const GMAIL_FIELDS: &str = "id,internalDate,labelIds,payload(headers(name,value))";
const CALENDAR_FIELDS: &str =
    "nextPageToken,items(id,summary,start(date,dateTime),end(date,dateTime),status)";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReadArgs {
    start_at: String,
    end_at: String,
    max_items: usize,
}
impl ReadArgs {
    fn parse(context: &RuntimeReadContext) -> AppResult<Self> {
        let args: Self =
            serde_json::from_value(context.args.clone()).map_err(|_| AppError::invalid())?;
        let start = timestamp(&args.start_at)?;
        let end = timestamp(&args.end_at)?;
        if !args.start_at.ends_with('Z')
            || !args.end_at.ends_with('Z')
            || end <= start
            || end - start > WINDOW_MS
            || args.max_items == 0
            || args.max_items > MAX_ITEMS
        {
            return Err(AppError::invalid());
        }
        // Metadata scope has no q/date search. Only scan a rolling recent inbox,
        // never widen into older mail or pretend a bounded scan is a full count.
        if context.operation == ReadOperation::GmailListMetadata {
            let now = chrono::Utc::now().timestamp_millis();
            if start < now - WINDOW_MS || end > now + 60_000 {
                return Err(AppError::invalid());
            }
        }
        Ok(args)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailMetadata {
    id: String,
    sender: String,
    subject: String,
    received_at: String,
    unread: bool,
    source_url: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    id: String,
    title: String,
    // All-day events retain YYYY-MM-DD and exclusive end date, never fabricated instants.
    start_at: String,
    end_at: String,
    all_day: bool,
}
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum ReadItems {
    Gmail(Vec<GmailMetadata>),
    Calendar(Vec<CalendarEvent>),
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorReadResult {
    kind: &'static str,
    retrieved_at: String,
    expires_at: String,
    start_at: String,
    end_at: String,
    truncated: bool,
    partial: bool,
    items: ReadItems,
}

// No Debug/Serialize: these are native-only credential leases, not tool payloads.
struct Request {
    url: reqwest::Url,
    access: Option<Zeroizing<String>>,
    refresh_form: Option<Vec<(&'static str, Zeroizing<String>)>>,
}
trait Transport: Send + Sync {
    fn send(
        &self,
        request: Request,
    ) -> Pin<Box<dyn Future<Output = AppResult<Zeroizing<Vec<u8>>>> + Send + '_>>;
}
struct GoogleTransport(reqwest::Client);
impl Transport for GoogleTransport {
    fn send(
        &self,
        request: Request,
    ) -> Pin<Box<dyn Future<Output = AppResult<Zeroizing<Vec<u8>>>> + Send + '_>> {
        Box::pin(async move {
            let mut builder = if let Some(form) = &request.refresh_form {
                if request.url.as_str() != oauth::TOKEN_ENDPOINT || request.access.is_some() {
                    return Err(AppError::invalid());
                }
                let fields: Vec<_> = form
                    .iter()
                    .map(|(name, value)| (*name, value.as_str()))
                    .collect();
                self.0.post(request.url.clone()).form(&fields)
            } else {
                let path = request.url.path();
                let gmail = request.url.host_str() == Some("gmail.googleapis.com")
                    && (path == "/gmail/v1/users/me/messages"
                        || path
                            .strip_prefix("/gmail/v1/users/me/messages/")
                            .is_some_and(valid_message_id));
                let calendar = request.url.host_str() == Some("www.googleapis.com")
                    && path == "/calendar/v3/calendars/primary/events";
                if request.url.scheme() != "https"
                    || request.url.port_or_known_default() != Some(443)
                    || !request.url.username().is_empty()
                    || request.url.password().is_some()
                    || request.url.fragment().is_some()
                    || !(gmail || calendar)
                {
                    return Err(AppError::invalid());
                }
                self.0.get(request.url.clone()).bearer_auth(
                    request
                        .access
                        .as_ref()
                        .ok_or_else(credential_error)?
                        .as_str(),
                )
            };
            builder = builder.header(reqwest::header::ACCEPT, "application/json");
            let mut response = builder.send().await.map_err(|_| network_error())?;
            if !response.status().is_success() {
                return Err(match response.status().as_u16() {
                    401 => AppError::new(
                        "connector_auth",
                        "Account access was refused. Reconnect the account.",
                    ),
                    403 => AppError::new(
                        "connector_scope",
                        "The account provider refused this bounded read.",
                    ),
                    429 => AppError::new(
                        "connector_rate_limit",
                        "The account provider is rate limiting reads. Try later.",
                    ),
                    _ => network_error(),
                });
            }
            let cap = if request.refresh_form.is_some() {
                oauth::MAX_RESPONSE_BYTES
            } else {
                MAX_BODY_BYTES
            };
            if response.content_length().is_some_and(|n| n > cap as u64) {
                return Err(data_error());
            }
            let mut bytes = Zeroizing::new(Vec::new());
            while let Some(chunk) = response.chunk().await.map_err(|_| network_error())? {
                if bytes.len().saturating_add(chunk.len()) > cap {
                    return Err(data_error());
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok(bytes)
        })
    }
}
fn network_error() -> AppError {
    AppError::new(
        "connector_network",
        "The account provider read could not complete.",
    )
}
fn data_error() -> AppError {
    AppError::new(
        "connector_data",
        "The account provider returned an unsupported or oversized metadata response.",
    )
}
fn valid_message_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_hexdigit())
}
fn valid_event_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 1024
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn text(value: String, cap: usize) -> AppResult<String> {
    if value.len() > cap || value.chars().any(|c| c.is_control() && c != '\t') {
        return Err(data_error());
    }
    Ok(value)
}
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> AppResult<T> {
    if bytes.len() > MAX_BODY_BYTES {
        return Err(data_error());
    }
    serde_json::from_slice(bytes).map_err(|_| data_error())
}
fn page_token(value: Option<String>, seen: &mut HashSet<String>) -> AppResult<Option<String>> {
    match value {
        Some(value)
            if value.is_empty()
                || value.len() > 2048
                || value.chars().any(char::is_control)
                || !seen.insert(value.clone()) =>
        {
            Err(data_error())
        }
        value => Ok(value),
    }
}

struct ReadLease {
    core: Arc<ConnectorsCore>,
    context: RuntimeReadContext,
    id: String,
    cancel: CancellationToken,
}
impl Drop for ReadLease {
    fn drop(&mut self) {
        if let Ok(mut life) = lifecycle_guard(&self.core) {
            life.grants.finish(&self.context.connection_id, &self.id);
        }
    }
}
fn check_connection(
    core: &ConnectorsCore,
    context: &RuntimeReadContext,
) -> AppResult<ConnectorStatus> {
    let status = store_guard(core)?
        .get(&context.connection_id)?
        .ok_or_else(grants::denied)?;
    if status.provider != "google" || !context.operation.scope_allowed(&status.scopes) {
        return Err(grants::denied());
    }
    Ok(status)
}
impl ReadLease {
    fn begin(core: &Arc<ConnectorsCore>, context: RuntimeReadContext) -> AppResult<Self> {
        let mut life = lifecycle_guard(core)?;
        life.grants
            .check(&context, chrono::Utc::now().timestamp_millis())?;
        check_connection(core, &context)?;
        let (id, cancel) = life
            .grants
            .begin(&context, chrono::Utc::now().timestamp_millis())?;
        Ok(Self {
            core: core.clone(),
            context,
            id,
            cancel,
        })
    }
    fn check(&self) -> AppResult<()> {
        let mut life = lifecycle_guard(&self.core)?;
        life.grants.check_read(
            &self.context,
            &self.id,
            chrono::Utc::now().timestamp_millis(),
        )?;
        check_connection(&self.core, &self.context)?;
        Ok(())
    }
    fn credential(&self) -> AppResult<(Zeroizing<String>, bool)> {
        let mut life = lifecycle_guard(&self.core)?;
        life.grants.check_read(
            &self.context,
            &self.id,
            chrono::Utc::now().timestamp_millis(),
        )?;
        let status = check_connection(&self.core, &self.context)?;
        let stored = read_stored(&self.core, &status.id)?;
        let expires = timestamp(&status.expires_at).map_err(|_| credential_error())?;
        Ok((
            Zeroizing::new(stored.access_token.clone()),
            expires <= chrono::Utc::now().timestamp_millis() + 60_000,
        ))
    }
    async fn request(
        &self,
        transport: &dyn Transport,
        request: Request,
    ) -> AppResult<Zeroizing<Vec<u8>>> {
        self.check()?;
        let milliseconds =
            timestamp(&self.context.expires_at)? - chrono::Utc::now().timestamp_millis();
        if milliseconds <= 0 {
            return Err(lifecycle::stale_error());
        }
        let response = tokio::select! {
            biased;
            _ = self.cancel.cancelled() => Err(lifecycle::stale_error()),
            _ = tokio::time::sleep(Duration::from_millis(milliseconds as u64)) => Err(lifecycle::stale_error()),
            result = transport.send(request) => result,
        }?;
        self.check()?;
        Ok(response)
    }
    async fn access(
        &self,
        transport: &dyn Transport,
        client_id: &str,
    ) -> AppResult<Zeroizing<String>> {
        let (access, refresh_needed) = self.credential()?;
        if !refresh_needed {
            return Ok(access);
        }
        drop(access);
        let (refresh_lease, refresh) = begin_refresh_checked(
            &self.core,
            &self.context.connection_id,
            Some((&self.context, &self.id)),
        )?;
        let request = Request {
            url: reqwest::Url::parse(oauth::TOKEN_ENDPOINT).map_err(|_| AppError::invalid())?,
            access: None,
            refresh_form: Some(vec![
                ("grant_type", Zeroizing::new("refresh_token".into())),
                ("client_id", Zeroizing::new(client_id.into())),
                ("refresh_token", refresh.clone()),
            ]),
        };
        let bytes = self.request(transport, request).await?;
        let tokens = oauth::parse_token_response(&bytes)?;
        self.check()?;
        commit_refresh_checked(
            &refresh_lease,
            &refresh,
            tokens,
            Some((&self.context, &self.id)),
        )?;
        drop(refresh_lease);
        self.check()?;
        let (access, expired) = self.credential()?;
        if expired {
            return Err(credential_error());
        }
        Ok(access)
    }
    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        transport: &dyn Transport,
        url: reqwest::Url,
        access: &str,
    ) -> AppResult<T> {
        let response = self
            .request(
                transport,
                Request {
                    url,
                    access: Some(Zeroizing::new(access.into())),
                    refresh_form: None,
                },
            )
            .await?;
        decode(&response)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailList {
    #[serde(default)]
    messages: Vec<GmailId>,
    next_page_token: Option<String>,
}
#[derive(Deserialize)]
struct GmailId {
    id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailMessage {
    id: String,
    internal_date: String,
    label_ids: Vec<String>,
    payload: GmailPayload,
}
#[derive(Deserialize)]
struct GmailPayload {
    #[serde(default)]
    headers: Vec<GmailHeader>,
}
#[derive(Deserialize)]
struct GmailHeader {
    name: String,
    value: String,
}
async fn gmail(
    lease: &ReadLease,
    args: &ReadArgs,
    transport: &dyn Transport,
    access: &str,
) -> AppResult<(ReadItems, bool, bool)> {
    let mut items = Vec::new();
    let mut scan_count = 0;
    let mut seen = HashSet::new();
    let mut pages = HashSet::new();
    let mut next: Option<String> = None;
    let mut truncated = false;
    let start = timestamp(&args.start_at)?;
    let end = timestamp(&args.end_at)?;
    for page in 0..MAX_PAGES {
        let remaining = MAX_ITEMS - scan_count;
        let mut url = reqwest::Url::parse(GMAIL_LIST).map_err(|_| AppError::invalid())?;
        url.query_pairs_mut()
            .append_pair("labelIds", "INBOX")
            .append_pair("includeSpamTrash", "false")
            .append_pair("maxResults", &remaining.to_string())
            .append_pair("fields", GMAIL_LIST_FIELDS);
        if let Some(token) = &next {
            url.query_pairs_mut().append_pair("pageToken", token);
        }
        let response: GmailList = lease.get(transport, url, access).await?;
        if response.messages.len() > remaining {
            return Err(data_error());
        }
        next = page_token(response.next_page_token, &mut pages)?;
        let count = response.messages.len();
        for (index, message) in response.messages.into_iter().enumerate() {
            if !valid_message_id(&message.id) || !seen.insert(message.id.clone()) {
                return Err(data_error());
            }
            let mut url = reqwest::Url::parse(&format!("{GMAIL_LIST}/{}", message.id))
                .map_err(|_| AppError::invalid())?;
            url.query_pairs_mut()
                .append_pair("format", "metadata")
                .append_pair("metadataHeaders", "From")
                .append_pair("metadataHeaders", "Subject")
                .append_pair("fields", GMAIL_FIELDS);
            let metadata: GmailMessage = lease.get(transport, url, access).await?;
            scan_count += 1;
            if metadata.id != message.id
                || metadata.label_ids.len() > 100
                || metadata.payload.headers.len() > 20
            {
                return Err(data_error());
            }
            let received: i64 = metadata.internal_date.parse().map_err(|_| data_error())?;
            if received < start
                || received >= end
                || !metadata.label_ids.iter().any(|l| l == "INBOX")
                || metadata
                    .label_ids
                    .iter()
                    .any(|l| l == "SPAM" || l == "TRASH")
            {
                continue;
            }
            let received_at = chrono::DateTime::from_timestamp_millis(received)
                .ok_or_else(data_error)?
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            let mut sender = None;
            let mut subject = None;
            for header in metadata.payload.headers {
                let target = if header.name.eq_ignore_ascii_case("from") {
                    &mut sender
                } else if header.name.eq_ignore_ascii_case("subject") {
                    &mut subject
                } else {
                    continue;
                };
                if target.is_some() {
                    return Err(data_error());
                }
                *target = Some(text(header.value, 1024)?);
            }
            items.push(GmailMetadata {
                source_url: format!("https://mail.google.com/mail/u/0/#inbox/{}", metadata.id),
                id: metadata.id,
                sender: sender.unwrap_or_default(),
                subject: subject.unwrap_or_default(),
                received_at,
                unread: metadata.label_ids.iter().any(|l| l == "UNREAD"),
            });
            if items.len() == args.max_items {
                truncated = index + 1 < count || next.is_some();
                break;
            }
        }
        if items.len() == args.max_items {
            break;
        }
        if next.is_none() {
            break;
        }
        if scan_count == MAX_ITEMS || page + 1 == MAX_PAGES {
            truncated = true;
            break;
        }
    }
    // The API supplies no metadata-scope date query or ordering guarantee. Never
    // claim newest-N/total count; partial signals an incomplete date-filtered scan.
    Ok((ReadItems::Gmail(items), truncated, truncated))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalendarList {
    #[serde(default)]
    items: Vec<ProviderEvent>,
    next_page_token: Option<String>,
}
#[derive(Deserialize)]
struct ProviderEvent {
    id: String,
    #[serde(default)]
    summary: String,
    start: EventTime,
    end: EventTime,
    status: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventTime {
    date: Option<String>,
    date_time: Option<String>,
}
impl EventTime {
    fn project(self) -> AppResult<(String, bool)> {
        match (self.date, self.date_time) {
            (Some(date), None) if date.len() == 10 => {
                chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").map_err(|_| data_error())?;
                Ok((date, true))
            }
            (None, Some(time)) => {
                timestamp(&time).map_err(|_| data_error())?;
                Ok((time, false))
            }
            _ => Err(data_error()),
        }
    }
}
async fn calendar(
    lease: &ReadLease,
    args: &ReadArgs,
    transport: &dyn Transport,
    access: &str,
) -> AppResult<(ReadItems, bool, bool)> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut pages = HashSet::new();
    let mut next: Option<String> = None;
    let mut scanned = 0;
    let mut truncated = false;
    for page in 0..MAX_PAGES {
        let remaining = args.max_items - scanned;
        let mut url = reqwest::Url::parse(CALENDAR_LIST).map_err(|_| AppError::invalid())?;
        url.query_pairs_mut()
            .append_pair("timeMin", &args.start_at)
            .append_pair("timeMax", &args.end_at)
            .append_pair("maxResults", &remaining.to_string())
            .append_pair("singleEvents", "true")
            .append_pair("orderBy", "startTime")
            .append_pair("showDeleted", "false")
            .append_pair("fields", CALENDAR_FIELDS);
        if let Some(token) = &next {
            url.query_pairs_mut().append_pair("pageToken", token);
        }
        let response: CalendarList = lease.get(transport, url, access).await?;
        if response.items.len() > remaining {
            return Err(data_error());
        }
        scanned += response.items.len();
        next = page_token(response.next_page_token, &mut pages)?;
        for event in response.items {
            if !valid_event_id(&event.id) || !seen.insert(event.id.clone()) {
                return Err(data_error());
            }
            if event.status == "cancelled" {
                continue;
            }
            if event.status != "confirmed" && event.status != "tentative" {
                return Err(data_error());
            }
            let (start_at, all_day) = event.start.project()?;
            let (end_at, end_all_day) = event.end.project()?;
            if all_day != end_all_day || (all_day && end_at <= start_at) {
                return Err(data_error());
            }
            if !all_day {
                let start = timestamp(&start_at)?;
                let end = timestamp(&end_at)?;
                if end <= start {
                    return Err(data_error());
                }
                if end <= timestamp(&args.start_at)? || start >= timestamp(&args.end_at)? {
                    continue;
                }
            }
            items.push(CalendarEvent {
                id: event.id,
                title: text(event.summary, 1024)?,
                start_at,
                end_at,
                all_day,
            });
        }
        if next.is_none() {
            break;
        }
        if scanned == args.max_items || page + 1 == MAX_PAGES {
            truncated = true;
            break;
        }
    }
    Ok((ReadItems::Calendar(items), truncated, truncated))
}

async fn dispatch(
    core: &Arc<ConnectorsCore>,
    context: RuntimeReadContext,
    transport: &dyn Transport,
    client_id: &str,
) -> AppResult<ConnectorReadResult> {
    let args = ReadArgs::parse(&context)?;
    let lease = ReadLease::begin(core, context)?;
    let access = lease.access(transport, client_id).await?;
    let (items, truncated, partial) = match lease.context.operation {
        ReadOperation::GmailListMetadata => gmail(&lease, &args, transport, &access).await?,
        ReadOperation::CalendarListEvents => calendar(&lease, &args, transport, &access).await?,
    };
    drop(access);
    let result = ConnectorReadResult {
        kind: match lease.context.operation {
            ReadOperation::GmailListMetadata => "gmailMetadata",
            ReadOperation::CalendarListEvents => "calendarEvents",
        },
        retrieved_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        // No native cache. Data must expire with this run request, not grant TTL.
        expires_at: lease.context.expires_at.clone(),
        start_at: args.start_at,
        end_at: args.end_at,
        truncated,
        partial,
        items,
    };
    if serde_json::to_vec(&result).map_err(|_| data_error())?.len() > MAX_BODY_BYTES {
        return Err(data_error());
    }
    lease.check()?;
    Ok(result)
}

/// Deliberately no environment flag or renderer argument can lift this gate.
/// Remote prompts/history/replay/Interface derivatives do not yet have verified
/// taint expiry and purge. Client ID and operator consent cannot substitute for it.
pub(super) fn live_gate() -> AppResult<&'static str> {
    let _ = client_id()?;
    Err(AppError::new("connector_live_gate", "Live Google reads are blocked pending Google compliance and verified runtime/model retention and revocation controls."))
}
pub async fn dispatch_runtime_read(
    app: &AppHandle,
    context: RuntimeReadContext,
) -> AppResult<ConnectorReadResult> {
    let client_id = live_gate()?; // before DB, credential store, or Google traffic
    let core = core(app)?;
    let transport = GoogleTransport(core.client.clone());
    dispatch(&core, context, &transport, client_id).await
}
pub fn validate_runtime_delivery(app: &AppHandle, context: &RuntimeReadContext) -> AppResult<()> {
    live_gate()?;
    let core = core(app)?;
    let mut life = lifecycle_guard(&core)?;
    life.grants
        .check(context, chrono::Utc::now().timestamp_millis())?;
    check_connection(&core, context)?;
    Ok(())
}

#[cfg(test)]
mod tests;
