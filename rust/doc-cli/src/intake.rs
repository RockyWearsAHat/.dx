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
//!  file  ──▶ this machine's inbox ──▶ POST <endpoint>        (immediately, best effort)
//!                                        │
//!  sync  ◀── reports.dx, folded ◀── GET <endpoint>/feed      (every subscribed checkout)
//!  close ──▶ block removed      ──▶ POST <endpoint>/close    (so a fix does not come back)
//! ```
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

/// Where reports go when nothing says otherwise.
///
/// A default rather than a setting, because the whole point is that an agent that has never
/// heard of this machinery still reaches the people who fix dx. `DX_REPORT_ENDPOINT` overrides
/// it, and `DX_REPORT_ENDPOINT=off` turns the push off entirely.
pub const DEFAULT_ENDPOINT: &str = "https://rockywearsahat.com/api/reports";

/// The project reports are about when nothing says otherwise.
pub const DEFAULT_PROJECT: &str = "dx";

/// Environment override for the endpoint. `off` disables the push.
const ENDPOINT_ENV: &str = "DX_REPORT_ENDPOINT";
/// Environment override for the feed token, for a machine that would rather not store one.
const TOKEN_ENV: &str = "DX_REPORT_TOKEN";
/// Environment override for the subscriptions file, so the suite never touches the real one.
const SUBSCRIPTIONS_ENV: &str = "DX_SUBSCRIPTIONS_FILE";

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

/// The endpoint this machine files to, or `None` when the push is turned off.
#[must_use]
pub fn endpoint() -> Option<String> {
    endpoint_from(std::env::var(ENDPOINT_ENV).ok().as_deref())
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
        Some(value) if !value.trim().is_empty() => {
            Some(value.trim().trim_end_matches('/').to_string())
        }
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
            endpoint,
        ],
        Some(body.as_bytes()),
    )?;

    let value: Value = serde_json::from_str(&answered)
        .map_err(|_| format!("{endpoint} answered something that is not JSON: {answered}"))?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(format!("{endpoint} refused the report: {error}"));
    }
    value
        .get("filed")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("{endpoint} did not say what it filed: {answered}"))
}

/// Every open report the intake holds for `project`.
///
/// # Errors
/// Returns a sentence when the feed cannot be read — including the token being wrong, which is
/// the one failure that otherwise reads as "no reports".
pub fn feed(endpoint: &str, project: &str, token: &str) -> Result<Vec<Value>, String> {
    if token.trim().is_empty() {
        return Err(format!(
            "reading {endpoint}/feed needs the owner's token — run `selfhost reports token` on \
             the box and pass it to `dx report subscribe --token`"
        ));
    }
    let config = format!(
        "url = \"{endpoint}/feed?project={project}\"\nheader = \"authorization: Bearer {token}\"\n"
    );
    let answered = run_curl(
        &["-sS", "--max-time", &TIMEOUT_SECONDS.to_string(), "-K", "-"],
        Some(config.as_bytes()),
    )?;
    let value: Value = serde_json::from_str(&answered)
        .map_err(|_| format!("{endpoint}/feed answered something that is not JSON: {answered}"))?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(format!("{endpoint}/feed refused: {error}"));
    }
    Ok(value
        .get("reports")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// Tells the intake that `id` is fixed, so no later sync folds it back in.
///
/// # Errors
/// Returns a sentence when the intake refuses or cannot be reached.
pub fn close(endpoint: &str, project: &str, id: &str, token: &str) -> Result<(), String> {
    if !id.starts_with("report-") || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(format!("`{id}` is not a report id"));
    }
    if token.trim().is_empty() {
        return Err(format!(
            "closing a report on {endpoint} needs the owner's token"
        ));
    }
    let config = format!(
        "url = \"{endpoint}/close\"\nheader = \"authorization: Bearer {token}\"\n\
         header = \"content-type: application/json\"\ndata = \"{{\\\"project\\\":\\\"{project}\\\",\\\"id\\\":\\\"{id}\\\"}}\"\n"
    );
    let answered = run_curl(
        &["-sS", "--max-time", &TIMEOUT_SECONDS.to_string(), "-K", "-"],
        Some(config.as_bytes()),
    )?;
    let value: Value = serde_json::from_str(&answered)
        .map_err(|_| format!("{endpoint}/close answered something that is not JSON: {answered}"))?;
    match value.get("error").and_then(Value::as_str) {
        Some(error) => Err(format!("{endpoint}/close refused: {error}")),
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
            (Some(endpoint), _) => format!(
                "filed {} ({kind}) — it is in the database at {endpoint} and will be in \
                 {} the next time this checkout syncs\n",
                self.id,
                reports::DOCUMENT
            ),
            (None, Some(problem)) => format!(
                "filed {} ({kind}) in {} — {} waiting, because the intake could not be \
                 reached: {problem}\nthey go out on the next `dx report sync`\n",
                self.id,
                inbox.display(),
                self.waiting
            ),
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
    match push(report, &endpoint, DEFAULT_PROJECT) {
        Ok(filed) => {
            let _ = std::fs::remove_file(&record);
            Ok(Filed {
                id: filed,
                reached: Some(endpoint),
                waiting: reports::read_inbox(&inbox)?.pending.len(),
                problem: None,
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
        if self.added.is_empty() && self.updated.is_empty() {
            lines.push(format!("{} is up to date", document.display()));
        } else {
            lines.push(format!(
                "{}: {} new, {} updated",
                document.display(),
                self.added.len(),
                self.updated.len()
            ));
            for id in self.added.iter().chain(&self.updated) {
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
        !self.added.is_empty() || !self.updated.is_empty()
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
        &subscription.token,
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
    /// Records that could not be folded, each a sentence.
    pub problems: Vec<String>,
}

/// Writes every report in `entries` into the document at `path`, one block each.
///
/// A block whose body already matches is left alone, so this is safe to run on a timer: a fold
/// with nothing to say touches no file and produces no diff.
///
/// # Errors
/// Returns a sentence when the document cannot be read or saved.
pub fn fold(entries: &[Value], path: &Path) -> Result<Folded, String> {
    let mut folded = Folded::default();
    if entries.is_empty() {
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

/// The token for `subscription`, letting the environment override the stored one.
#[must_use]
pub fn token_for(subscription: &Subscription) -> String {
    std::env::var(TOKEN_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| subscription.token.clone())
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
        let root = scratch("subscribe");
        std::env::set_var(SUBSCRIPTIONS_ENV, root.join("subscriptions.json"));

        let subscription = Subscription {
            workspace: root.clone(),
            project: "dx".to_string(),
            endpoint: "https://example.com/api/reports".to_string(),
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

    #[test]
    fn the_endpoint_can_be_pointed_elsewhere_or_turned_off() {
        assert_eq!(
            endpoint_from(Some("https://elsewhere.example/api/reports/")).as_deref(),
            Some("https://elsewhere.example/api/reports")
        );
        assert!(endpoint_from(Some("off")).is_none());
        assert!(endpoint_from(Some("OFF")).is_none());
        assert_eq!(endpoint_from(None).as_deref(), Some(DEFAULT_ENDPOINT));
        assert_eq!(endpoint_from(Some("  ")).as_deref(), Some(DEFAULT_ENDPOINT));
    }

    #[test]
    fn a_workspace_path_never_leaves_this_machine() {
        assert_eq!(workspace_name("/Users/someone/code/App"), "App");
        assert_eq!(workspace_name("C:\\Users\\A\\Proj"), "Proj");
        assert_eq!(workspace_name("DOC"), "DOC");
    }

    #[test]
    fn reading_a_feed_without_a_token_says_where_to_get_one() {
        let error = feed("https://example.com/api/reports", "dx", "  ").expect_err("refused");
        assert!(error.contains("selfhost reports token"), "{error}");
    }

    #[test]
    fn closing_refuses_an_id_that_is_not_one() {
        let error =
            close("https://example.com/api/reports", "dx", "../../etc", "t").expect_err("refused");
        assert!(error.contains("not a report id"), "{error}");
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
            String::from_utf8_lossy(&body).to_string()
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

        let body = received.join().expect("listener");
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
}
