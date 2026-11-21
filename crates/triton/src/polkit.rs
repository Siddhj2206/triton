//! The polkit authentication agent: Triton answers interactive
//! `CheckAuthorization` calls with its own dialog.
//!
//! # The protocol
//!
//! When a client authorizes interactively, polkitd calls
//! `org.freedesktop.PolicyKit1.AuthenticationAgent.BeginAuthentication` on the
//! registered agent, and the agent completes the session by calling
//! `AuthenticationAgentResponse2` on the authority — or never does, leaving
//! the request to time out. Both halves live on the **system** bus: the agent
//! object `/org/freedesktop/PolicyKit1/AuthenticationAgent` is served on
//! `Connection::system()` and `RegisterAuthenticationAgent` is sent on that
//! same connection, because the authority reaches the agent by the unique bus
//! name of the registration call's sender. No well-known name is requested;
//! serving an object under our unique name needs no policy file.
//!
//! The password itself never goes over D-Bus. It goes through the setuid
//! `polkit-agent-helper-1` binary, which the agent spawns as
//! `polkit-agent-helper-1 <cookie> <identity>` and which runs a PAM
//! conversation as a line protocol on its stdio: it writes
//! `PAM_PROMPT_ECHO_OFF <text>`, `PAM_PROMPT_ECHO_ON <text>`,
//! `PAM_ERROR_MSG <text>` and `PAM_TEXT_INFO <text>` on stdout (one line
//! each; a missing text still ends the line), reads one reply line per prompt
//! on stdin, and finishes with `SUCCESS` or `FAILURE`. Only `SUCCESS` earns
//! the `AuthenticationAgentResponse2` call; anything else lets the
//! authorization time out on polkitd's side.
//!
//! Unless `--real-services` is passed, [`agent`] is a dead stream: a dev or
//! preview box must never steal the session's real elevation prompts.
//!
//! Sources: polkit's `polkit-agent-helper-1.c` (the line protocol and the
//! helper argv) and `polkitagentsession.c` (`SUCCESS` →
//! `AuthenticationAgentResponse2`), the polkit reference manual for the
//! `Authority`/`AuthenticationAgent` interfaces, and the
//! lxqt-policykit-agent and hyprpolkitagent implementations.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;

use iced_futures::Subscription;
use iced_futures::futures::channel::mpsc;
use zbus::{Connection, OwnedVariant, Variant, fdo, interface, proxy};

/// The object path every polkit agent serves; the authority only cares that
/// it matches what registration declared.
const AGENT_PATH: &str = "/org/freedesktop/PolicyKit1/AuthenticationAgent";

/// An interactive authorization polkitd asked us to mediate.
#[derive(Debug, Clone)]
pub struct Prompt {
    /// Identifies the pending session; [`respond`] and [`cancel`] take it.
    pub cookie: String,
    /// The action being authorized (`org.example.app.do-thing`).
    pub action_id: String,
    /// The message to show, e.g. "Authentication is required to …".
    pub message: String,
    /// A themed icon name, when the action provides one.
    pub icon: Option<String>,
    /// Display name of the identity we picked (the gecos name, else login).
    pub user: String,
    /// The identity in polkit's `kind:id` text form (`unix-user:1000`);
    /// passed verbatim to [`respond`], which turns it back into both the
    /// helper's argv and the dict `AuthenticationAgentResponse2` wants.
    pub identity: String,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// polkitd started an interactive authorization; show the dialog.
    Prompt(Prompt),
    /// The prompt for `cookie` resolved without the user's secret:
    /// cancelled by the authority (or another agent won), or a [`respond`]
    /// conversation ended. Either way, close its dialog.
    Cancelled { cookie: String },
}

/// The `a{sv}` maps polkit speaks in: subjects, identities, details.
type SvMap = BTreeMap<String, Variant<'static>>;

/// Where the app listens, refreshed on every (re)subscription.
static EVENT_TX: Mutex<Option<mpsc::UnboundedSender<Event>>> = Mutex::new(None);

/// A cookie's auth session. `Pending` is the reservation [`respond`] takes
/// before the helper exists, so a cancel landing during the spawn still
/// wins; `Running` keeps the child so it can be killed on cancel.
enum Session {
    Pending,
    Running(Arc<Mutex<Child>>),
}

/// Auth sessions by cookie. The worker that owns a session keeps the
/// child's `Arc` too, and reaps it at the end.
static SESSIONS: Mutex<BTreeMap<String, Session>> = Mutex::new(BTreeMap::new());

/// A poisoned map must not break an auth flow: recover the guard.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Forward an event to the app; gone is fine, the prompt just times out.
fn emit(event: Event) {
    let tx = lock(&EVENT_TX).clone();
    if let Some(tx) = tx {
        let _ = tx.unbounded_send(event);
    }
}

/// The authority, as far as the agent needs it.
#[proxy(
    interface = "org.freedesktop.PolicyKit1.Authority",
    default_service = "org.freedesktop.PolicyKit1",
    default_path = "/org/freedesktop/PolicyKit1/Authority"
)]
trait Authority {
    /// Claim the agent role for `subject` — our unix session.
    async fn register_authentication_agent(
        &self,
        subject: &SvMap,
        object_path: &str,
        locale: &str,
    ) -> zbus::Result<()>;

    /// Complete a session the helper's PAM conversation approved.
    /// Signature `(usa{sv})`: the agent's uid, the cookie, the identity it
    /// authenticated as.
    async fn authentication_agent_response2(
        &self,
        uid: u32,
        cookie: &str,
        identity: &SvMap,
    ) -> zbus::Result<()>;
}

/// The `org.freedesktop.PolicyKit1.AuthenticationAgent` object.
struct Listener {
    tx: mpsc::UnboundedSender<Event>,
}

#[interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Listener {
    /// polkitd starts an interactive authorization for one cookie. The call
    /// returns at once; the session completes when we answer the authority
    /// or it times out.
    fn begin_authentication(
        &self,
        action_id: &str,
        message: &str,
        icon_name: &str,
        details: BTreeMap<String, String>,
        cookie: String,
        identities: Vec<BTreeMap<String, OwnedVariant>>,
    ) -> fdo::Result<()> {
        tracing::debug!(%action_id, %cookie, ?details, "polkit: begin authentication");
        let Some((identity, user)) = pick_identity(&identities, our_uid()) else {
            tracing::warn!(%cookie, "polkit: no usable identity");
            return Err(fdo::Error::Failed(
                "polkit offered no identity to authenticate as".into(),
            ));
        };
        let prompt = Prompt {
            cookie: cookie.clone(),
            action_id: action_id.to_owned(),
            message: message.to_owned(),
            icon: (!icon_name.is_empty()).then(|| icon_name.to_owned()),
            user,
            identity,
        };
        if self.tx.unbounded_send(Event::Prompt(prompt)).is_err() {
            tracing::debug!(%cookie, "polkit: the app is gone, prompt dropped");
        }
        Ok(())
    }

    /// polkitd cancelled a pending authorization (or another agent won).
    fn cancel_authentication(&self, cookie: String) {
        tracing::debug!(%cookie, "polkit: cancel authentication");
        kill_helper(&cookie);
        let _ = self.tx.unbounded_send(Event::Cancelled { cookie });
    }
}

/// Serve the agent object, then register it for our session. Keeps the
/// connection (and the object server on it) alive forever after.
async fn run(tx: mpsc::UnboundedSender<Event>) {
    let conn = match Connection::system().await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::warn!(%e, "polkit: no system bus");
            return;
        }
    };
    if let Err(e) = conn
        .object_server()
        .at(AGENT_PATH, Listener { tx })
        .await
    {
        tracing::warn!(%e, "polkit: cannot serve the agent object");
        return;
    }
    let Some(subject) = agent_subject() else {
        tracing::warn!("polkit: no session id and no start time; agent stays unregistered");
        return;
    };
    let authority = match AuthorityProxy::new(&conn).await {
        Ok(authority) => authority,
        Err(e) => {
            tracing::warn!(%e, "polkit: cannot reach the authority");
            return;
        }
    };
    match authority
        .register_authentication_agent(&subject, AGENT_PATH, &locale())
        .await
    {
        Ok(()) => tracing::info!("polkit: registered as the authentication agent"),
        Err(e) => {
            // Typically "an authentication agent already exists": the
            // session's own agent won. Serving the object harmlessly, the
            // authority never calls an unregistered one.
            tracing::warn!(%e, "polkit: registration failed");
        }
    }
    std::future::pending::<()>().await;
}

/// The subject `RegisterAuthenticationAgent` takes: our unix session when
/// the environment names it (`$XDG_SESSION_ID`), else our unix process —
/// which scopes the agent to this shell instance only.
fn agent_subject() -> Option<SvMap> {
    let mut subject = SvMap::new();
    if let Some(id) = std::env::var("XDG_SESSION_ID")
        .ok()
        .filter(|id| !id.is_empty())
    {
        subject.insert("kind".to_owned(), Variant::from("unix-session"));
        subject.insert("session-id".to_owned(), Variant::from(id));
    } else {
        let start_time = process_start_time()?;
        subject.insert("kind".to_owned(), Variant::from("unix-process"));
        subject.insert("pid".to_owned(), Variant::from(std::process::id()));
        subject.insert("start-time".to_owned(), Variant::from(start_time));
    }
    Some(subject)
}

/// The locale argument of `RegisterAuthenticationAgent`.
fn locale() -> String {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(var) {
            if !value.is_empty() {
                return value;
            }
        }
    }
    "C".to_owned()
}

/// Our real uid, from `/proc/self/status` (`getuid()` without libc).
fn our_uid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("Uid:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// The process start time the `unix-process` subject wants: field 22 of
/// `/proc/self/stat`, in clock ticks.
fn process_start_time() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    stat_start_time(&stat)
}

fn stat_start_time(stat: &str) -> Option<u64> {
    // `comm` may hold spaces and parens; what follows the last ')' starts
    // at field 3 (state), so starttime is the 20th word after it.
    let after_comm = stat.rsplit_once(')')?.1;
    after_comm.split_whitespace().nth(19)?.parse().ok()
}

/// Pick an identity dict, preferring one for our own uid, and give back its
/// `kind:id` text form plus a display name.
fn pick_identity(
    identities: &[BTreeMap<String, OwnedVariant>],
    uid: Option<u32>,
) -> Option<(String, String)> {
    let dict = identities
        .iter()
        .find(|dict| match uid {
            Some(uid) => {
                dict_str(dict, "kind").as_deref() == Some("unix-user")
                    && dict_num(dict, "uid") == Some(uid)
            }
            None => false,
        })
        .or_else(|| identities.first())?;
    let identity = identity_string(dict)?;
    let display = display_name(dict, &identity);
    Some((identity, display))
}

fn dict_str(dict: &BTreeMap<String, OwnedVariant>, key: &str) -> Option<String> {
    dict.get(key)
        .and_then(|v| <&str>::try_from(v).ok())
        .map(str::to_owned)
}

fn dict_num(dict: &BTreeMap<String, OwnedVariant>, key: &str) -> Option<u32> {
    dict.get(key).and_then(|v| u32::try_from(v).ok())
}

/// The `kind:id` text form `polkit_identity_to_string` produces and
/// `polkit-agent-helper-1` parses (`unix-user:1000`, `unix-group:wheel`).
fn identity_string(dict: &BTreeMap<String, OwnedVariant>) -> Option<String> {
    let kind = dict_str(dict, "kind")?;
    let id = match kind.as_str() {
        "unix-user" => dict_num(dict, "uid")
            .map(|n| n.to_string())
            .or_else(|| dict_str(dict, "name")),
        "unix-group" => dict_num(dict, "gid")
            .map(|n| n.to_string())
            .or_else(|| dict_str(dict, "name")),
        _ => dict_str(dict, "name"),
    }?;
    Some(format!("{kind}:{id}"))
}

/// `kind:id` back to the `a{sv}` `AuthenticationAgentResponse2` wants;
/// `polkit_identity_from_gvariant` takes `uid`/`gid` (`u`) or `name` (`s`).
fn identity_dict(identity: &str) -> Option<SvMap> {
    let (kind, id) = identity
        .split_once(':')
        .map(|(k, i)| (k.to_owned(), i.to_owned()))?;
    let mut dict = SvMap::new();
    dict.insert("kind".to_owned(), Variant::from(kind.clone()));
    match kind.as_str() {
        "unix-user" | "unix-group" => {
            let key = if kind == "unix-user" { "uid" } else { "gid" };
            match id.parse::<u32>() {
                Ok(n) => dict.insert(key.to_owned(), Variant::from(n)),
                Err(_) => dict.insert("name".to_owned(), Variant::from(id)),
            };
        }
        _ => {
            dict.insert("name".to_owned(), Variant::from(id));
        }
    }
    Some(dict)
}

/// What the dialog shows for the picked identity.
fn display_name(dict: &BTreeMap<String, OwnedVariant>, identity: &str) -> String {
    match dict_str(dict, "kind").as_deref() {
        Some("unix-user") => dict_num(dict, "uid")
            .and_then(user_name)
            .or_else(|| dict_str(dict, "name"))
            .unwrap_or_else(|| identity.to_owned()),
        _ => dict_str(dict, "name").unwrap_or_else(|| identity.to_owned()),
    }
}

/// A uid's display name from `/etc/passwd`: the gecos full name if set,
/// else the login.
fn user_name(uid: u32) -> Option<String> {
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd_user(&passwd, uid)
}

fn passwd_user(contents: &str, uid: u32) -> Option<String> {
    contents.lines().find_map(|line| {
        let mut fields = line.splitn(7, ':');
        let name = fields.next()?;
        fields.next()?; // password
        let line_uid: u32 = fields.next()?.parse().ok()?;
        if line_uid != uid {
            return None;
        }
        fields.next()?; // gid
        let gecos = fields.next().unwrap_or_default();
        let full = gecos.split(',').next().unwrap_or_default();
        Some(if full.is_empty() {
            name.to_owned()
        } else {
            full.to_owned()
        })
    })
}

/// One line of the helper's stdio protocol (see the module docs).
#[derive(Debug, PartialEq, Eq)]
enum HelperLine<'a> {
    /// `PAM_PROMPT_ECHO_*`: answer on stdin. `echo` says whether the reply
    /// may echo; we send the secret regardless.
    Prompt { echo: bool, text: &'a str },
    /// `PAM_ERROR_MSG`: a PAM module's complaint, for the log.
    Error(&'a str),
    /// `PAM_TEXT_INFO`: a PAM module's note, for the log.
    Info(&'a str),
    Success,
    Failure,
    /// Anything else: logged, the conversation continues.
    Unknown,
}

fn parse_helper_line(line: &str) -> HelperLine<'_> {
    if let Some(text) = line.strip_prefix("PAM_PROMPT_ECHO_OFF") {
        HelperLine::Prompt {
            echo: false,
            text: text.trim(),
        }
    } else if let Some(text) = line.strip_prefix("PAM_PROMPT_ECHO_ON") {
        HelperLine::Prompt {
            echo: true,
            text: text.trim(),
        }
    } else if let Some(text) = line.strip_prefix("PAM_ERROR_MSG") {
        HelperLine::Error(text.trim())
    } else if let Some(text) = line.strip_prefix("PAM_TEXT_INFO") {
        HelperLine::Info(text.trim())
    } else if line == "SUCCESS" {
        HelperLine::Success
    } else if line == "FAILURE" {
        HelperLine::Failure
    } else {
        HelperLine::Unknown
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// The helper's PAM conversation approved: answer the authority.
    Success,
    /// `FAILURE`, a dead helper, or a killed pipe: never answer the
    /// authority, the request times out.
    Failure,
}

/// The helper conversation, factored off the process plumbing so tests can
/// drive it with in-memory pipes: every prompt gets the secret, `SUCCESS`
/// or `FAILURE` ends it.
fn converse<R: BufRead, W: Write>(mut out: R, stdin: &mut W, secret: &[u8]) -> Outcome {
    let mut line = String::new();
    loop {
        line.clear();
        match out.read_line(&mut line) {
            Ok(0) => {
                tracing::debug!("polkit: the helper closed its output");
                return Outcome::Failure;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(%e, "polkit: cannot read the helper");
                return Outcome::Failure;
            }
        }
        match parse_helper_line(line.trim_end()) {
            HelperLine::Prompt { echo, text } => {
                tracing::debug!(echo, %text, "polkit: PAM prompt");
                if let Err(e) = stdin
                    .write_all(secret)
                    .and_then(|()| stdin.write_all(b"\n"))
                    .and_then(|()| stdin.flush())
                {
                    tracing::debug!(%e, "polkit: cannot answer the helper");
                    return Outcome::Failure;
                }
            }
            HelperLine::Error(text) => {
                tracing::debug!(%text, "polkit: PAM error message");
            }
            HelperLine::Info(text) => {
                tracing::debug!(%text, "polkit: PAM text info");
            }
            HelperLine::Success => return Outcome::Success,
            HelperLine::Failure => return Outcome::Failure,
            HelperLine::Unknown => {
                tracing::debug!(%line, "polkit: unexpected helper line");
            }
        }
    }
}

/// The setuid helper's usual libexec paths; it is never on `PATH`.
fn helper_path() -> Option<PathBuf> {
    const CANDIDATES: &[&str] = &[
        // Fedora, RHEL, openSUSE, Arch.
        "/usr/libexec/polkit-agent-helper-1",
        // Debian, Ubuntu.
        "/usr/lib/polkit-1/polkit-agent-helper-1",
        "/lib/polkit-1/polkit-agent-helper-1",
        // Gentoo's libdir layout.
        "/usr/lib/polkit/polkit-agent-helper-1",
        // NixOS setuid wrappers.
        "/run/wrappers/bin/polkit-agent-helper-1",
    ];
    CANDIDATES.iter().map(PathBuf::from).find(|p| p.is_file())
}

/// Spawn `polkit-agent-helper-1 <cookie> <identity>` with piped stdio.
fn spawn_helper(cookie: &str, identity: &str) -> Option<Child> {
    let Some(helper) = helper_path() else {
        tracing::error!("polkit: no polkit-agent-helper-1 found");
        return None;
    };
    match Command::new(&helper)
        .arg(cookie)
        .arg(identity)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => Some(child),
        Err(e) => {
            tracing::error!(%e, ?helper, "polkit: cannot spawn the helper");
            None
        }
    }
}

/// Kill the conversation for `cookie`, if one is running. The worker sees
/// dead pipes, reaps the child, and — its map entry already gone — stays
/// quiet instead of emitting a duplicate [`Event::Cancelled`].
fn kill_helper(cookie: &str) {
    let entry = lock(&SESSIONS).remove(cookie);
    if let Some(Session::Running(child)) = entry {
        let _ = lock(&child).kill();
    }
}

/// The worker side of [`respond`]: spawn the helper, run the conversation,
/// answer the authority on `SUCCESS`, and tell the app the prompt is over.
/// Every failure path logs and just lets the polkit request time out.
fn run_session(cookie: String, identity: String, password: String) {
    {
        let mut sessions = lock(&SESSIONS);
        match sessions.entry(cookie.clone()) {
            Entry::Vacant(slot) => {
                slot.insert(Session::Pending);
            }
            Entry::Occupied(_) => {
                tracing::warn!(%cookie, "polkit: a conversation is already running");
                return;
            }
        }
    }
    let Some(mut child) = spawn_helper(&cookie, &identity) else {
        lock(&SESSIONS).remove(&cookie);
        emit(Event::Cancelled { cookie });
        return;
    };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        tracing::error!(%cookie, "polkit: cannot pipe the helper");
        lock(&SESSIONS).remove(&cookie);
        let _ = child.kill();
        let _ = child.wait();
        emit(Event::Cancelled { cookie });
        return;
    };
    let session = Arc::new(Mutex::new(child));
    let live = {
        let mut sessions = lock(&SESSIONS);
        match sessions.get_mut(&cookie) {
            Some(entry) => {
                *entry = Session::Running(Arc::clone(&session));
                true
            }
            // A cancel landed while the helper spawned.
            None => false,
        }
    };
    if !live {
        let mut child = lock(&session);
        let _ = child.kill();
        let _ = child.wait();
        return;
    }
    let mut secret = password.into_bytes();
    let outcome = converse(BufReader::new(stdout), &mut stdin, &secret);
    secret.fill(0);
    // Whoever removes the entry owns the `Cancelled` event: `cancel` and
    // `CancelAuthentication` take it through `kill_helper`, so the worker
    // emits only while the session was still open at the end.
    let still_open = lock(&SESSIONS).remove(&cookie).is_some();
    {
        let mut child = lock(&session);
        let _ = child.kill();
        let _ = child.wait();
    }
    if outcome == Outcome::Success {
        respond_to_authority(&cookie, &identity);
    } else {
        tracing::warn!(%cookie, "polkit: the helper did not authenticate");
    }
    if still_open {
        emit(Event::Cancelled { cookie });
    }
}

/// `AuthenticationAgentResponse2` on the authority — the call that actually
/// completes `CheckAuthorization` on polkitd's side. A fresh system-bus
/// connection per call: prompts are rare, and it keeps the worker free of
/// the agent connection's runtime.
fn respond_to_authority(cookie: &str, identity: &str) {
    let Some(uid) = our_uid() else {
        tracing::warn!(%cookie, "polkit: no uid, cannot answer the authority");
        return;
    };
    let Some(identity) = identity_dict(identity) else {
        tracing::warn!(%cookie, "polkit: malformed identity, cannot answer the authority");
        return;
    };
    zbus::block_on(async {
        let conn = match Connection::system().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!(%e, "polkit: no system bus for the response");
                return;
            }
        };
        let authority = match AuthorityProxy::new(&conn).await {
            Ok(authority) => authority,
            Err(e) => {
                tracing::warn!(%e, "polkit: cannot reach the authority");
                return;
            }
        };
        if let Err(e) = authority
            .authentication_agent_response2(uid, cookie, &identity)
            .await
        {
            tracing::warn!(%e, %cookie, "polkit: AuthenticationAgentResponse2 failed");
        }
    });
}

/// Serve the agent: a stream of [`Event`]s the app turns into dialogs.
/// `register` is the `--real-services` opt-in; without it this is a dead
/// stream — a dev box must not hijack real elevation prompts.
pub fn agent(register: bool) -> Subscription<Event> {
    Subscription::run_with(register, |&register| {
        let (tx, rx) = mpsc::unbounded();
        if register {
            *lock(&EVENT_TX) = Some(tx.clone());
            thread::Builder::new()
                .name("polkit".into())
                .spawn(move || zbus::block_on(run(tx)))
                .map_err(|e| tracing::error!(%e, "polkit: cannot start thread"))
                .ok();
        }
        rx
    })
}

/// The user submitted `password` for `cookie`'s prompt (`identity` is the
/// [`Prompt::identity`] it carried). The helper conversation runs on a
/// worker thread; its end — success or failure — comes back as
/// `Event::Cancelled`, which just closes the dialog.
pub fn respond(cookie: &str, identity: &str, password: String) {
    let cookie = cookie.to_owned();
    let identity = identity.to_owned();
    thread::Builder::new()
        .name("polkit-auth".into())
        .spawn(move || run_session(cookie, identity, password))
        .map_err(|e| tracing::error!(%e, "polkit: cannot start the auth thread"))
        .ok();
}

/// The user dismissed the dialog: kill the conversation for `cookie`, if
/// one is running. Nothing comes back — the app already closed the dialog.
pub fn cancel(cookie: &str) {
    kill_helper(cookie);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(pairs: &[(&str, OwnedVariant)]) -> BTreeMap<String, OwnedVariant> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    fn ov_str(s: &str) -> OwnedVariant {
        OwnedVariant::try_from(Variant::from(s)).expect("a str variant owns")
    }

    #[test]
    fn parses_helper_lines() {
        assert_eq!(
            parse_helper_line("PAM_PROMPT_ECHO_OFF Password: "),
            HelperLine::Prompt {
                echo: false,
                text: "Password:"
            }
        );
        assert_eq!(
            parse_helper_line("PAM_PROMPT_ECHO_OFF"),
            HelperLine::Prompt {
                echo: false,
                text: ""
            }
        );
        assert_eq!(
            parse_helper_line("PAM_PROMPT_ECHO_ON user: "),
            HelperLine::Prompt {
                echo: true,
                text: "user:"
            }
        );
        assert_eq!(
            parse_helper_line("PAM_ERROR_MSG bad password"),
            HelperLine::Error("bad password")
        );
        assert_eq!(
            parse_helper_line("PAM_TEXT_INFO hi"),
            HelperLine::Info("hi")
        );
        assert_eq!(parse_helper_line("SUCCESS"), HelperLine::Success);
        assert_eq!(parse_helper_line("FAILURE"), HelperLine::Failure);
        assert_eq!(parse_helper_line("???"), HelperLine::Unknown);
    }

    #[test]
    fn converse_answers_each_prompt_until_success() {
        let script = b"PAM_PROMPT_ECHO_OFF Password: \nPAM_TEXT_INFO hi\nPAM_PROMPT_ECHO_ON again: \nSUCCESS\n";
        let mut stdin = Vec::new();
        let outcome = converse(BufReader::new(&script[..]), &mut stdin, b"hunter2");
        assert_eq!(outcome, Outcome::Success);
        assert_eq!(stdin, b"hunter2\nhunter2\n");
    }

    #[test]
    fn converse_ends_on_failure_and_eof() {
        let mut stdin = Vec::new();
        assert_eq!(
            converse(
                BufReader::new(&b"PAM_PROMPT_ECHO_OFF\nFAILURE\n"[..]),
                &mut stdin,
                b"x"
            ),
            Outcome::Failure
        );
        assert_eq!(stdin, b"x\n");
        assert_eq!(
            converse(BufReader::new(&b""[..]), &mut stdin, b"x"),
            Outcome::Failure
        );
    }

    #[test]
    fn identity_roundtrip() {
        let by_uid = identity_dict("unix-user:1000").expect("unix-user:1000 parses");
        assert_eq!(by_uid["kind"], Variant::from("unix-user"));
        assert_eq!(by_uid["uid"], Variant::from(1000u32));
        let by_name = identity_dict("unix-user:root").expect("names parse");
        assert_eq!(by_name["name"], Variant::from("root"));
        let group = identity_dict("unix-group:wheel").expect("groups parse");
        assert_eq!(group["kind"], Variant::from("unix-group"));
        assert_eq!(group["name"], Variant::from("wheel"));
        let netgroup = identity_dict("unix-netgroup:adm").expect("netgroups parse");
        assert_eq!(netgroup["name"], Variant::from("adm"));
        assert!(identity_dict("bogus").is_none());
    }

    #[test]
    fn identity_from_polkit_dict() {
        let dict = sv(&[
            ("kind", ov_str("unix-user")),
            ("uid", OwnedVariant::from(1000u32)),
        ]);
        assert_eq!(identity_string(&dict).as_deref(), Some("unix-user:1000"));
        let group = sv(&[
            ("kind", ov_str("unix-group")),
            ("gid", OwnedVariant::from(10u32)),
        ]);
        assert_eq!(identity_string(&group).as_deref(), Some("unix-group:10"));
        let named = sv(&[
            ("kind", ov_str("unix-user")),
            ("name", ov_str("root")),
        ]);
        assert_eq!(identity_string(&named).as_deref(), Some("unix-user:root"));
        let empty = sv(&[]);
        assert!(identity_string(&empty).is_none());
    }

    #[test]
    fn picks_our_identity_first() {
        let root = sv(&[
            ("kind", ov_str("unix-user")),
            ("uid", OwnedVariant::from(0u32)),
        ]);
        let wheel = sv(&[
            ("kind", ov_str("unix-group")),
            ("gid", OwnedVariant::from(10u32)),
        ]);
        let ours = sv(&[
            ("kind", ov_str("unix-user")),
            ("uid", OwnedVariant::from(1000u32)),
        ]);
        let (identity, _user) = pick_identity(&[root, wheel.clone(), ours], Some(1000))
            .expect("a matching identity is picked");
        assert_eq!(identity, "unix-user:1000");
        // No match: the first offered identity is picked.
        let (identity, _user) =
            pick_identity(&[wheel], Some(1000)).expect("the first identity is picked");
        assert_eq!(identity, "unix-group:10");
        assert!(pick_identity(&[], Some(1000)).is_none());
    }

    #[test]
    fn stat_start_time_is_field_22() {
        let stat = "1 (comm) S 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 987654";
        assert_eq!(stat_start_time(stat), Some(987_654));
        // `comm` with spaces and parens: the last ')' still bounds it.
        let stat = "1 (a (b) c) S 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 42";
        assert_eq!(stat_start_time(stat), Some(42));
        assert_eq!(stat_start_time("garbage"), None);
    }

    #[test]
    fn passwd_gives_gecos_else_login() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\n\
                      sid:x:1000:1000:Sid J,101,x,y:/home/sid:/bin/zsh\n";
        assert_eq!(passwd_user(passwd, 1000).as_deref(), Some("Sid J"));
        assert_eq!(passwd_user(passwd, 0).as_deref(), Some("root"));
        assert!(passwd_user(passwd, 42).is_none());
    }
}
