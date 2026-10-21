//! The WebSocket matchmaking session.

use std::io::{Read, Write};
use std::net::{SocketAddrV4, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use dolphin_integrations::Log;
use slippi_user::UserManager;
use tungstenite::client::IntoClientRequest;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::protocol::{self, Candidate, DebugOverrides};
use crate::stun::classify_nat;

const PROD_URL: &str = "wss://matchmaking.slippi.gg/v1/queue";
const DEV_URL: &str = "wss://matchmaking-dev.slippi.gg/v1/queue";

/// Environment variable that overrides the service URL, for local testing
/// against a service started with authentication disabled.
pub const URL_ENV_VAR: &str = "SLIPPI_MM_URL";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_READ_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_READ_TIMEOUT: Duration = Duration::from_millis(250);
const CONNECT_ATTEMPTS: u32 = 3;
/// How long after losing a queued ticket's connection we keep trying to
/// resume. The service drops a silent ticket after thirty seconds.
const RESUME_WINDOW: Duration = Duration::from_secs(15);
const RETRY_DELAY: Duration = Duration::from_secs(1);
/// How long a connection may go without the service acknowledging its join
/// or resume, matching the previous client's join timeout.
const JOIN_TIMEOUT: Duration = Duration::from_secs(5);
/// The service sends a status message every few seconds while a ticket is
/// queued. After this long without any frame we send a ping, and after
/// `LIVENESS_TIMEOUT` we treat the connection as lost. Without this a network
/// that drops silently would leave the search running forever while the
/// service had long since dropped the ticket.
const PING_AFTER: Duration = Duration::from_secs(5);
const LIVENESS_TIMEOUT: Duration = Duration::from_secs(15);

/// Where a matchmaking session is. The numeric values cross the FFI.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchmakingState {
    Idle = 0,
    Connecting = 1,
    Queued = 2,
    Matched = 3,
    Failed = 4,
}

impl MatchmakingState {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => MatchmakingState::Connecting,
            2 => MatchmakingState::Queued,
            3 => MatchmakingState::Matched,
            4 => MatchmakingState::Failed,
            _ => MatchmakingState::Idle,
        }
    }
}

/// What the Dolphin side knows when it asks for a match.
#[derive(Clone, Debug, Default)]
pub struct MatchRequest {
    /// 0 ranked, 1 unranked, 2 direct, 3 teams, 4 party.
    pub mode: u8,
    /// Opponent connect code or lobby code as raw Shift-JIS bytes.
    pub connect_code: Vec<u8>,
    /// Local UDP port the ENet socket is bound to.
    pub netplay_port: u16,
    /// "ip:port" on the local network, or empty.
    pub lan_addr: String,
    /// The public address each of two STUN servers reported for the netplay
    /// socket, in the order they were asked. None where a server did not
    /// answer. Two answers are needed to classify the NAT.
    pub stun: [Option<SocketAddrV4>; 2],
    /// Only honored by a service running with authentication disabled.
    pub debug: Option<DebugOverrides>,
}

#[derive(Debug, Default)]
struct Shared {
    state: AtomicU8,
    cancel: AtomicBool,
    result_json: Mutex<Option<String>>,
    error: Mutex<String>,
}

impl Shared {
    fn set_state(&self, s: MatchmakingState) {
        self.state.store(s as u8, Ordering::SeqCst);
    }

    fn state(&self) -> MatchmakingState {
        MatchmakingState::from_u8(self.state.load(Ordering::SeqCst))
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn fail(&self, message: impl Into<String>) {
        let message = message.into();
        tracing::error!(target: Log::SlippiOnline, "[Matchmaking] {}", message);
        *self.error.lock().unwrap() = message;
        self.set_state(MatchmakingState::Failed);
    }
}

/// A matchmaking client. One session runs at a time; starting a new one ends
/// the previous session.
#[derive(Debug)]
pub struct MatchmakingClient {
    user_manager: UserManager,
    slippi_semver: String,
    url: String,
    inner: Mutex<Inner>,
}

/// The part that changes as sessions start and end. It sits behind a mutex
/// because the Dolphin side reaches it from more than one thread: an old
/// matchmaking object winding down on a cleanup thread while a new one starts
/// the next search.
#[derive(Debug)]
struct Inner {
    shared: Arc<Shared>,
    /// The worker is never joined, so cancelling does not wait on a connect
    /// or handshake in progress. The handle is kept only until the next
    /// session replaces it.
    worker: Option<JoinHandle<()>>,
    session_id: u64,
}

impl MatchmakingClient {
    /// Creates a client. The service URL comes from `SLIPPI_MM_URL` when set,
    /// otherwise the dev or prod service depending on the build version.
    pub fn new(user_manager: UserManager, slippi_semver: String) -> Self {
        let url = std::env::var(URL_ENV_VAR).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| {
            if slippi_semver.contains("dev") {
                DEV_URL.to_string()
            } else {
                PROD_URL.to_string()
            }
        });

        Self {
            user_manager,
            slippi_semver,
            url,
            inner: Mutex::new(Inner {
                shared: Arc::new(Shared::default()),
                worker: None,
                session_id: 0,
            }),
        }
    }

    /// The URL this client connects to.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Begins a session and returns its id. Any previous session is cancelled
    /// first.
    pub fn start(&self, request: MatchRequest) -> u64 {
        let join = self.build_join(&request);
        let url = self.url.clone();
        let user_manager = self.user_manager.clone();

        let mut inner = self.inner.lock().unwrap();
        Self::cancel_inner(&mut inner);

        inner.session_id += 1;
        let shared = Arc::new(Shared::default());
        shared.set_state(MatchmakingState::Connecting);
        inner.shared = shared.clone();

        tracing::info!(target: Log::SlippiOnline, "[Matchmaking] Starting session against {}", url);

        inner.worker = Some(thread::spawn(move || {
            Session {
                shared,
                url,
                join,
                user_manager,
                ticket: None,
                resume_deadline: None,
                acknowledged: false,
            }
            .run();
        }));

        inner.session_id
    }

    /// Cancels the session with the given id. Ids from earlier sessions are
    /// ignored, so tearing down an old caller cannot end a newer search.
    pub fn cancel_session(&self, session_id: u64) {
        let mut inner = self.inner.lock().unwrap();
        if session_id == inner.session_id {
            Self::cancel_inner(&mut inner);
        }
    }

    /// Current state of the session.
    pub fn state(&self) -> MatchmakingState {
        self.inner.lock().unwrap().shared.state()
    }

    /// The match message as the service sent it, once the state is Matched.
    /// Each match is returned once.
    pub fn take_result_json(&self) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        let taken = inner.shared.result_json.lock().unwrap().take();
        taken
    }

    /// The failure reason, once the state is Failed.
    pub fn error_message(&self) -> String {
        let inner = self.inner.lock().unwrap();
        let message = inner.shared.error.lock().unwrap().clone();
        message
    }

    /// Leaves the queue and ends the session. Safe to call at any time and
    /// never blocks on the network.
    pub fn cancel(&self) {
        let mut inner = self.inner.lock().unwrap();
        Self::cancel_inner(&mut inner);
    }

    /// Flags the running session to stop. The worker notices within one read
    /// timeout, sends `cancel` if it is connected, and exits on its own. It is
    /// not joined: a worker stuck in a connect or handshake would otherwise
    /// hold up the caller for many seconds, and it touches nothing but its
    /// own shared state once flagged.
    fn cancel_inner(inner: &mut Inner) {
        inner.shared.cancel.store(true, Ordering::SeqCst);
        drop(inner.worker.take());
    }

    fn build_join(&self, request: &MatchRequest) -> protocol::Join {
        let user = self.user_manager.get(|user| protocol::UserInfo {
            uid: user.uid.clone(),
            play_key: user.play_key.clone(),
            connect_code: user.connect_code.clone(),
            display_name: user.display_name.clone(),
        });

        // The NAT is classified from what two servers saw. The addresses themselves are
        // deliberately kept out of the log.
        let nat_type = classify_nat(request.stun[0], request.stun[1]);
        tracing::info!(target: Log::SlippiOnline, "[Matchmaking] NAT type: {}", nat_type.as_str());

        let mut candidates = Vec::new();
        let mut port = request.netplay_port;
        if let Some(reflexive) = request.stun.iter().flatten().next() {
            candidates.push(Candidate {
                ip: reflexive.ip().to_string(),
                port: reflexive.port(),
                kind: "srflx".into(),
            });
            port = reflexive.port();
        }
        if let Some((ip, lan_port)) = request.lan_addr.rsplit_once(':') {
            if let Ok(lan_port) = lan_port.parse::<u16>() {
                candidates.push(Candidate {
                    ip: ip.to_string(),
                    port: lan_port,
                    kind: "host".into(),
                });
            }
        }

        protocol::Join {
            kind: "join",
            user,
            search: protocol::Search {
                mode: request.mode,
                connect_code: request.connect_code.clone(),
            },
            app_version: self.slippi_semver.clone(),
            port,
            lan_addr: request.lan_addr.clone(),
            candidates,
            nat_type: nat_type.as_str().into(),
            debug: request.debug.clone(),
        }
    }
}

impl Drop for MatchmakingClient {
    fn drop(&mut self) {
        self.cancel();
    }
}

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

/// Why a connection ended, as seen by the session loop.
enum Ended {
    /// The session reached a terminal state; nothing more to do.
    Done,
    /// The connection dropped while the ticket may still be queued.
    Lost,
    /// The service never acknowledged the join.
    JoinTimeout,
}

struct Session {
    shared: Arc<Shared>,
    url: String,
    join: protocol::Join,
    user_manager: UserManager,
    /// Ticket ID and token once the service has accepted us.
    ticket: Option<(String, String)>,
    /// Until when a lost connection is worth resuming. Set when the connection
    /// drops with a ticket in hand and cleared when the service takes us back.
    resume_deadline: Option<Instant>,
    /// Whether the current connection's join or resume has been acknowledged.
    acknowledged: bool,
}

impl Session {
    fn run(mut self) {
        let mut connect_failures = 0;

        loop {
            if self.shared.cancelled() {
                self.shared.set_state(MatchmakingState::Idle);
                return;
            }

            let mut socket = match connect(&self.url, &self.shared) {
                Ok(socket) => socket,
                Err(ConnectError::Cancelled) => {
                    self.shared.set_state(MatchmakingState::Idle);
                    return;
                },
                Err(ConnectError::Failed(err)) => {
                    tracing::warn!(target: Log::SlippiOnline, "[Matchmaking] Connect failed: {}", err);
                    connect_failures += 1;
                    if self.give_up(connect_failures) || self.sleep_unless_cancelled(RETRY_DELAY) {
                        return;
                    }
                    continue;
                },
            };
            connect_failures = 0;

            let first = match &self.ticket {
                Some((ticket_id, token)) => serde_json::to_string(&protocol::Resume {
                    kind: "resume",
                    ticket_id: ticket_id.clone(),
                    token: token.clone(),
                }),
                None => serde_json::to_string(&self.join),
            }
            .expect("protocol message serializes");

            if let Err(err) = socket.send(Message::text(first)) {
                tracing::warn!(target: Log::SlippiOnline, "[Matchmaking] Failed to send first message: {}", err);
                let _ = socket.close(None);
                connect_failures += 1;
                if self.give_up(connect_failures) || self.sleep_unless_cancelled(RETRY_DELAY) {
                    return;
                }
                continue;
            }

            match self.serve(&mut socket) {
                Ended::Done => return,
                Ended::JoinTimeout => {
                    self.shared.fail("Failed to join mm queue");
                    return;
                },
                Ended::Lost => {
                    if self.ticket.is_none() {
                        self.shared.fail("Lost connection to the mm server");
                        return;
                    }
                    match self.resume_deadline {
                        None => self.resume_deadline = Some(Instant::now() + RESUME_WINDOW),
                        Some(deadline) if Instant::now() >= deadline => {
                            self.shared.fail("Lost connection to the mm server");
                            return;
                        },
                        Some(_) => {},
                    }
                    tracing::warn!(target: Log::SlippiOnline, "[Matchmaking] Connection lost, resuming ticket");
                    if self.sleep_unless_cancelled(RETRY_DELAY) {
                        return;
                    }
                },
            }
        }
    }

    /// Decides whether a failed connect ends the session: past the resume
    /// window when resuming, after a few attempts when first joining. Fails
    /// the session when it does.
    fn give_up(&self, connect_failures: u32) -> bool {
        let give_up = match self.resume_deadline {
            Some(deadline) => Instant::now() >= deadline,
            None => connect_failures >= CONNECT_ATTEMPTS,
        };
        if give_up {
            self.shared.fail(if self.ticket.is_some() {
                "Lost connection to the mm server"
            } else {
                "Failed to connect to mm server"
            });
        }
        give_up
    }

    /// Runs one connection until the session ends or the connection drops.
    fn serve(&mut self, socket: &mut Socket) -> Ended {
        let opened = Instant::now();
        let mut last_frame = opened;
        let mut pinged = false;
        self.acknowledged = false;

        loop {
            if self.shared.cancelled() {
                let _ = socket.send(Message::text(
                    serde_json::to_string(&protocol::Simple { kind: "cancel" }).unwrap(),
                ));
                let _ = socket.close(None);
                self.shared.set_state(MatchmakingState::Idle);
                return Ended::Done;
            }

            match socket.read() {
                Ok(Message::Text(text)) => {
                    last_frame = Instant::now();
                    pinged = false;
                    if let Some(ended) = self.handle(text.as_str(), socket) {
                        return ended;
                    }
                },
                Ok(Message::Close(frame)) => {
                    tracing::info!(target: Log::SlippiOnline, "[Matchmaking] Server closed connection: {:?}", frame);
                    return Ended::Lost;
                },
                Ok(_) => {
                    // Binary, ping, and pong frames carry nothing for us but
                    // prove the service is there. tungstenite queues pong
                    // replies; flush sends them.
                    last_frame = Instant::now();
                    pinged = false;
                    let _ = socket.flush();
                },
                Err(tungstenite::Error::Io(err)) if is_timeout(&err) => {
                    let _ = socket.flush();
                },
                Err(err) => {
                    tracing::warn!(target: Log::SlippiOnline, "[Matchmaking] Read failed: {}", err);
                    return Ended::Lost;
                },
            }

            if !self.acknowledged && opened.elapsed() >= JOIN_TIMEOUT {
                tracing::warn!(target: Log::SlippiOnline, "[Matchmaking] Service did not acknowledge the connection in time");
                let _ = socket.close(None);
                return if self.ticket.is_none() {
                    Ended::JoinTimeout
                } else {
                    Ended::Lost
                };
            }

            let idle = last_frame.elapsed();
            if idle >= LIVENESS_TIMEOUT {
                tracing::warn!(target: Log::SlippiOnline, "[Matchmaking] Nothing from the service for {:?}, connection lost", idle);
                let _ = socket.close(None);
                return Ended::Lost;
            }
            if idle >= PING_AFTER && !pinged {
                pinged = true;
                let _ = socket.send(Message::text(
                    serde_json::to_string(&protocol::Simple { kind: "ping" }).unwrap(),
                ));
            }
        }
    }

    /// Handles one server message. Returns Some when the connection is done.
    fn handle(&mut self, text: &str, socket: &mut Socket) -> Option<Ended> {
        let envelope: protocol::Envelope = match serde_json::from_str(text) {
            Ok(env) => env,
            Err(err) => {
                tracing::warn!(target: Log::SlippiOnline, "[Matchmaking] Ignoring malformed message: {}", err);
                return None;
            },
        };

        match envelope.kind.as_str() {
            "joined" => {
                if let Ok(joined) = serde_json::from_str::<protocol::Joined>(text) {
                    tracing::info!(target: Log::SlippiOnline, "[Matchmaking] Ticket {} accepted", joined.ticket_id);
                    self.ticket = Some((joined.ticket_id, joined.token));
                    self.acknowledged = true;
                    // The service took us back, so the next drop gets a full
                    // resume window of its own.
                    self.resume_deadline = None;
                    self.shared.set_state(MatchmakingState::Queued);
                }
                None
            },
            "status" => {
                if let Ok(status) = serde_json::from_str::<protocol::Status>(text) {
                    tracing::debug!(target: Log::SlippiOnline, "[Matchmaking] In queue for {:.1}s", status.seconds_in_queue);
                }
                None
            },
            "match" => {
                match serde_json::from_str::<protocol::MatchSummary>(text) {
                    Ok(summary) => tracing::info!(
                        target: Log::SlippiOnline,
                        "[Matchmaking] Match found: {} (host: {})",
                        summary.match_id,
                        summary.is_host
                    ),
                    Err(err) => tracing::warn!(target: Log::SlippiOnline, "[Matchmaking] Match message unreadable: {}", err),
                }
                *self.shared.result_json.lock().unwrap() = Some(text.to_string());
                self.shared.set_state(MatchmakingState::Matched);
                let _ = socket.close(None);
                Some(Ended::Done)
            },
            "error" => {
                let message = match serde_json::from_str::<protocol::ErrorMessage>(text) {
                    Ok(err) => {
                        if !err.latest_version.is_empty() {
                            // The service knows a newer build exists. Remember it
                            // for players whose update check is broken.
                            self.user_manager.overwrite_latest_version(err.latest_version);
                        }
                        err.error
                    },
                    Err(_) => "Received an error from the mm server".to_string(),
                };
                self.shared.fail(message);
                let _ = socket.close(None);
                Some(Ended::Done)
            },
            _ => None,
        }
    }

    /// Sleeps in small steps so a cancel is noticed. Returns true if cancelled.
    fn sleep_unless_cancelled(&self, total: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < total {
            if self.shared.cancelled() {
                self.shared.set_state(MatchmakingState::Idle);
                return true;
            }
            thread::sleep(Duration::from_millis(50));
        }
        false
    }
}

fn is_timeout(err: &std::io::Error) -> bool {
    matches!(err.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
}

enum ConnectError {
    /// The session was cancelled while connecting.
    Cancelled,
    Failed(String),
}

/// Opens a WebSocket to `url` over IPv4 only. Netplay is IPv4, so the address
/// the service observes must be the IPv4 one. The cancel flag is checked
/// between address attempts so a cancel does not wait out every timeout.
fn connect(url: &str, shared: &Shared) -> Result<Socket, ConnectError> {
    let failed = |e: String| ConnectError::Failed(e);
    let request = url.into_client_request().map_err(|e| failed(e.to_string()))?;
    let uri = request.uri().clone();
    let host = uri.host().ok_or_else(|| failed("matchmaking URL has no host".to_string()))?;
    let secure = uri.scheme_str() == Some("wss");
    let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });

    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|e| failed(format!("resolving {}: {}", host, e)))?
        .filter(|a| a.is_ipv4())
        .collect();
    if addrs.is_empty() {
        return Err(failed(format!("no IPv4 address for {}", host)));
    }

    let mut last_err = String::new();
    for addr in addrs {
        if shared.cancelled() {
            return Err(ConnectError::Cancelled);
        }
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                let _ = stream.set_read_timeout(Some(HANDSHAKE_READ_TIMEOUT));
                let (socket, _response) = tungstenite::client_tls(request, stream).map_err(|e| failed(e.to_string()))?;
                set_read_timeout(&socket, POLL_READ_TIMEOUT);
                return Ok(socket);
            },
            Err(err) => last_err = format!("connecting to {}: {}", addr, err),
        }
    }
    Err(failed(last_err))
}

fn set_read_timeout(socket: &Socket, timeout: Duration) {
    let stream: &TcpStream = match socket.get_ref() {
        MaybeTlsStream::Plain(s) => s,
        MaybeTlsStream::Rustls(s) => s.get_ref(),
        _ => return,
    };
    let _ = stream.set_read_timeout(Some(timeout));
}

// Keep the trait imports used even when a TLS feature set changes what
// `MaybeTlsStream` exposes.
#[allow(dead_code)]
fn _assert_stream_traits<S: Read + Write>() {}
