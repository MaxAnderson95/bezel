//! The Session's thread: a tokio runtime driving IronRDP's client, one connection attempt at
//! a time.
//!
//! The certificate callback runs inside the TLS handshake and must answer at once, so it never
//! waits for a person. When the Trust decision needs to ask, the callback records the question
//! and refuses the certificate; the failed attempt then becomes a `Certificate` event, and once
//! the app trusts it the engine connects again with the fingerprint trusted for the Session. No
//! thread blocks, nothing times out, and the server never holds a half-open handshake while a
//! dialog is up.
//!
//! When the link drops, IronRDP reconnects with the server's auto-reconnect cookie and asks
//! before each try; the engine lets it try for `RECONNECT_FOR` from the drop, then the Session
//! ends with the reason.

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bezel_core::trust::{Certificate, CertificateError, Decision, Question, SessionTrust};
use ironrdp_client::config::{Config, ConfigBuilder, Destination};
use ironrdp_client::output_channel::{OutputEventReceiver, output_channel};
use ironrdp_client::rdp::{AutoReconnectDecision, RdpClient, RdpInputEvent, RdpInputSender, RdpOutputEvent};
use ironrdp_input::{Database, MousePosition, Operation};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use rune_image::Pixels;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tracing::{debug, info, warn};

use crate::audio;
use crate::clipboard::Clipboard;
use crate::failure::{self, Ended, Failure};
use crate::framebuffer::{Framebuffer, Region};
use crate::input::{self, Outbox, Wheel};
use crate::retina::Desktop;
use crate::{Answer, Auth, CertificateRequest, Command, Event, Input, Pointer, PointerShape, State, Target, View};

pub(crate) type Emit = Arc<dyn Fn(Event) + Send + Sync>;

const RESIZE_DEBOUNCE: Duration = Duration::from_millis(250);
/// How soon to offer queued input to IronRDP again after its queue was full.
const INPUT_RETRY: Duration = Duration::from_millis(5);
/// How long after the link drops IronRDP may keep trying to reconnect.
const RECONNECT_FOR: Duration = Duration::from_secs(30);
/// More tries than fit in `RECONNECT_FOR`: IronRDP waits a second before each, so the time
/// limit is what ends them.
const RECONNECT_TRIES: u32 = 60;

pub(crate) fn spawn(
    target: Target,
    auth: Auth,
    emit: Emit,
    commands: mpsc::UnboundedReceiver<Command>,
    pixels: Pixels,
) {
    let emit_failure = emit.clone();
    let spawned = std::thread::Builder::new()
        .name(format!("session {}", target.host))
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    emit(Event::State(State::Disconnected(Ended::new(
                        "Bezel could not start the connection",
                        &e,
                    ))));
                    return;
                }
            };
            let trust = Arc::new(Mutex::new(SessionTrust::new(target.trusted_certificate.clone())));
            let engine = Engine {
                trust,
                view: target.view,
                target,
                auth,
                clipboard: Clipboard::new(emit.clone()),
                muted: Arc::default(),
                emit,
                commands,
                framebuffer: Framebuffer::new(pixels),
            };
            runtime.block_on(engine.run());
        });
    if let Err(e) = spawned {
        emit_failure(Event::State(State::Disconnected(Ended::new(
            "Bezel could not start the connection",
            &e,
        ))));
    }
}

struct Engine {
    target: Target,
    auth: Auth,
    emit: Emit,
    commands: mpsc::UnboundedReceiver<Command>,
    framebuffer: Framebuffer,
    /// What this Session trusts, shared with the certificate callback of every attempt.
    trust: Arc<Mutex<SessionTrust>>,
    /// The latest content area the app reported.
    view: View,
    clipboard: Clipboard,
    /// Shared with every connection's sound channel.
    muted: Arc<AtomicBool>,
}

enum Outcome {
    Ended(Event),
    Ask(Question),
    /// Connect again with a fresh configuration for the current View.
    Reconnect,
}

/// The certificate callback's question for the attempt it refused.
type Pending = Arc<Mutex<Option<Result<Question, CertificateError>>>>;

impl Engine {
    fn emit(&self, event: Event) {
        (self.emit)(event);
    }

    async fn run(mut self) {
        let (host, port) = (self.target.host.clone(), self.target.port);
        self.emit(Event::State(State::Connecting));
        if let Err(e) = tokio::net::lookup_host((host.as_str(), port)).await {
            self.emit(Event::State(State::Disconnected(Ended::new(
                format!("{host} could not be found"),
                &e,
            ))));
            return;
        }
        loop {
            let question = match self.attempt().await {
                Outcome::Ended(event) => return self.emit(event),
                Outcome::Ask(question) => question,
                Outcome::Reconnect => continue,
            };
            let (reply, mut answer) = oneshot::channel();
            self.emit(Event::Certificate(CertificateRequest {
                host: host.clone(),
                port,
                question: question.clone(),
                reply,
            }));
            let remember = loop {
                tokio::select! {
                    answer = &mut answer => break match answer {
                        Ok(Answer::Trust { remember }) => Some(remember),
                        Ok(Answer::Cancel) | Err(_) => None,
                    },
                    command = self.commands.recv() => match command {
                        None | Some(Command::Disconnect) => break None,
                        Some(Command::Resize(view)) => self.view = view,
                        Some(command) => self.keep(command),
                    },
                }
            };
            let Some(remember) = remember else {
                return self.emit(Event::State(State::Disconnected(Ended::by_user())));
            };
            info!(%host, port, remember, "certificate trusted");
            self.trust.lock().unwrap_or_else(PoisonError::into_inner).accept(
                &host,
                port,
                &question,
                remember,
                time::OffsetDateTime::now_utc().date(),
            );
            self.emit(Event::State(State::Connecting));
        }
    }

    /// One connection, from the TCP connect until it ends or needs a certificate answered.
    async fn attempt(&mut self) -> Outcome {
        let pending = Pending::default();
        let requested = Desktop::for_view(self.view);
        let config = match self.config(requested, pending.clone()) {
            Ok(config) => config,
            Err(e) => {
                return Outcome::Ended(disconnected(Ended::new(
                    "Bezel could not set up the connection",
                    e.as_ref(),
                )));
            }
        };
        info!(host = %self.target.host, port = self.target.port, ?requested, "connecting");
        let (sender, mut receiver) = output_channel(64);
        let client = RdpClient::new(config, sender)
            .with_desktop_updates()
            .with_auto_reconnect(RECONNECT_TRIES);
        let input = client.input_sender();
        let cliprdr = input.clone();
        let client = client.with_cliprdr_backend_factory(self.clipboard.factory(Arc::new(move |message| {
            let _ = cliprdr.send_clipboard(message);
        })));
        let mut attempt = Attempt {
            input,
            outbox: Outbox::default(),
            database: Database::new(),
            wheel: Wheel::default(),
            configured: requested,
            retry_at: None,
            requested,
            connected: false,
            resize_at: None,
            pending,
            dropped: None,
            size: (0, 0),
        };
        // Polled here rather than spawned: IronRDP's client future is not `Send`.
        let run = client.run();
        tokio::pin!(run);
        let outcome = self.drive(&mut attempt, &mut receiver, run.as_mut()).await;
        attempt.input.request_close();
        outcome
    }

    async fn drive(
        &mut self,
        attempt: &mut Attempt,
        receiver: &mut OutputEventReceiver,
        mut run: Pin<&mut impl Future<Output = ()>>,
    ) -> Outcome {
        // Once the client finishes, its output channel closes and the receiver drains to the end.
        let mut running = true;
        loop {
            let resize = sleep_until(attempt.resize_at);
            tokio::select! {
                () = &mut run, if running => running = false,
                // IronRDP checks with the engine only between tries, and a server can take a
                // try's connection and then say nothing.
                () = sleep_until(attempt.dropped.map(|at| at + RECONNECT_FOR)) => {
                    // A reconnect that finished as time ran out has its event waiting, and
                    // `select!` may have picked this branch over it.
                    if let Ok(Some(event)) = tokio::time::timeout(Duration::ZERO, receiver.recv()).await
                        && let Some(outcome) = self.on_events(attempt, receiver, event).await
                    {
                        return outcome;
                    }
                    if attempt.dropped.is_some() {
                        info!(host = %self.target.host, "the connection did not come back in time");
                        return Outcome::Ended(disconnected(failure::not_back(None, &self.target.host)));
                    }
                }
                () = sleep_until(attempt.retry_at) => {
                    attempt.retry_at = None;
                    attempt.flush();
                }
                event = receiver.recv() => {
                    let Some(event) = event else { return self.closed() };
                    if let Some(outcome) = self.on_events(attempt, receiver, event).await {
                        return outcome;
                    }
                }
                command = self.commands.recv() => match command {
                    None | Some(Command::Disconnect) => return Outcome::Ended(disconnected(Ended::by_user())),
                    Some(Command::Resize(view)) => {
                        self.view = view;
                        attempt.resize_at = Some(Instant::now() + RESIZE_DEBOUNCE);
                    }
                    Some(Command::Input(input)) if attempt.connected => {
                        let ops = self.operations(attempt, input);
                        attempt.send(ops);
                    }
                    Some(Command::CtrlAltDel) if attempt.connected => attempt.send(input::CTRL_ALT_DEL.into_iter().map(input::key).collect()),
                    Some(command) => self.keep(command),
                },
                () = resize => {
                    attempt.resize_at = None;
                    attempt.resize_to(Desktop::for_view(self.view));
                }
            }
        }
    }

    /// Handles `first` and every event already waiting behind it before publishing, so a burst
    /// of updates costs one copy into the framebuffer the app draws.
    async fn on_events(
        &mut self,
        attempt: &mut Attempt,
        receiver: &mut OutputEventReceiver,
        first: RdpOutputEvent,
    ) -> Option<Outcome> {
        let mut next = first;
        loop {
            if let Some(outcome) = self.on_event(attempt, next) {
                return Some(outcome);
            }
            match tokio::time::timeout(Duration::ZERO, receiver.recv()).await {
                Ok(Some(event)) => next = event,
                Ok(None) => return Some(self.closed()),
                Err(_) => break,
            }
        }
        self.framebuffer.publish();
        None
    }

    /// IronRDP's client finished and its output channel has nothing more.
    fn closed(&self) -> Outcome {
        Outcome::Ended(disconnected(Ended::plain(format!(
            "The connection to {} ended",
            self.target.host
        ))))
    }

    fn on_event(&mut self, attempt: &mut Attempt, event: RdpOutputEvent) -> Option<Outcome> {
        let host = &self.target.host;
        match event {
            RdpOutputEvent::Connected => {
                info!(%host, "connected");
                attempt.connected = true;
                // Also how a reconnect ends whose cookie the server refused but whose sign-in it
                // took.
                attempt.dropped = None;
                let Desktop { width, height, .. } = attempt.requested;
                self.connected(attempt, width, height);
                // The window may have changed while connecting.
                attempt.resize_to(Desktop::for_view(self.view));
            }
            RdpOutputEvent::DesktopUpdate(update) => {
                let (buffer, width, height, region) = update.into_parts();
                let region = Region {
                    left: region.left,
                    top: region.top,
                    right: region.right,
                };
                if self.framebuffer.apply(width.get(), height.get(), region, &buffer) {
                    debug!(width = width.get(), height = height.get(), "desktop size");
                    self.connected(attempt, width.get(), height.get());
                }
            }
            RdpOutputEvent::DisplayResizeFallback(reason) => {
                info!(%host, ?reason, "the server cannot resize in place; reconnecting at the new size");
                attempt.connected = false;
                self.emit(Event::State(State::Reconnecting));
                // IronRDP's own reconnect keeps the scale factors the attempt was configured
                // with, so a move to a display of another scale connects again from scratch.
                if !Desktop::for_view(self.view).same_scale(&attempt.configured) {
                    return Some(Outcome::Reconnect);
                }
                attempt.configured = Desktop {
                    width: attempt.requested.width,
                    height: attempt.requested.height,
                    ..attempt.configured
                };
            }
            RdpOutputEvent::AutoReconnecting {
                attempt: tries,
                response,
                ..
            } => {
                if attempt.dropped.is_none() {
                    info!(%host, "the connection dropped; reconnecting");
                    attempt.lose_link();
                    self.emit(Event::State(State::Reconnecting));
                }
                let since = *attempt.dropped.get_or_insert_with(Instant::now);
                // A certificate that needs a decision ends the tries so the person is asked.
                let asking = attempt.pending.lock().unwrap_or_else(PoisonError::into_inner).is_some();
                let go = !asking && since.elapsed() < RECONNECT_FOR;
                debug!(%host, tries, go, "reconnect");
                let _ = response.send(match go {
                    true => AutoReconnectDecision::Continue,
                    false => AutoReconnectDecision::Stop,
                });
            }
            RdpOutputEvent::AutoReconnected => {
                info!(%host, "reconnected");
                attempt.dropped = None;
                attempt.connected = true;
                // The size the app last heard; a desktop of another size says so with its first
                // update.
                let (width, height) = attempt.size;
                self.connected(attempt, width, height);
                attempt.resize_to(Desktop::for_view(self.view));
            }
            RdpOutputEvent::PointerDefault => self.emit(Event::Pointer(Pointer::Default)),
            RdpOutputEvent::PointerHidden => self.emit(Event::Pointer(Pointer::Hidden)),
            RdpOutputEvent::PointerBitmap(pointer) => {
                if pointer.bitmap_data.len() == usize::from(pointer.width) * usize::from(pointer.height) * 4 {
                    self.emit(Event::Pointer(Pointer::Shape(Arc::new(PointerShape {
                        width: pointer.width,
                        height: pointer.height,
                        hotspot: (pointer.hotspot_x, pointer.hotspot_y),
                        rgba: pointer.bitmap_data.clone(),
                    }))));
                }
            }
            RdpOutputEvent::ConnectionFailure(error) => {
                if let Some(pending) = attempt.pending.lock().unwrap_or_else(PoisonError::into_inner).take() {
                    return Some(match pending {
                        Ok(question) => Outcome::Ask(question),
                        Err(e) => Outcome::Ended(disconnected(Ended::new(
                            format!("The certificate of {host} could not be read"),
                            &e,
                        ))),
                    });
                }
                warn!(%host, error = %error.report(), "connection failed");
                return Some(Outcome::Ended(
                    match failure::classify(&error, host, self.target.port) {
                        Failure::SignIn { wrong_password, ended } => Event::SignInFailed { wrong_password, ended },
                        // The last try to reconnect failed: the drop is what the person needs
                        // to hear about.
                        Failure::Ended(_) if attempt.dropped.is_some() => {
                            disconnected(failure::not_back(Some(&error), host))
                        }
                        Failure::Ended(ended) => disconnected(ended),
                    },
                ));
            }
            RdpOutputEvent::Terminated(result) => {
                use ironrdp_session::GracefulDisconnectReason as Reason;
                let ended = match result {
                    Ok(Reason::UserInitiated) => Ended::by_user(),
                    Ok(Reason::ServerInitiated) => Ended::plain(format!("{host} ended the session")),
                    Ok(Reason::Other(why)) => Ended::plain(format!("{host} ended the session: {why}")),
                    Err(error) => {
                        warn!(%host, error = %error.report(), "session lost");
                        match attempt.dropped {
                            Some(_) => failure::not_back(Some(&error), host),
                            None => failure::lost(&error, host),
                        }
                    }
                };
                info!(%host, message = %ended.message, "session ended");
                return Some(Outcome::Ended(disconnected(ended)));
            }
            RdpOutputEvent::MonitorLayoutRejected => warn!(%host, "the server refused the new desktop size"),
            RdpOutputEvent::LogonError(info) => info!(%host, ?info, "the server reported a logon error"),
            other => debug!(event = ?std::mem::discriminant(&other), "ignored output event"),
        }
        None
    }

    fn connected(&self, attempt: &mut Attempt, width: u16, height: u16) {
        attempt.size = (width, height);
        self.emit(Event::State(State::Connected { width, height }));
    }

    /// A command with nothing to send now: the engine keeps what it says for the connection.
    fn keep(&self, command: Command) {
        match command {
            Command::Clipboard(text) => self.clipboard.set(text),
            Command::Mute(muted) => self.muted.store(muted, Ordering::Relaxed),
            Command::Input(_) | Command::Resize(_) | Command::CtrlAltDel | Command::Disconnect => {}
        }
    }

    fn operations(&self, attempt: &mut Attempt, input: Input) -> Vec<Operation> {
        match input {
            Input::PointerMove { x, y } => {
                let (width, height) = self.framebuffer.size();
                let at = |fraction: f32, side: u16| {
                    (fraction * f32::from(side)).clamp(0.0, f32::from(side.saturating_sub(1))) as u16
                };
                vec![Operation::MouseMove(MousePosition {
                    x: at(x, width),
                    y: at(y, height),
                })]
            }
            Input::Button { button, pressed: true } => vec![Operation::MouseButtonPressed(input::button(button))],
            Input::Button { button, pressed: false } => vec![Operation::MouseButtonReleased(input::button(button))],
            Input::Wheel { dx, dy } => attempt.wheel.rotations(dx, dy),
            Input::Keys(ops) => ops.into_iter().map(input::key).collect(),
            Input::ReleaseAll => {
                attempt.outbox.push(attempt.database.release_all());
                attempt.flush();
                Vec::new()
            }
        }
    }

    fn config(&self, desktop: Desktop, pending: Pending) -> anyhow::Result<Config> {
        let Target { host, port, .. } = &self.target;
        let trust = self.trust.clone();
        let callback = Arc::new(move |der: &[u8], _endpoint: &str, reason: &str| {
            let mut pending = pending.lock().unwrap_or_else(PoisonError::into_inner);
            let certificate = match Certificate::parse(der) {
                Ok(certificate) => certificate,
                Err(e) => {
                    *pending = Some(Err(e));
                    return false;
                }
            };
            let decision = trust
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .decide(&certificate);
            match decision {
                Decision::Accept => true,
                Decision::Ask(question) => {
                    info!(reason, fingerprint = %certificate.sha256, "the certificate needs a decision");
                    *pending = Some(Ok(question));
                    false
                }
            }
        });
        let muted = self.muted.clone();
        let (username, domain) = self.auth.username.protocol_parts();
        let mut builder = ConfigBuilder::new()
            .with_destination(Destination::from_parts(host.clone(), *port))
            .with_username(username)
            .with_password(self.auth.password.0.clone())
            .with_client_name("Bezel")
            .with_client_build(1)
            .with_client_dir(r"C:\Windows\System32\mstscax.dll")
            .with_platform(if cfg!(target_os = "macos") {
                MajorPlatformType::MACINTOSH
            } else {
                MajorPlatformType::UNIX
            })
            // NLA first; TLS alone lets a server with NLA off show its login screen. Legacy RDP
            // security without TLS is never offered.
            .with_credssp(true)
            .with_tls(true)
            .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
            .with_certificate_validation_callback(callback)
            .with_desktop_width(desktop.width)
            .with_desktop_height(desktop.height)
            .with_desktop_scale_factor(desktop.desktop_scale)
            .with_device_scale_factor(desktop.device_scale)
            .with_tcp_connect_attempt_timeout(Duration::from_secs(10))
            .with_static_channel(move |_| Some(audio::channel(muted.clone())));
        if let Some(domain) = domain {
            builder = builder.with_domain(domain);
        }
        builder.build()
    }
}

struct Attempt {
    input: RdpInputSender,
    /// Input waiting for room in IronRDP's bounded queue, in order.
    outbox: Outbox,
    /// The key and button state of the input queued so far.
    database: Database,
    wheel: Wheel,
    /// What the server was last asked for.
    requested: Desktop,
    /// What the attempt's configuration asked for, whose scale factors IronRDP keeps when it
    /// reconnects on its own.
    configured: Desktop,
    /// When to offer the outbox to IronRDP again, kept across wakes so other traffic can't
    /// postpone it.
    retry_at: Option<Instant>,
    connected: bool,
    resize_at: Option<Instant>,
    pending: Pending,
    /// When the link dropped, while IronRDP reconnects.
    dropped: Option<Instant>,
    /// The remote desktop's size as the app last heard it.
    size: (u16, u16),
}

impl Attempt {
    /// The link dropped: input waits for the reconnect, which starts the server from no keys
    /// held and at the size IronRDP's configuration asks for.
    fn lose_link(&mut self) {
        self.connected = false;
        self.outbox = Outbox::default();
        self.retry_at = None;
        self.database = Database::new();
        self.requested = self.configured;
    }

    fn send(&mut self, ops: Vec<Operation>) {
        if !ops.is_empty() {
            self.outbox.push(self.database.apply(ops));
            self.flush();
        }
    }

    fn flush(&mut self) {
        let input = &self.input;
        self.outbox
            .flush(|batch| match input.try_send(RdpInputEvent::FastPath(batch)) {
                Ok(()) => Ok(()),
                Err(TrySendError::Full(RdpInputEvent::FastPath(batch))) => Err(batch),
                // The session is closing; its end arrives as an output event.
                Err(_) => Ok(()),
            });
        self.retry_at = match self.outbox.is_empty() {
            true => None,
            false => Some(self.retry_at.unwrap_or_else(|| Instant::now() + INPUT_RETRY)),
        };
    }

    fn resize_to(&mut self, desktop: Desktop) {
        if !self.connected || desktop == self.requested {
            return;
        }
        let layout = match desktop.layout() {
            Ok(layout) => layout,
            Err(e) => return warn!(error = %e, ?desktop, "no monitor layout for this size"),
        };
        match self.input.try_send(RdpInputEvent::MonitorLayout(layout)) {
            Ok(()) => {
                info!(?desktop, "resizing");
                self.requested = desktop;
            }
            Err(e) => {
                warn!(error = %e, "resize not queued; trying again");
                self.resize_at = Some(Instant::now() + INPUT_RETRY);
            }
        }
    }
}

/// Sleeps until `at`, or forever when there is nothing to wait for.
async fn sleep_until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

fn disconnected(ended: Ended) -> Event {
    Event::State(State::Disconnected(ended))
}

#[cfg(test)]
mod tests {
    use bezel_core::usernames::Username;

    use super::*;

    /// An engine for dc01 whose events go nowhere, with the sender that keeps its commands open.
    fn engine() -> (Engine, mpsc::UnboundedSender<Command>) {
        let view = View {
            width: 1024.0,
            height: 768.0,
            scale: 2.0,
        };
        let emit: Emit = Arc::new(|_| {});
        let (commands, receiver) = mpsc::unbounded_channel();
        let engine = Engine {
            target: Target {
                host: "dc01.corp.lan".into(),
                port: 3389,
                view,
                trusted_certificate: None,
            },
            auth: Auth {
                username: Username::parse("administrator").unwrap(),
                password: String::new().into(),
            },
            clipboard: Clipboard::new(emit.clone()),
            emit,
            commands: receiver,
            framebuffer: Framebuffer::new(Pixels::new()),
            trust: Arc::new(Mutex::new(SessionTrust::new(None))),
            view,
            muted: Arc::default(),
        };
        (engine, commands)
    }

    /// An attempt whose link dropped longer than `RECONNECT_FOR` ago, with the receiver that
    /// keeps its input queue open.
    fn dropped_at_the_deadline(engine: &Engine) -> (Attempt, tokio::sync::mpsc::Receiver<RdpInputEvent>) {
        let (input, queue) = RdpInputSender::channel(8);
        let requested = Desktop::for_view(engine.view);
        let attempt = Attempt {
            input,
            outbox: Outbox::default(),
            database: Database::new(),
            wheel: Wheel::default(),
            requested,
            configured: requested,
            retry_at: None,
            connected: false,
            resize_at: None,
            pending: Pending::default(),
            // A second over, so the deadline has passed by tokio's millisecond timer too.
            dropped: Some(Instant::now() - RECONNECT_FOR - Duration::from_secs(1)),
            size: (2048, 1536),
        };
        (attempt, queue)
    }

    /// The deadline and a reconnect that already succeeded are both ready: the Session is back.
    #[tokio::test]
    async fn a_reconnect_waiting_at_the_deadline_wins() {
        // `select!` picks among ready branches at random, so enough runs meet both orders.
        for _ in 0..32 {
            let (mut engine, _commands) = engine();
            let (mut attempt, _queue) = dropped_at_the_deadline(&engine);
            let (sender, mut receiver) = output_channel(4);
            sender.try_send(RdpOutputEvent::AutoReconnected).unwrap();
            let mut run = std::pin::pin!(std::future::pending::<()>());
            let drive = engine.drive(&mut attempt, &mut receiver, run.as_mut());
            let ended = tokio::time::timeout(Duration::from_millis(20), drive).await.is_ok();
            assert!(!ended, "the Session ended at the deadline");
            assert!(attempt.dropped.is_none() && attempt.connected);
        }
    }

    /// A server that says nothing by the deadline ends the Session.
    #[tokio::test]
    async fn a_silent_server_ends_the_session_at_the_deadline() {
        let (mut engine, _commands) = engine();
        let (mut attempt, _queue) = dropped_at_the_deadline(&engine);
        let (_sender, mut receiver) = output_channel(4);
        let mut run = std::pin::pin!(std::future::pending::<()>());
        let drive = engine.drive(&mut attempt, &mut receiver, run.as_mut());
        let outcome = tokio::time::timeout(Duration::from_secs(1), drive)
            .await
            .expect("an ending");
        assert!(matches!(
            outcome,
            Outcome::Ended(Event::State(State::Disconnected(ended))) if ended.message.ends_with("did not come back")
        ));
    }
}
