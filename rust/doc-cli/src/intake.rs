//! The intake: filing a report reaches the project's checkout without anyone relaying it.
//!
//! [`crate::reports`] is the local half — this machine's inbox, and the drain that folds it
//! into a `reports.dx`. That half only works when the agent that hit the defect happens to be
//! standing in the dx checkout, which is almost never true: an agent working on some other
//! project hits a defect in dx, files it, and the record sits on *that* machine forever.
//!
//! So a report also travels. One endpoint anybody can POST to holds the database; a checkout
//! **subscribes** to a project and folds that project's open reports into its own
//! `reports.dx`. The next agent in the checkout reads the document and inherits what everyone
//! else ran into, and nobody has to remember to carry anything across.
//!
//! ```text
//!  file  ──▶ this machine's inbox ──▶ POST <endpoint>?<service>       (immediately, best effort)
//!                                        │
//!  sync  ◀── reports.dx, folded ◀── GET <endpoint>/feed?<service>     (every subscribed checkout)
//!  close ──▶ block removed      ──▶ POST <endpoint>/close?<service>   (so a fix does not return)
//! ```
//!
//! The service is the query and nothing else — `…/report?dx` is dx's own database, and
//! `…/report?<name>` is how another internal service registers one. [`address`] is the only
//! place a URL is built, so every call reaches the same box the same way.
//!
//! `docs/intake.dx` is the authority on what crosses the wire — the bodies, the answers, and
//! the computed id both ends have to agree on. This module states when each call is made; it
//! does not restate the shapes.
//!
//! # Why the inbox is still written first
//!
//! The POST is best effort by construction: the reporter may be offline, the box may be down,
//! and neither is a reason to lose what an agent just learned. The inbox record is written
//! before the push is attempted and deleted only once the intake has answered, so a report is
//! either *here* or *there* at every instant, never in flight and nowhere.
//!
//! # Why `curl` rather than an HTTP client
//!
//! The endpoint is HTTPS, and this workspace has no TLS stack — `doc-cli` links no network
//! crate at all, deliberately. `curl` ships with macOS, Windows 10 and later, and every Linux
//! this runs on, and it is used the way `git` and `ssh` are used elsewhere in this repository:
//! as a program, with its arguments controlled here. The token never appears in an argument —
//! it is written to curl's own config on stdin, so it is not in `ps` output for whoever else
//! is on the machine.
//!
//! # What the sync is allowed to change
//!
//! One block per open report, keyed by the fingerprint the intake computed — the same id
//! [`crate::reports::Report::id`] computes, so a report filed locally and the same defect
//! filed from another machine land on one block. A block whose body already says what the
//! database says is left byte-for-byte alone, so a sync that has nothing to add writes
//! nothing and git sees no change.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use doc_core::edit;
use doc_core::format::parse;
use serde_json::{json, Value};

use crate::reports::{self, Report};
use crate::workspace;

/// Where reports go when nothing says otherwise — the base, without a service on it.
///
/// A default rather than a setting, because the whole point is that an agent that has never
/// heard of this machinery still reaches the people who fix dx. The service the report belongs
/// to is the query [`address`] puts on: dx's own reports are `.../report?dx`.
/// `DX_REPORT_ENDPOINT` overrides the base — and names the service when it carries one, so
/// `DX_REPORT_ENDPOINT=https://rockywearsahat.com/report?billing` is the whole of registering
/// `billing`. `DX_REPORT_ENDPOINT=off` turns the push off entirely.
pub const DEFAULT_ENDPOINT: &str = "https://rockywearsahat.com/report";

/// The project reports are about when nothing says otherwise — the service `?dx` names.
pub const DEFAULT_PROJECT: &str = "dx";

/// Environment override for the endpoint. `off` disables the push.
const ENDPOINT_ENV: &str = "DX_REPORT_ENDPOINT";
/// Environment override for the feed token, for a machine that would rather not store one.
const TOKEN_ENV: &str = "DX_REPORT_TOKEN";
/// Environment override for the subscriptions file, so the suite never touches the real one.
const SUBSCRIPTIONS_ENV: &str = "DX_SUBSCRIPTIONS_FILE";
/// Environment override for the machine-wide token file, so the suite never touches the real one.
const TOKEN_FILE_ENV: &str = "DX_REPORT_TOKEN_FILE";

/// How long any one call to the intake may take.
///
/// Short: a reporter is an agent mid-task, and a report that has been stored locally is not
/// worth ten seconds of anybody's turn. What does not go through now goes through on the next
/// sync.
const TIMEOUT_SECONDS: u64 = 8;

/// A checkout that receives one project's reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    /// The workspace root whose `reports.dx` the reports are folded into.
    pub workspace: PathBuf,
    /// The project key on the intake — `dx`.
    pub project: String,
    /// The intake's base URL.
    pub endpoint: String,
    /// The owner's feed token. Empty means this machine can push but not read.
    pub token: String,
}

impl Subscription {
    /// This subscription as the JSON one line of the subscriptions file holds.
    fn to_json(&self) -> Value {
        json!({
            "workspace": self.workspace.display().to_string(),
            "project": self.project,
            "endpoint": self.endpoint,
            "token": self.token,
        })
    }

    /// The subscription a stored record stands for, or `None` when the record is not one.
    fn from_json(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default();
        let workspace = text("workspace");
        if workspace.is_empty() {
            return None;
        }
        Some(Self {
            workspace: PathBuf::from(workspace),
            project: nonempty(text("project"), DEFAULT_PROJECT),
            endpoint: nonempty(text("endpoint"), DEFAULT_ENDPOINT),
            token: text("token").to_string(),
        })
    }

    /// The document this subscription writes.
    #[must_use]
    pub fn document(&self) -> PathBuf {
        self.workspace.join(reports::DOCUMENT)
    }
}

/// The URL one call goes to: the intake's base, the route when the call has one, and the
/// service as the query — `https://rockywearsahat.com/report?dx`, `…/report/feed?dx`,
/// `…/report/close?dx`.
///
/// A bare query key rather than `project=`, because that is the address the intake publishes:
/// `report?<service>` is what a new internal service registers under. An `endpoint` that
/// already carries a service is tolerated and its query replaced, so a subscription stored
/// with the full address and one stored with the base both reach the same place.
#[must_use]
pub fn address(endpoint: &str, route: &str, project: &str) -> String {
    let (base, named) = split_service(endpoint);
    let service = match project.trim() {
        "" => named.unwrap_or_else(|| DEFAULT_PROJECT.to_string()),
        stated => stated.to_string(),
    };
    if route.is_empty() {
        format!("{base}?{service}")
    } else {
        format!("{base}/{route}?{service}")
    }
}

/// An endpoint split into the base the routes are built from and the service its query names.
///
/// `https://rockywearsahat.com/report?billing` splits into that base and `Some("billing")`. A
/// query that is not a bare service name belongs to nothing this understands and is dropped
/// rather than carried into a URL that already has a `?` in it.
#[must_use]
pub fn split_service(endpoint: &str) -> (String, Option<String>) {
    let trimmed = endpoint.trim();
    let (base, query) = trimmed.split_once('?').unwrap_or((trimmed, ""));
    let named = query
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (
        base.trim_end_matches('/').to_string(),
        (named && !query.is_empty()).then(|| query.to_string()),
    )
}

/// The endpoint this machine files to, or `None` when the push is turned off.
#[must_use]
pub fn endpoint() -> Option<String> {
    endpoint_from(std::env::var(ENDPOINT_ENV).ok().as_deref())
}

/// The service this machine's endpoint setting names, when it names one.
///
/// `None` means nothing was said, and the caller's own project key — `dx` for a report about
/// dx — stands.
#[must_use]
pub fn service() -> Option<String> {
    service_from(std::env::var(ENDPOINT_ENV).ok().as_deref())
}

/// The project a filed report belongs to — always `dx`, unless this machine's own
/// `DX_REPORT_ENDPOINT` names something else on purpose.
///
/// A report is about dx itself, filed from whatever project the reporter happens to be
/// standing in ([`crate::mcp::tools`]'s `dx_report` description is the authority on that), so
/// the workspace it was filed from has no say in where it goes — a workspace's own
/// subscription (`dx report subscribe --project lvlup`) only decides what `dx report sync`
/// reads back into its local reports.dx, never where a report just filed here is sent.
#[must_use]
pub fn project_for() -> String {
    service().unwrap_or_else(|| DEFAULT_PROJECT.to_string())
}

/// The service `setting` names in its query, if it names one. Split from [`service`] for the
/// same reason [`endpoint_from`] is split from [`endpoint`]: a suite must not write a
/// process-wide variable to test the rule.
#[must_use]
pub fn service_from(setting: Option<&str>) -> Option<String> {
    setting
        .filter(|value| !value.trim().eq_ignore_ascii_case("off"))
        .and_then(|value| split_service(value).1)
}

/// The endpoint `setting` names: itself, or the default when it says nothing, or nothing at
/// all when it says `off`.
///
/// Split from [`endpoint`] so the rule can be tested without writing a process-wide
/// environment variable — a suite that mutates one is a suite where another test filing a
/// report can find it unset and reach the real intake.
#[must_use]
pub fn endpoint_from(setting: Option<&str>) -> Option<String> {
    match setting {
        Some(value) if value.trim().eq_ignore_ascii_case("off") => None,
        Some(value) if !value.trim().is_empty() => Some(split_service(value).0),
        _ => Some(DEFAULT_ENDPOINT.to_string()),
    }
}

/// The file this machine's subscriptions live in.
///
/// Beside the report inbox, in durable per-user data and never inside a repository: it holds a
/// token, and a token in a checkout is a token in somebody's next commit.
#[must_use]
pub fn subscriptions_file() -> PathBuf {
    if let Some(explicit) = std::env::var_os(SUBSCRIPTIONS_ENV) {
        if !explicit.is_empty() {
            return PathBuf::from(explicit);
        }
    }
    crate::home::data_dir()
        .join("dx")
        .join("subscriptions.json")
}

/// The file this machine's stored report token lives in.
///
/// One token, machine-wide, beside the subscriptions — every subscription on this machine reads
/// the same box, so the token is a property of the machine, not of any one checkout; stored
/// once, and setup in any repository needs nothing further.
#[must_use]
pub fn token_file() -> PathBuf {
    if let Some(explicit) = std::env::var_os(TOKEN_FILE_ENV) {
        if !explicit.is_empty() {
            return PathBuf::from(explicit);
        }
    }
    crate::home::data_dir().join("dx").join("report-token")
}

/// Reads the raw token and endpoint base from the stored token file, or `None` when missing
/// or empty.
///
/// This is the internal reader; callers should use [`stored_token_for`] to get a token bound
/// to an endpoint.
fn read_stored_token_pair() -> Option<(String, String)> {
    let path = token_file();
    std::fs::read_to_string(&path).ok().and_then(|text| {
        if text.trim().is_empty() {
            return None;
        }
        let mut lines = text.lines();
        let token = lines.next()?.trim().to_string();
        if token.is_empty() {
            return None;
        }
        let endpoint = lines
            .next()
            .map(|line| line.trim().to_string())
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_string());
        Some((token, endpoint))
    })
}

/// The token this machine has stored for reading feeds when it belongs to `endpoint`, or
/// `None` when the token file is missing, holds only whitespace, or was stored for a
/// different endpoint.
///
/// Tokens are bound to the endpoint they were stored for: a machine-wide token always goes
/// to a specific database. A token stored for the default endpoint will not travel to
/// anywhere else, and a foreign endpoint gets no token this machine ever acquired.
#[must_use]
pub fn stored_token_for(endpoint: &str) -> Option<String> {
    let (token, stored_base) = read_stored_token_pair()?;
    let (request_base, _) = split_service(endpoint);
    if stored_base == request_base {
        Some(token)
    } else {
        None
    }
}

/// Stores a token for this machine to use reading feeds from `endpoint`.
///
/// The token is bound to the endpoint base it was stored for — only requests to the same
/// endpoint will see it. This prevents a token acquired for one database from traveling to
/// another through [`token_for`]'s precedence.
///
/// Creates the parent directory if needed, writes the token on the first line and the
/// endpoint base on the second, and restricts the file to the owning user.
///
/// # Errors
/// Returns a sentence naming the path when the parent cannot be created or the file cannot
/// be written.
pub fn store_token(token: &str, endpoint: &str) -> Result<PathBuf, String> {
    let path = token_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let trimmed_token = token.trim();
    let (base, _) = split_service(endpoint);
    std::fs::write(&path, format!("{trimmed_token}\n{base}\n"))
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    restrict(&path);
    Ok(path)
}

/// Every subscription this machine holds.
///
/// # Errors
/// Returns a sentence when the file exists but cannot be read or is not a list — never
/// silence, because a subscription that has quietly stopped working looks exactly like a
/// project with no reports.
pub fn subscriptions() -> Result<Vec<Subscription>, String> {
    let path = subscriptions_file();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{} is not readable: {error}", path.display()))?;
    let list = value
        .as_array()
        .ok_or_else(|| format!("{} should hold a list of subscriptions", path.display()))?;
    Ok(list.iter().filter_map(Subscription::from_json).collect())
}

/// Records `subscription`, replacing any this machine already had for that workspace.
///
/// # Errors
/// Returns a sentence when the file cannot be written.
pub fn subscribe(subscription: &Subscription) -> Result<(), String> {
    let mut held = subscriptions()?;
    held.retain(|existing| existing.workspace != subscription.workspace);
    held.push(subscription.clone());

    let path = subscriptions_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(&Value::Array(
        held.iter().map(Subscription::to_json).collect(),
    ))
    .map_err(|error| format!("could not encode the subscriptions: {error}"))?;
    std::fs::write(&path, format!("{body}\n"))
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    restrict(&path);
    Ok(())
}

/// Forgets the subscription for `workspace`, if there was one.
///
/// # Errors
/// Returns a sentence when the file cannot be rewritten.
pub fn unsubscribe(workspace: &Path) -> Result<bool, String> {
    let mut held = subscriptions()?;
    let before = held.len();
    held.retain(|existing| existing.workspace != workspace);
    if held.len() == before {
        return Ok(false);
    }
    let path = subscriptions_file();
    let body = serde_json::to_string_pretty(&Value::Array(
        held.iter().map(Subscription::to_json).collect(),
    ))
    .map_err(|error| format!("could not encode the subscriptions: {error}"))?;
    std::fs::write(&path, format!("{body}\n"))
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(true)
}

/// The subscription covering `workspace`, if this machine holds one.
///
/// # Errors
/// Returns a sentence when the subscriptions cannot be read.
pub fn subscription_for(workspace: &Path) -> Result<Option<Subscription>, String> {
    Ok(subscriptions()?
        .into_iter()
        .find(|subscription| subscription.workspace == workspace))
}

/// Pushes one report to `endpoint`, returning the id the intake filed it under.
///
/// # Errors
/// Returns a sentence naming what refused: no `curl` on this machine, a network that did not
/// answer, or the intake's own refusal, which is already written for an agent to read.
pub fn push(report: &Report, endpoint: &str, project: &str) -> Result<String, String> {
    let body = serde_json::to_string(&json!({
        "project": project,
        "kind": report.kind.as_str(),
        "title": report.title,
        "detail": report.detail,
        "route": report.route,
        "repro": report.repro,
        "tool": format!("dx {}", report.dx),
        "platform": report.platform,
        "workspace": workspace_name(&report.workspace),
    }))
    .map_err(|error| format!("could not encode the report: {error}"))?;

    let url = address(endpoint, "", project);
    let answered = run_curl(
        &[
            "-sS",
            "--max-time",
            &TIMEOUT_SECONDS.to_string(),
            "-X",
            "POST",
            "-H",
            "content-type: application/json",
            "--data-binary",
            "@-",
            &url,
        ],
        Some(body.as_bytes()),
    )?;

    let value: Value = serde_json::from_str(&answered)
        .map_err(|_| format!("{url} answered something that is not JSON: {answered}"))?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(format!("{url} refused the report: {error}"));
    }
    value
        .get("filed")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("{url} did not say what it filed: {answered}"))
}

/// Every open report the intake holds for `project`.
///
/// # Errors
/// Returns a sentence when the feed cannot be read — including the token being wrong, which is
/// the one failure that otherwise reads as "no reports".
pub fn feed(endpoint: &str, project: &str, token: &str) -> Result<Vec<Value>, String> {
    let url = address(endpoint, "feed", project);
    if token.trim().is_empty() {
        return Err(format!(
            "reading {url} needs the owner's token — run `selfhost reports token` on the box and \
             pass it to `dx report subscribe --token`"
        ));
    }
    let config = format!("url = \"{url}\"\nheader = \"authorization: Bearer {token}\"\n");
    let answered = run_curl(
        &["-sS", "--max-time", &TIMEOUT_SECONDS.to_string(), "-K", "-"],
        Some(config.as_bytes()),
    )?;
    let value: Value = serde_json::from_str(&answered)
        .map_err(|_| format!("{url} answered something that is not JSON: {answered}"))?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(format!("{url} refused: {error}"));
    }
    // An answer with no list is not an empty list. A sync prunes local reports against
    // this, and "the box said nothing" must never read as "the box said nothing is open" —
    // that reading would empty a document of every report it holds.
    let Some(reports) = value.get("reports").and_then(Value::as_array) else {
        return Err(format!("{url} answered no report list: {answered}"));
    };
    Ok(reports.clone())
}

/// Tells the intake that `id` is fixed, so no later sync folds it back in.
///
/// # Errors
/// Returns a sentence when the intake refuses or cannot be reached.
pub fn close(endpoint: &str, project: &str, id: &str, token: &str) -> Result<(), String> {
    if !id.starts_with("report-") || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(format!("`{id}` is not a report id"));
    }
    let url = address(endpoint, "close", project);
    if token.trim().is_empty() {
        return Err(format!("closing a report at {url} needs the owner's token"));
    }
    let config = format!(
        "url = \"{url}\"\nheader = \"authorization: Bearer {token}\"\n\
         header = \"content-type: application/json\"\ndata = \"{{\\\"project\\\":\\\"{project}\\\",\\\"id\\\":\\\"{id}\\\"}}\"\n"
    );
    let answered = run_curl(
        &["-sS", "--max-time", &TIMEOUT_SECONDS.to_string(), "-K", "-"],
        Some(config.as_bytes()),
    )?;
    let value: Value = serde_json::from_str(&answered)
        .map_err(|_| format!("{url} answered something that is not JSON: {answered}"))?;
    match value.get("error").and_then(Value::as_str) {
        Some(error) => Err(format!("{url} refused: {error}")),
        None => Ok(()),
    }
}

/// What filing one report did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filed {
    /// The id the report was filed under, here and at the intake — they are the same string.
    pub id: String,
    /// The intake that accepted it, when one did.
    pub reached: Option<String>,
    /// How many records are still waiting in this machine's inbox.
    pub waiting: usize,
    /// Why the intake was not reached, when it was not. Never fatal: the report is filed here.
    pub problem: Option<String>,
}

impl Filed {
    /// The sentence the reporter is given — where the report went, and what happens next.
    #[must_use]
    pub fn summary(&self, kind: &str, inbox: &Path) -> String {
        match (&self.reached, &self.problem) {
            (Some(endpoint), problem) => format!(
                "filed {} ({kind}) — it is in the database at {endpoint} and will be in \
                 {} the next time this checkout syncs\n{}",
                self.id,
                reports::DOCUMENT,
                problem
                    .as_ref()
                    .map(|said| format!("{said}\n"))
                    .unwrap_or_default()
            ),
            (None, Some(problem)) => {
                let is_permanent = problem.contains("refused the report");
                if is_permanent {
                    format!(
                        "filed {} ({kind}) in {} — {} waiting, and this one will not go out \
                         because the intake refused it: {problem}\nremove it with \
                         `dx report drop {}`\n",
                        self.id,
                        inbox.display(),
                        self.waiting,
                        self.id
                    )
                } else {
                    format!(
                        "filed {} ({kind}) in {} — {} waiting, because the intake could not be \
                         reached: {problem}\nthey go out on the next `dx report sync`\n",
                        self.id,
                        inbox.display(),
                        self.waiting
                    )
                }
            }
            (None, None) => format!(
                "filed {} ({kind}) — {} waiting in {}\nrun `dx report drain` in the dx checkout \
                 to fold them into {}\n",
                self.id,
                self.waiting,
                inbox.display(),
                reports::DOCUMENT
            ),
        }
    }
}

/// Files `report` on this machine and, if this machine can reach one, at the intake.
///
/// The order is the guarantee: the inbox record is written first, so a report exists on disk
/// before anything is attempted over a network, and it is removed only once the intake has
/// answered with an id. A push that fails is not an error — the report is filed, and the next
/// sync carries it.
///
/// # Errors
/// Returns a sentence only when the report cannot be written to this machine's inbox at all,
/// which is the one case where the reporter must be told it was not filed.
pub fn file(report: &Report) -> Result<Filed, String> {
    let inbox = reports::inbox();
    let (id, record) = reports::file_record(report, &inbox)?;

    let Some(endpoint) = endpoint() else {
        return Ok(Filed {
            id,
            reached: None,
            waiting: reports::read_inbox(&inbox)?.pending.len(),
            problem: None,
        });
    };
    let project = project_for();
    match push(report, &endpoint, &project) {
        Ok(filed) => {
            // The intake has it, so this machine no longer needs to — and a record that
            // cannot be removed is said out loud rather than swallowed: the next sync would
            // push it again, which costs a duplicate sighting nobody saw twice.
            let problem = std::fs::remove_file(&record).err().map(|error| {
                format!(
                    "{filed} reached the intake but {} could not be removed: {error} — remove \
                     it by hand, or the next sync files a sighting nobody saw",
                    record.display()
                )
            });
            Ok(Filed {
                id: filed,
                reached: Some(endpoint),
                waiting: reports::read_inbox(&inbox)?.pending.len(),
                problem,
            })
        }
        Err(problem) => Ok(Filed {
            id,
            reached: None,
            waiting: reports::read_inbox(&inbox)?.pending.len(),
            problem: Some(problem),
        }),
    }
}

/// What one sync did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Synced {
    /// Reports pushed out of this machine's inbox.
    pub pushed: Vec<String>,
    /// Reports the document did not have and now does.
    pub added: Vec<String>,
    /// Reports whose block changed — another sighting, usually.
    pub updated: Vec<String>,
    /// Ids the feed no longer lists — closed at the intake, by this machine or another, and
    /// removed here so the document stops claiming an open defect nobody will ever see again.
    pub closed: Vec<String>,
    /// What could not be done, each already a sentence. A sync reports these rather than
    /// failing: a feed that is unreachable must not stop the inbox being flushed, or the other
    /// way round.
    pub problems: Vec<String>,
}

impl Synced {
    /// The sentence a reader gets.
    #[must_use]
    pub fn summary(&self, document: &Path) -> String {
        let mut lines = Vec::new();
        if !self.pushed.is_empty() {
            lines.push(format!("pushed {} report(s)", self.pushed.len()));
        }
        if self.added.is_empty() && self.updated.is_empty() && self.closed.is_empty() {
            lines.push(format!("{} is up to date", document.display()));
        } else {
            lines.push(format!(
                "{}: {} new, {} updated, {} closed elsewhere",
                document.display(),
                self.added.len(),
                self.updated.len(),
                self.closed.len()
            ));
            for id in self.added.iter().chain(&self.updated).chain(&self.closed) {
                lines.push(format!("  {id}"));
            }
        }
        for problem in &self.problems {
            lines.push(format!("  {problem}"));
        }
        lines.join("\n")
    }

    /// Whether this sync changed the document.
    #[must_use]
    pub fn changed(&self) -> bool {
        !self.added.is_empty() || !self.updated.is_empty() || !self.closed.is_empty()
    }
}

/// Flushes this machine's inbox to the intake, then folds the project's open reports into the
/// subscription's document.
///
/// Both halves are attempted whatever the other does, and every failure is collected rather
/// than thrown: a box that is down must not strand the inbox, and an inbox that cannot be
/// pushed must not stop the document being brought up to date.
///
/// # Errors
/// Returns a sentence only when the document itself cannot be read or written — the case where
/// carrying on would mean reporting success over a document nobody updated.
pub fn sync(subscription: &Subscription) -> Result<Synced, String> {
    let mut synced = Synced::default();

    match reports::read_inbox(&reports::inbox()) {
        Ok(inbox) => {
            for waiting in inbox.pending {
                match push(
                    &waiting.report,
                    &subscription.endpoint,
                    &subscription.project,
                ) {
                    Ok(id) => {
                        // The intake has it, so this machine no longer needs to.
                        if let Err(error) = std::fs::remove_file(&waiting.record) {
                            synced.problems.push(format!(
                                "{id} reached the intake but {} could not be removed: {error}",
                                waiting.record.display()
                            ));
                        }
                        synced.pushed.push(id);
                    }
                    Err(reason) => {
                        synced.problems.push(reason);
                        break;
                    }
                }
            }
        }
        Err(reason) => synced.problems.push(reason),
    }

    let entries = match feed(
        &subscription.endpoint,
        &subscription.project,
        &token_for(subscription),
    ) {
        Ok(entries) => entries,
        Err(reason) => {
            synced.problems.push(reason);
            return Ok(synced);
        }
    };
    let folded = fold(&entries, &subscription.document())?;
    synced.added = folded.added;
    synced.updated = folded.updated;
    synced.closed = folded.closed;
    synced.problems.extend(folded.problems);
    Ok(synced)
}

/// What one fold did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folded {
    /// Ids that became new blocks.
    pub added: Vec<String>,
    /// Ids whose block body changed.
    pub updated: Vec<String>,
    /// Ids removed because the feed no longer lists them — closed at the intake since this
    /// checkout last synced, by this machine or another.
    pub closed: Vec<String>,
    /// Records that could not be folded, each a sentence.
    pub problems: Vec<String>,
}

/// Writes every report in `entries` into the document at `path`, one block each, then removes
/// any locally open report the feed no longer lists.
///
/// A block whose body already matches is left alone, so this is safe to run on a timer: a fold
/// with nothing to say touches no file and produces no diff. The reverse direction matters just
/// as much: a report closed at the intake — by this machine's own `dx report close`, or by
/// another machine on the same subscription — must stop being open here too, or a document that
/// claims to be synced quietly disagrees with the database it synced from.
///
/// # Errors
/// Returns a sentence when the document cannot be read or saved.
pub fn fold(entries: &[Value], path: &Path) -> Result<Folded, String> {
    let mut folded = Folded::default();
    if entries.is_empty() && !path.exists() {
        // Nothing to write and nothing to prune: a document that does not exist cannot be
        // holding a report the intake has already closed.
        return Ok(folded);
    }
    if !path.exists() {
        workspace::save(path, &parse(reports::SCAFFOLD))?;
    }

    for entry in entries {
        let Some(id) = entry.get("id").and_then(Value::as_str) else {
            folded
                .problems
                .push("the intake sent a report with no id".to_string());
            continue;
        };
        if !id.starts_with("report-") {
            folded.problems.push(format!("`{id}` is not a report id"));
            continue;
        }
        let body = block_body(entry);
        let source = workspace::read(path)?;
        let document = parse(&source);

        let updated = match edit::find(&document, id) {
            Ok(index) => {
                if edit::body(&document.blocks[index]) == body {
                    continue;
                }
                folded.updated.push(id.to_string());
                edit::set_block(&source, id, &body)?
            }
            Err(_) => {
                let insertion = edit::Insertion {
                    kind: "paragraph",
                    body: &body,
                    id,
                    level: 0,
                    language: "",
                    run: false,
                    deps: "",
                };
                let last = document.blocks.last().map(|block| block.id.clone());
                folded.added.push(id.to_string());
                edit::insert_after(&source, last.as_deref(), &insertion)?.0
            }
        };
        workspace::save_source(path, &updated)?;
    }

    // The converse question, which is what makes this a sync rather than an accumulation:
    // a block whose id the feed no longer lists was closed at the intake, and the document
    // is claiming an open defect nobody will ever see again.
    let listed: std::collections::HashSet<&str> = entries
        .iter()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .collect();
    for open in reports::open_reports(path)? {
        if listed.contains(open.id.as_str()) {
            continue;
        }
        let source = workspace::read(path)?;
        let without = edit::remove_block(&source, &open.id)?;
        workspace::save_source(path, &without)?;
        folded.closed.push(open.id);
    }
    Ok(folded)
}

/// The block body one stored report becomes — the same shape a local drain writes, so a
/// document cannot be told which half of the machinery filed a report.
fn block_body(entry: &Value) -> String {
    let text = |key: &str| {
        entry
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let mut lines = vec![
        format!("**{} — {}**", text("kind"), text("title")),
        text("detail"),
    ];
    let route = text("route");
    if !route.is_empty() {
        lines.push(format!("Route: {route}"));
    }
    let repro = text("repro");
    if !repro.is_empty() {
        lines.push(format!("Repro: {repro}"));
    }
    for sighting in entry
        .get("seen")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
    {
        let field = |key: &str| {
            sighting
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        lines.push(format!(
            "seen {} · {} · {} · {}",
            field("at"),
            or_dash(&field("tool")),
            or_dash(&field("platform")),
            or_dash(&field("workspace"))
        ));
    }
    lines.join("\n")
}

/// The value, or a dash when the reporter did not give one.
fn or_dash(value: &str) -> String {
    if value.is_empty() {
        "—".to_string()
    } else {
        value.to_string()
    }
}

/// The name of a workspace directory — never the path, which is somebody's machine layout and
/// no business of a database anyone can read.
fn workspace_name(workspace: &str) -> String {
    workspace
        .rsplit(['/', '\\'])
        .find(|part| !part.trim().is_empty())
        .unwrap_or_default()
        .to_string()
}

/// `value` unless it is blank, in which case `fallback`.
fn nonempty(value: &str, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value.trim().to_string()
    }
}

/// The token for `subscription`, with precedence: the TOKEN_ENV environment variable, else the
/// subscription's own token, else the machine-wide stored token bound to its endpoint, else empty.
///
/// A subscription written before the machine store existed keeps working — its own token is
/// carried and used. One written after needs no token of its own; the machine's stored token
/// reaches it via this precedence, but only if it was stored for the same endpoint.
#[must_use]
pub fn token_for(subscription: &Subscription) -> String {
    if let Ok(value) = std::env::var(TOKEN_ENV) {
        if !value.trim().is_empty() {
            return value;
        }
    }
    if !subscription.token.is_empty() {
        return subscription.token.clone();
    }
    stored_token_for(&subscription.endpoint).unwrap_or_default()
}

/// Runs `curl` with `arguments`, writing `input` to its standard input.
///
/// # Errors
/// Returns a sentence when curl is missing, cannot be run, or exits non-zero — with whatever it
/// said on stderr, because that is where the reason for a network failure is.
fn run_curl(arguments: &[&str], input: Option<&[u8]>) -> Result<String, String> {
    let mut child = Command::new("curl")
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            format!(
                "could not run curl: {error} — dx pushes reports with curl, which ships with \
                 macOS, Windows 10 and later, and every supported Linux"
            )
        })?;
    if let Some(bytes) = input {
        child
            .stdin
            .as_mut()
            .ok_or("curl would not take the request on stdin")?
            .write_all(bytes)
            .map_err(|error| format!("could not hand curl the request: {error}"))?;
    }
    drop(child.stdin.take());

    let finished = child
        .wait_with_output()
        .map_err(|error| format!("curl did not finish: {error}"))?;
    if !finished.status.success() {
        let said = String::from_utf8_lossy(&finished.stderr).trim().to_string();
        return Err(format!(
            "curl could not reach the intake{}",
            if said.is_empty() {
                String::new()
            } else {
                format!(": {said}")
            }
        ));
    }
    Ok(String::from_utf8_lossy(&finished.stdout).trim().to_string())
}

/// Restricts a file holding a token to its owner, where the platform has such a concept.
fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reports::Kind;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-intake-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        root
    }

    fn stored(id: &str, title: &str, sightings: &[&str]) -> Value {
        json!({
            "id": id,
            "project": "dx",
            "kind": "bug",
            "title": title,
            "detail": "The search answered with the heading.",
            "route": "dx_search",
            "repro": "dx search gpu",
            "seen": sightings.iter().map(|at| json!({
                "at": at,
                "tool": "dx 0.1.0",
                "platform": "macos",
                "workspace": "DOC",
            })).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn folding_writes_one_block_per_report_and_is_idempotent() {
        let root = scratch("fold");
        let document = root.join("reports.dx");
        let entries = vec![stored(
            "report-1a2b3c4d",
            "search misses",
            &["2026-08-11T00:00:00Z"],
        )];

        let first = fold(&entries, &document).expect("fold");
        assert_eq!(first.added, vec!["report-1a2b3c4d"]);
        let text = workspace::read(&document).expect("read");
        assert!(text.contains("**bug — search misses**"), "{text}");
        assert!(text.contains("Route: dx_search"), "{text}");
        assert!(
            text.contains("seen 2026-08-11T00:00:00Z · dx 0.1.0 · macos · DOC"),
            "{text}"
        );

        let again = fold(&entries, &document).expect("fold again");
        assert!(
            again.added.is_empty() && again.updated.is_empty(),
            "{again:?}"
        );
        assert_eq!(
            workspace::read(&document).expect("read"),
            text,
            "an idle sync rewrites nothing"
        );
    }

    #[test]
    fn another_sighting_updates_the_block_rather_than_adding_a_second() {
        let root = scratch("update");
        let document = root.join("reports.dx");
        fold(
            &[stored(
                "report-1a2b3c4d",
                "search misses",
                &["2026-08-11T00:00:00Z"],
            )],
            &document,
        )
        .expect("first");

        let grown = fold(
            &[stored(
                "report-1a2b3c4d",
                "search misses",
                &["2026-08-11T00:00:00Z", "2026-08-12T00:00:00Z"],
            )],
            &document,
        )
        .expect("second");
        assert_eq!(grown.updated, vec!["report-1a2b3c4d"]);

        let text = workspace::read(&document).expect("read");
        assert_eq!(text.matches("**bug — search misses**").count(), 1);
        assert_eq!(text.matches("seen ").count(), 2);
    }

    #[test]
    fn a_report_the_feed_no_longer_lists_is_taken_out_of_the_document() {
        let root = scratch("prune-one");
        let document = root.join("reports.dx");
        fold(
            &[
                stored(
                    "report-1a2b3c4d",
                    "search misses",
                    &["2026-08-11T00:00:00Z"],
                ),
                stored(
                    "report-2b3c4d5e",
                    "board draws stale",
                    &["2026-08-12T00:00:00Z"],
                ),
            ],
            &document,
        )
        .expect("first");

        let pruned = fold(
            &[stored(
                "report-1a2b3c4d",
                "search misses",
                &["2026-08-11T00:00:00Z"],
            )],
            &document,
        )
        .expect("prune");
        assert_eq!(pruned.closed, vec!["report-2b3c4d5e"]);
        assert!(pruned.added.is_empty() && pruned.updated.is_empty());

        let text = workspace::read(&document).expect("read");
        assert!(text.contains("search misses"), "{text}");
        assert!(!text.contains("board draws stale"), "{text}");
    }

    #[test]
    fn a_feed_with_nothing_open_empties_the_document_of_reports() {
        let root = scratch("prune-all");
        let document = root.join("reports.dx");
        fold(
            &[stored(
                "report-1a2b3c4d",
                "search misses",
                &["2026-08-11T00:00:00Z"],
            )],
            &document,
        )
        .expect("first");

        let emptied = fold(&[], &document).expect("empty feed");
        assert_eq!(emptied.closed, vec!["report-1a2b3c4d"]);

        let text = workspace::read(&document).expect("read");
        assert!(!text.contains("search misses"), "{text}");
    }

    #[test]
    fn folding_the_same_feed_twice_still_rewrites_nothing_on_the_second_pass() {
        let root = scratch("prune-idle");
        let document = root.join("reports.dx");
        let entries = vec![stored(
            "report-1a2b3c4d",
            "search misses",
            &["2026-08-11T00:00:00Z"],
        )];
        fold(&entries, &document).expect("first");
        let text = workspace::read(&document).expect("read");

        let again = fold(&entries, &document).expect("second");
        assert!(
            again.added.is_empty() && again.updated.is_empty() && again.closed.is_empty(),
            "{again:?}"
        );
        assert_eq!(
            workspace::read(&document).expect("read"),
            text,
            "a prune pass with nothing to prune must not touch the document"
        );
    }

    #[test]
    fn a_report_the_feed_no_longer_lists_is_a_close_not_an_up_to_date() {
        let synced = Synced {
            closed: vec!["report-1a2b3c4d".to_string()],
            ..Synced::default()
        };
        let summary = synced.summary(Path::new("reports.dx"));
        assert!(!summary.contains("is up to date"), "{summary}");
        assert!(summary.contains("1 closed elsewhere"), "{summary}");
        assert!(summary.contains("report-1a2b3c4d"), "{summary}");
        assert!(synced.changed());
    }

    #[test]
    fn an_answer_with_no_report_list_is_a_problem_rather_than_an_empty_feed() {
        use std::io::{BufRead, BufReader, Write as _};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("head");
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            let answer = "{\"ok\":true}";
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                        answer.len()
                    )
                    .as_bytes(),
                )
                .expect("answer");
        });

        let error = feed(&format!("http://{address}"), "dx", "t").expect_err("no report list");
        assert!(error.contains("answered no report list"), "{error}");
        server.join().expect("listener");
    }

    #[test]
    fn a_report_the_intake_sent_without_an_id_is_named_rather_than_written() {
        let root = scratch("no-id");
        let document = root.join("reports.dx");
        let folded = fold(&[json!({"title": "nameless"})], &document).expect("fold");
        assert!(folded.added.is_empty());
        assert_eq!(folded.problems.len(), 1);
        assert!(
            folded.problems[0].contains("no id"),
            "{:?}",
            folded.problems
        );
    }

    #[test]
    fn a_subscription_round_trips_through_the_file_it_is_stored_in() {
        let _env = crate::env_lock();
        let root = scratch("subscribe");
        std::env::set_var(SUBSCRIPTIONS_ENV, root.join("subscriptions.json"));

        let subscription = Subscription {
            workspace: root.clone(),
            project: "dx".to_string(),
            endpoint: "https://example.com/report".to_string(),
            token: "a-token".to_string(),
        };
        subscribe(&subscription).expect("subscribe");
        assert_eq!(
            subscription_for(&root).expect("read"),
            Some(subscription.clone())
        );

        // Subscribing the same workspace again replaces rather than duplicates.
        subscribe(&Subscription {
            project: "self-host".to_string(),
            ..subscription.clone()
        })
        .expect("resubscribe");
        assert_eq!(subscriptions().expect("read").len(), 1);
        assert_eq!(
            subscription_for(&root).expect("read").expect("one").project,
            "self-host"
        );

        assert!(unsubscribe(&root).expect("unsubscribe"));
        assert!(subscription_for(&root).expect("read").is_none());
        std::env::remove_var(SUBSCRIPTIONS_ENV);
    }

    /// The bug this closes: a workspace subscribed to another project's own service (`dx
    /// report subscribe --project lvlup`, run so `dx report sync` can read that project's
    /// reports back) must not thereby redirect *dx's own* reports — filed with `dx_report`
    /// from inside that workspace — into the subscribed project's database. `dx_report`
    /// always reports on dx, so it always defaults to dx's own shared database, no matter
    /// which project's reports the workspace happens to be subscribed to read back.
    #[test]
    fn project_for_defaults_to_dx_regardless_of_the_workspaces_own_subscription() {
        let _env = crate::env_lock();
        let root = scratch("project-for");
        std::env::set_var(SUBSCRIPTIONS_ENV, root.join("subscriptions.json"));
        std::env::remove_var(ENDPOINT_ENV);

        // Nothing subscribed yet: the shared `dx` database stands.
        assert_eq!(project_for(), DEFAULT_PROJECT);

        subscribe(&Subscription {
            workspace: root.clone(),
            project: "lvlup".to_string(),
            endpoint: DEFAULT_ENDPOINT.to_string(),
            token: "t".to_string(),
        })
        .expect("subscribe");
        assert_eq!(
            project_for(),
            DEFAULT_PROJECT,
            "a workspace's own subscription reads reports back locally — it must not \
             redirect where dx_report's own filings go"
        );

        // An explicit override still wins — this machine named somewhere else on purpose.
        std::env::set_var(ENDPOINT_ENV, "https://elsewhere.example/report?billing");
        assert_eq!(project_for(), "billing");

        std::env::remove_var(ENDPOINT_ENV);
        std::env::remove_var(SUBSCRIPTIONS_ENV);
    }

    #[test]
    fn the_endpoint_can_be_pointed_elsewhere_or_turned_off() {
        assert_eq!(
            endpoint_from(Some("https://elsewhere.example/report/")).as_deref(),
            Some("https://elsewhere.example/report")
        );
        assert!(endpoint_from(Some("off")).is_none());
        assert!(endpoint_from(Some("OFF")).is_none());
        assert_eq!(endpoint_from(None).as_deref(), Some(DEFAULT_ENDPOINT));
        assert_eq!(endpoint_from(Some("  ")).as_deref(), Some(DEFAULT_ENDPOINT));
    }

    /// The published address, built from the base: this is what a reporter, a feed, and a
    /// close actually reach.
    #[test]
    fn every_call_reaches_the_service_named_in_the_query() {
        assert_eq!(
            address(DEFAULT_ENDPOINT, "", DEFAULT_PROJECT),
            "https://rockywearsahat.com/report?dx"
        );
        assert_eq!(
            address(DEFAULT_ENDPOINT, "feed", "dx"),
            "https://rockywearsahat.com/report/feed?dx"
        );
        assert_eq!(
            address(DEFAULT_ENDPOINT, "close", "dx"),
            "https://rockywearsahat.com/report/close?dx"
        );
        // Registering another internal service is the same URL under its own name.
        assert_eq!(
            address(DEFAULT_ENDPOINT, "", "billing"),
            "https://rockywearsahat.com/report?billing"
        );
    }

    /// An endpoint that already carries its service — which is how the intake publishes the
    /// address — is understood rather than pasted into a second `?`.
    #[test]
    fn an_endpoint_that_carries_a_service_names_it_rather_than_doubling_the_query() {
        assert_eq!(
            split_service("https://rockywearsahat.com/report?billing"),
            (
                "https://rockywearsahat.com/report".to_string(),
                Some("billing".to_string())
            )
        );
        assert_eq!(
            split_service(DEFAULT_ENDPOINT),
            (DEFAULT_ENDPOINT.to_string(), None)
        );
        // A query that is not a bare service name is not one.
        assert_eq!(
            split_service("https://example.com/report?a=b").1,
            None,
            "only a bare name is a service"
        );

        assert_eq!(
            endpoint_from(Some("https://rockywearsahat.com/report?billing")).as_deref(),
            Some(DEFAULT_ENDPOINT)
        );
        assert_eq!(
            service_from(Some("https://rockywearsahat.com/report?billing")).as_deref(),
            Some("billing")
        );
        assert_eq!(service_from(Some(DEFAULT_ENDPOINT)), None);
        assert_eq!(service_from(Some("off")), None);
        assert_eq!(service_from(None), None);

        // The stored form makes no difference to where a call goes.
        assert_eq!(
            address(
                "https://rockywearsahat.com/report?billing",
                "feed",
                "billing"
            ),
            address(DEFAULT_ENDPOINT, "feed", "billing")
        );
    }

    #[test]
    fn a_workspace_path_never_leaves_this_machine() {
        assert_eq!(workspace_name("/Users/someone/code/App"), "App");
        assert_eq!(workspace_name("C:\\Users\\A\\Proj"), "Proj");
        assert_eq!(workspace_name("DOC"), "DOC");
    }

    #[test]
    fn reading_a_feed_without_a_token_says_where_to_get_one() {
        let error = feed("https://example.com/report", "dx", "  ").expect_err("refused");
        assert!(error.contains("selfhost reports token"), "{error}");
    }

    #[test]
    fn closing_refuses_an_id_that_is_not_one() {
        let error =
            close("https://example.com/report", "dx", "../../etc", "t").expect_err("refused");
        assert!(error.contains("not a report id"), "{error}");
    }

    #[test]
    fn filed_summary_distinguishes_permanent_refusal_from_transient_failure() {
        let inbox = std::env::temp_dir().join("intake-test-inbox");
        let id = "report-aaaaaaaa".to_string();

        // Permanent refusal: the intake rejected the report
        let refused = Filed {
            id: id.clone(),
            reached: None,
            waiting: 1,
            problem: Some(
                "https://example.com/report refused the report: route is too long".to_string(),
            ),
        };
        let summary = refused.summary("bug", &inbox);
        assert!(summary.contains("refused it"), "{summary}");
        assert!(summary.contains("will not go out"), "{summary}");
        assert!(summary.contains("dx report drop"), "{summary}");
        assert!(!summary.contains("next `dx report sync`"), "{summary}");

        // Transient failure: network unreachable
        let unreached = Filed {
            id,
            reached: None,
            waiting: 1,
            problem: Some("curl could not reach the intake: Connection refused".to_string()),
        };
        let summary = unreached.summary("bug", &inbox);
        assert!(summary.contains("could not be reached"), "{summary}");
        assert!(summary.contains("next `dx report sync`"), "{summary}");
        assert!(!summary.contains("will not go out"), "{summary}");
    }

    /// The push, end to end, against a listener that answers like the intake. This is the one
    /// test that proves `curl` is driven correctly — arguments, stdin body, and the answer read
    /// back — without reaching the network.
    #[test]
    fn a_report_is_pushed_to_a_listening_intake_and_the_id_read_back() {
        use std::io::{BufRead, BufReader, Write as _};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("address");
        let received = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut head = String::new();
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("head");
                if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                head.push_str(&line);
            }
            let mut body = vec![0u8; length];
            std::io::Read::read_exact(&mut reader, &mut body).expect("body");
            let answer = "{\"filed\":\"report-deadbeef\",\"sightings\":1}";
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                        answer.len()
                    )
                    .as_bytes(),
                )
                .expect("answer");
            (head, String::from_utf8_lossy(&body).to_string())
        });

        let report = Report::now(
            Kind::Bug,
            "pushed over a socket",
            "and read back",
            "dx report",
            "",
            Path::new("/tmp/DOC"),
        )
        .expect("report");
        let id = push(&report, &format!("http://{address}"), "dx").expect("push");
        assert_eq!(id, "report-deadbeef");

        let (head, body) = received.join().expect("listener");
        assert!(
            head.starts_with("POST /?dx "),
            "the service is the query the intake publishes: {head}"
        );
        assert!(body.contains("\"project\":\"dx\""), "{body}");
        assert!(
            body.contains("\"title\":\"pushed over a socket\""),
            "{body}"
        );
        assert!(body.contains("\"workspace\":\"DOC\""), "{body}");
        assert!(
            !body.contains("/tmp/"),
            "a path must never leave the machine: {body}"
        );
    }

    #[test]
    fn token_file_honors_the_env_override() {
        let _env = crate::env_lock();
        let root = scratch("token-file-env");
        std::env::set_var(TOKEN_FILE_ENV, root.join("custom-token"));

        let path = token_file();
        assert!(path.to_string_lossy().contains("custom-token"), "{path:?}");

        std::env::remove_var(TOKEN_FILE_ENV);
    }

    #[test]
    fn store_token_and_stored_token_for_round_trip_with_two_line_format() {
        let _env = crate::env_lock();
        let root = scratch("store-token");
        std::env::set_var(TOKEN_FILE_ENV, root.join("token"));

        let endpoint = "https://rockywearsahat.com/report";
        let stored_path = store_token("my-secret-token", endpoint).expect("store");
        assert!(stored_path.exists(), "token file was created");
        let text = std::fs::read_to_string(&stored_path).expect("read");
        assert_eq!(
            text, "my-secret-token\nhttps://rockywearsahat.com/report\n",
            "token stored on first line, endpoint base on second line"
        );

        let retrieved = stored_token_for(endpoint).expect("retrieve");
        assert_eq!(
            retrieved, "my-secret-token",
            "token round-trips for matching endpoint"
        );

        std::env::remove_var(TOKEN_FILE_ENV);
    }

    #[test]
    fn legacy_single_line_token_file_binds_to_default_endpoint() {
        let _env = crate::env_lock();
        let root = scratch("legacy-token");
        std::env::set_var(TOKEN_FILE_ENV, root.join("token"));

        let path = token_file();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        // Write a legacy single-line token file (as the old code would have)
        std::fs::write(&path, "legacy-token\n").expect("write legacy file");

        // Legacy token should bind to DEFAULT_ENDPOINT
        let retrieved = stored_token_for(DEFAULT_ENDPOINT).expect("retrieve from default");
        assert_eq!(
            retrieved, "legacy-token",
            "legacy single-line token reads from default"
        );

        // But not from a different endpoint
        let other = stored_token_for("https://elsewhere.example/report");
        assert!(
            other.is_none(),
            "legacy token should not apply to different endpoint"
        );

        std::env::remove_var(TOKEN_FILE_ENV);
    }

    #[test]
    fn stored_token_for_reads_as_none_when_file_missing_or_blank() {
        let _env = crate::env_lock();
        let root = scratch("stored-token-none");
        std::env::set_var(TOKEN_FILE_ENV, root.join("no-token"));

        assert!(
            stored_token_for(DEFAULT_ENDPOINT).is_none(),
            "missing file reads as None"
        );

        store_token("", DEFAULT_ENDPOINT).expect("store blank");
        assert!(
            stored_token_for(DEFAULT_ENDPOINT).is_none(),
            "blank file reads as None"
        );

        std::env::remove_var(TOKEN_FILE_ENV);
    }

    #[test]
    fn a_machine_token_never_travels_to_an_endpoint_it_was_not_stored_for() {
        let _env = crate::env_lock();
        let root = scratch("token-endpoint-leak");
        std::env::set_var(TOKEN_FILE_ENV, root.join("token"));

        // Store a token for the default endpoint
        store_token("default-endpoint-token", DEFAULT_ENDPOINT).expect("store for default");

        // Build a subscription to a foreign endpoint with no token of its own
        let foreign_endpoint = "https://elsewhere.example/report";
        let subscription = Subscription {
            workspace: root.clone(),
            project: "dx".to_string(),
            endpoint: foreign_endpoint.to_string(),
            token: String::new(),
        };

        // The stored token must not be used for the foreign endpoint
        let token = token_for(&subscription);
        assert!(
            token.is_empty(),
            "machine token for default endpoint must not travel to foreign endpoint"
        );

        std::env::remove_var(TOKEN_FILE_ENV);
    }

    #[test]
    fn token_for_precedence_is_env_over_subscription_over_stored() {
        let _env = crate::env_lock();
        let root = scratch("token-for-precedence");
        std::env::set_var(TOKEN_FILE_ENV, root.join("token"));

        let endpoint = "https://example.com/report";
        let subscription = Subscription {
            workspace: root.clone(),
            project: "dx".to_string(),
            endpoint: endpoint.to_string(),
            token: "subscription-token".to_string(),
        };

        assert_eq!(
            token_for(&subscription),
            "subscription-token",
            "subscription token when no env or stored"
        );

        store_token("stored-token", endpoint).expect("store");
        let empty_sub = Subscription {
            workspace: root.clone(),
            project: "dx".to_string(),
            endpoint: endpoint.to_string(),
            token: String::new(),
        };
        assert_eq!(
            token_for(&empty_sub),
            "stored-token",
            "stored token when subscription empty"
        );

        std::env::set_var(TOKEN_ENV, "env-token");
        assert_eq!(
            token_for(&subscription),
            "env-token",
            "env token takes precedence"
        );

        std::env::remove_var(TOKEN_ENV);
        std::env::remove_var(TOKEN_FILE_ENV);
    }
}
