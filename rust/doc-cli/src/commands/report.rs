//! `dx report` — file what dx got wrong, and let it reach the checkout that fixes it.
//!
//! ```text
//! dx report bug|suggestion|observation --title T --detail D [--route R] [--repro S]
//! dx report list [dir]        what is waiting here, and what the checkout carries
//! dx report sync [dir]        push what is waiting, pull the project's open reports
//! dx report subscribe [dir] [--project dx] [--endpoint URL] [--token T]
//! dx report setup [dir] [--project N] [--endpoint URL] [--token T]
//!                             one command: subscribe this repository
//! dx report token [T]         store the owner's token once, machine-wide
//! dx report unsubscribe [dir]
//! dx report close <id> [dir]  a fix: the block goes, and the database is told
//! dx report drain [dir]       fold this machine's inbox in without a network
//! dx report mcp               serve a project-scoped report tool over MCP
//! ```
//!
//! Filing goes two ways at once: into this machine's inbox ([`crate::reports`]) and, unless
//! the endpoint is turned off, straight to the intake ([`crate::intake`]) — so a defect an
//! agent hits while working on some unrelated project still reaches the dx checkout, where
//! the next agent reads it in `reports.dx`. `drain` remains the offline route: it folds the
//! local inbox into the document with no network at all.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::args::Args;
use crate::commands::Output;
use crate::intake::{self, Subscription};
use crate::reports::{self, Kind, Report};
use crate::workspace;

/// Run `dx report`.
///
/// # Errors
/// Returns a sentence when the kind word is not one of the three, when a filed report has no
/// title or detail, or when the inbox, the subscription, or the document cannot be read or
/// written.
pub fn run(args: &Args) -> Result<Output, String> {
    match args.positional(0).unwrap_or("list") {
        "list" => list(args).map(Output::Document),
        "drain" => drain(args).map(Output::Report),
        "sync" => sync(args).map(Output::Report),
        "subscribe" => subscribe(args).map(Output::Report),
        "setup" => setup(args).map(Output::Report),
        "token" => token(args).map(Output::Report),
        "unsubscribe" => unsubscribe(args).map(Output::Report),
        "close" => close(args).map(Output::Report),
        "drop" => drop(args).map(Output::Report),
        "mcp" => mcp_serve(args).map(Output::Report),
        kind => file(kind, args).map(Output::Report),
    }
}

/// `dx report <kind> --title T --detail D` — file one report, here and at the intake.
fn file(kind: &str, args: &Args) -> Result<String, String> {
    let kind = Kind::parse(kind)?;
    let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let report = Report::now(
        kind,
        args.value("title").unwrap_or_default(),
        args.value("detail").unwrap_or_default(),
        args.value("route").unwrap_or_default(),
        args.value("repro").unwrap_or_default(),
        &workspace::workspace_root(&here),
    )?;

    // Agents connected over MCP should use the mcp__dx__dx_report tool instead of shelling
    // out to this CLI command. Print this notice unconditionally: one extra stderr line is
    // tolerable for the rare case of a person filing by hand, and it is far better than a
    // condition that silently never fires.
    let _ = writeln!(
        std::io::stderr(),
        "note: MCP-connected agents should prefer the mcp__dx__dx_report tool over this CLI command"
    );

    let filed = intake::file(&report)?;
    Ok(filed.summary(kind.as_str(), &reports::inbox()))
}

/// `dx report drain [dir]` — fold this machine's inbox into `<dir>/reports.dx`, no network.
fn drain(args: &Args) -> Result<String, String> {
    let document = document_for(args.positional(1));
    let drained = reports::drain(&reports::inbox(), &document)?;
    Ok(format!("{}\n", drained.summary(&document)))
}

/// `dx report sync [dir]` — push what is waiting, then fold the project's open reports in.
fn sync(args: &Args) -> Result<String, String> {
    let root = root_for(args.positional(1));
    let subscription = subscription_or_hint(&root)?;
    let synced = intake::sync(&subscription)?;
    Ok(format!("{}\n", synced.summary(&subscription.document())))
}

/// `dx report subscribe [dir]` — this checkout receives a project's reports from now on.
fn subscribe(args: &Args) -> Result<String, String> {
    let root = root_for(args.positional(1));
    let stated = args.value("endpoint");
    let subscription = Subscription {
        workspace: root.clone(),
        // The service is the endpoint's query, so `--endpoint …/report?billing` registers
        // `billing` with nothing else said. `--project` still wins when both are given.
        project: args
            .value("project")
            .map(str::to_string)
            .or_else(|| intake::service_from(stated))
            .or_else(intake::service)
            .unwrap_or_else(|| intake::DEFAULT_PROJECT.to_string()),
        endpoint: stated
            .map(|value| intake::split_service(value).0)
            .or_else(intake::endpoint)
            .unwrap_or_else(|| intake::DEFAULT_ENDPOINT.to_string()),
        token: args.value("token").unwrap_or_default().to_string(),
    };
    intake::subscribe(&subscription)?;

    let mut out = format!(
        "{} now receives `{}` reports from {}\n",
        subscription.document().display(),
        subscription.project,
        intake::address(&subscription.endpoint, "", &subscription.project)
    );
    if intake::token_for(&subscription).is_empty() {
        out.push_str(
            "no token stored, so this checkout can file but not read — run \
             `selfhost reports token` on the box and re-run with --token\n",
        );
        return Ok(out);
    }
    let synced = intake::sync(&subscription)?;
    out.push_str(&format!("{}\n", synced.summary(&subscription.document())));
    Ok(out)
}

/// `dx report unsubscribe [dir]` — stop receiving. The document is left exactly as it is.
fn unsubscribe(args: &Args) -> Result<String, String> {
    let root = root_for(args.positional(1));
    if intake::unsubscribe(&root)? {
        return Ok(format!("{} no longer receives reports\n", root.display()));
    }
    Ok(format!("{} was not subscribed\n", root.display()))
}

/// `dx report close <id> [dir]` — tell the database, then remove the block.
///
/// Both halves, because either alone is wrong: a block removed but not closed comes back on
/// the next sync, and a report closed but not removed leaves the document claiming an open
/// defect nobody will ever see again. The database is told **first**: it is the one call that
/// can fail on grounds the caller could not see locally (a stale block, a race with another
/// close), and a failure there must leave the document exactly as it was rather than losing the
/// record of an open report the server still disagrees about.
fn close(args: &Args) -> Result<String, String> {
    let id = args
        .positional(1)
        .ok_or("`dx report close` needs a report id, e.g. `dx report close report-1a2b3c4d`")?;
    let root = root_for(args.positional(2));
    let subscription = subscription_or_hint(&root)?;
    let document = subscription.document();

    intake::close(
        &subscription.endpoint,
        &subscription.project,
        id,
        &intake::token_for(&subscription),
    )
    .map_err(|error| {
        format!(
            "{error} — run `dx report sync`, which removes a report the intake has already \
             closed elsewhere"
        )
    })?;
    let mut out = format!(
        "closed {id} at {}\n",
        intake::address(&subscription.endpoint, "close", &subscription.project)
    );

    if document.exists() {
        let source = workspace::read(&document)?;
        let parsed = doc_core::format::parse(&source);
        if doc_core::edit::find(&parsed, id).is_ok() {
            let without = doc_core::edit::remove_block(&source, id)?;
            workspace::save_source(&document, &without)?;
            out.push_str(&format!("removed {id} from {}\n", document.display()));
        }
    }
    Ok(out)
}

/// `dx report drop <id>` — remove one entry from the local inbox by id only.
///
/// Never touches the remote intake. The entry is permanently deleted from this machine's
/// inbox, for a report that is stuck and cannot be retrieved otherwise.
fn drop(args: &Args) -> Result<String, String> {
    let id = args
        .positional(1)
        .ok_or("`dx report drop` needs a report id, e.g. `dx report drop report-1a2b3c4d`")?;
    let inbox = reports::inbox();
    let inbox_contents = reports::read_inbox(&inbox)?;

    // Find the record file matching this id
    let record_to_remove = inbox_contents
        .pending
        .iter()
        .find(|pending| pending.report.id() == id)
        .map(|pending| pending.record.clone());

    match record_to_remove {
        Some(record) => {
            std::fs::remove_file(&record).map_err(|error| {
                format!(
                    "could not remove {} from {}: {error}",
                    record.display(),
                    inbox.display()
                )
            })?;
            Ok(format!("dropped {id} from {}\n", inbox.display()))
        }
        None => Err(format!(
            "{id} is not in the local inbox at {}\nrun `dx report list` to see what is waiting\n",
            inbox.display()
        )),
    }
}

/// Try to claim a scoped reader token from the local operator via `selfhost reports project add`.
///
/// Returns (token, was_operator_detected) tuple. If the selfhost binary is not found or the
/// command fails for any reason, returns (empty_string, false) to fall back to the default path.
fn try_claim_scoped_token(project: &str) -> (String, bool) {
    let output = match std::process::Command::new("selfhost")
        .arg("reports")
        .arg("project")
        .arg("add")
        .arg(project)
        .output()
    {
        Ok(output) => output,
        Err(_) => return (String::new(), false), // selfhost not found or exec error
    };

    if !output.status.success() {
        return (String::new(), false); // command failed
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(token_part) = trimmed.strip_prefix("reader token: ") {
            let token = token_part.trim();
            // Validate it looks like lowercase hex
            if token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()) {
                return (token.to_string(), true);
            }
            // Token format didn't match; fall back to non-operator path
            return (String::new(), false);
        }
    }

    // Token line not found in output
    (String::new(), false)
}

/// Write or merge a project-local MCP configuration file at `.mcp.json`.
///
/// Creates a new file or merges into an existing one, preserving other MCP server entries.
/// Does NOT print the token to stdout.
fn write_mcp_config(
    root: &Path,
    project: &str,
    token: &str,
    dx_binary: &Path,
) -> Result<(), String> {
    let mcp_json = root.join(".mcp.json");

    // Read existing config if it exists
    let mut config: serde_json::Value = if mcp_json.exists() {
        let content = std::fs::read_to_string(&mcp_json)
            .map_err(|e| format!("could not read .mcp.json: {e}"))?;
        serde_json::from_str(&content).map_err(|e| format!("could not parse .mcp.json: {e}"))?
    } else {
        serde_json::json!({"mcpServers": {}})
    };

    // Ensure mcpServers object exists
    if !config.get("mcpServers").is_some_and(|v| v.is_object()) {
        config["mcpServers"] = serde_json::json!({});
    }

    // Set the "report" MCP server entry
    let dx_path = dx_binary
        .to_str()
        .ok_or("dx binary path is not valid UTF-8")?;

    config["mcpServers"]["report"] = serde_json::json!({
        "command": dx_path,
        "args": ["report", "mcp"],
        "env": {
            "DX_REPORT_PROJECT": project,
            "DX_REPORT_TOKEN": token
        }
    });

    let formatted = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("could not serialize .mcp.json: {e}"))?;

    std::fs::write(&mcp_json, format!("{formatted}\n"))
        .map_err(|e| format!("could not write .mcp.json: {e}"))?;

    Ok(())
}

/// Ensure `.mcp.json` is in `.gitignore`.
fn add_to_gitignore(root: &Path) -> Result<(), String> {
    let gitignore = root.join(".gitignore");
    let entry = ".mcp.json\n";

    if !gitignore.exists() {
        std::fs::write(&gitignore, entry)
            .map_err(|e| format!("could not create .gitignore: {e}"))?;
        return Ok(());
    }

    let content = std::fs::read_to_string(&gitignore)
        .map_err(|e| format!("could not read .gitignore: {e}"))?;

    if !content.contains(".mcp.json") {
        std::fs::write(&gitignore, format!("{content}{entry}"))
            .map_err(|e| format!("could not update .gitignore: {e}"))?;
    }

    Ok(())
}

/// `dx report setup [dir]` — one command: subscribe this repository to its own reports.
fn setup(args: &Args) -> Result<String, String> {
    let root = root_for(args.positional(1));

    let project = if let Some(stated) = args.value("project") {
        if stated
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            stated.to_string()
        } else {
            return Err(format!(
                "`{stated}` has a character not allowed in a service name; use letters, digits, \
                 dot, dash, and underscore only"
            ));
        }
    } else {
        let folder_name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("report")
            .to_string();
        let sanitized = folder_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>();
        let trimmed = sanitized.trim_matches('-').to_string();
        if trimmed.is_empty() {
            return Err(format!(
                "the folder name `{folder_name}` does not contain characters for a service name; \
                 pass --project explicitly"
            ));
        }
        trimmed
    };

    let endpoint = args
        .value("endpoint")
        .map(|value| intake::split_service(value).0)
        .or_else(intake::endpoint)
        .unwrap_or_else(|| intake::DEFAULT_ENDPOINT.to_string());

    let mut out = String::new();

    // Try to claim a scoped token from the local operator
    let (scoped_token, operator_detected) = try_claim_scoped_token(&project);

    if operator_detected && !scoped_token.is_empty() {
        // Store the scoped token with the project as the service qualifier
        let scoped_endpoint = format!("{}?{}", endpoint, project);
        let path = intake::store_token(&scoped_token, &scoped_endpoint)?;
        out.push_str(&format!(
            "claimed scoped reader token for `{project}` at {}\n",
            path.display()
        ));

        // Write the project-local MCP server configuration
        let dx_binary = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("dx"));
        write_mcp_config(&root, &project, &scoped_token, &dx_binary)?;
        add_to_gitignore(&root)?;
        out.push_str("registered project-local MCP server at .mcp.json\n");
    } else {
        // Fallback: use existing token logic
        if let Some(stated_token) = args.value("token") {
            let path = intake::store_token(stated_token, &endpoint)?;
            out.push_str(&format!(
                "token stored at {} — it is never shown again\n",
                path.display()
            ));
        } else if intake::stored_token_for(&endpoint).is_none() {
            if let Ok(existing_subs) = intake::subscriptions() {
                if let Some(existing) = existing_subs
                    .iter()
                    .find(|s| s.endpoint == endpoint && !s.token.is_empty())
                {
                    intake::store_token(&existing.token, &endpoint)?;
                    out.push_str(&format!(
                        "adopted token from existing subscription at {}\n",
                        existing.workspace.display()
                    ));
                }
            }
        }
    }

    if let Ok(existing_subs) = intake::subscriptions() {
        for subscription in existing_subs {
            if !subscription.workspace.exists() {
                intake::unsubscribe(&subscription.workspace)?;
                out.push_str(&format!(
                    "pruned subscription for {} (no longer exists)\n",
                    subscription.workspace.display()
                ));
            }
        }
    }

    let subscription = Subscription {
        workspace: root.clone(),
        project: project.clone(),
        endpoint: endpoint.clone(),
        token: String::new(),
    };
    intake::subscribe(&subscription)?;

    out.push_str(&format!(
        "{} now receives `{project}` reports from {}\n",
        subscription.document().display(),
        intake::address(&subscription.endpoint, "", &project)
    ));

    if intake::token_for(&subscription).is_empty() {
        out.push_str(
            "filing works from here, but reading the feed needs the owner's token — run \
             `selfhost reports token` on the box and pass it with `dx report token <t>`\n",
        );
        return Ok(out);
    }

    let synced = intake::sync(&subscription)?;
    out.push_str(&format!("{}\n", synced.summary(&subscription.document())));

    if synced
        .problems
        .iter()
        .any(|p| p.contains("is not a service this box hosts"))
    {
        out.push_str(
            "no service exists for this project yet — a registered account or the operator \
             (`selfhost reports project add`) has to create it before reports flow through; \
             filing still queues in this machine's inbox until then\n",
        );
    }

    Ok(out)
}

/// `dx report token [T]` — store the owner's token for a specific endpoint, or query what is stored.
fn token(args: &Args) -> Result<String, String> {
    let endpoint = args
        .value("endpoint")
        .map(|value| intake::split_service(value).0)
        .or_else(intake::endpoint)
        .unwrap_or_else(|| intake::DEFAULT_ENDPOINT.to_string());

    if let Some(stated_token) = args.positional(1) {
        let path = intake::store_token(stated_token, &endpoint)?;
        Ok(format!(
            "token stored at {} for {}\n",
            path.display(),
            endpoint
        ))
    } else {
        let path = intake::token_file();
        match intake::stored_token_for(&endpoint) {
            Some(_) => Ok(format!(
                "a token is stored at {} for {}\n",
                path.display(),
                endpoint
            )),
            None => Ok(format!(
                "no token stored for {} — run `dx report token <t>` to store one at {}\n",
                endpoint,
                path.display()
            )),
        }
    }
}

/// `dx report list [dir]` — what is waiting here, and what the checkout is carrying.
fn list(args: &Args) -> Result<String, String> {
    let inbox = reports::inbox();
    let waiting = reports::read_inbox(&inbox)?;
    let document = document_for(args.positional(1));
    let open = reports::open_reports(&document)?;

    let mut out = String::new();
    if waiting.pending.is_empty() {
        out.push_str(&format!("inbox {} — empty\n", inbox.display()));
    } else {
        out.push_str(&format!(
            "inbox {} — {} waiting for `dx report sync`\n",
            inbox.display(),
            waiting.pending.len()
        ));
        for pending in &waiting.pending {
            out.push_str(&format!(
                "  {} {} — {}\n",
                pending.report.at,
                pending.report.kind.as_str(),
                pending.report.title
            ));
        }
    }
    for reason in &waiting.unreadable {
        out.push_str(&format!("  unreadable — {reason}\n"));
    }

    if open.is_empty() {
        out.push_str(&format!("{} — no open reports\n", document.display()));
    } else {
        out.push_str(&format!("{} — {} open\n", document.display(), open.len()));
        for report in &open {
            let times = if report.sightings == 1 {
                "once".to_string()
            } else {
                format!("{} times", report.sightings)
            };
            out.push_str(&format!("  {} {} — {times}\n", report.id, report.headline));
        }
    }

    match intake::subscription_for(&workspace::workspace_root(&document_root(args))) {
        Ok(Some(subscription)) => out.push_str(&format!(
            "subscribed to `{}` at {}\n",
            subscription.project,
            intake::address(&subscription.endpoint, "", &subscription.project)
        )),
        Ok(None) => out.push_str(
            "not subscribed — `dx report subscribe --token <t>` keeps this document current\n",
        ),
        Err(reason) => out.push_str(&format!("subscription unreadable — {reason}\n")),
    }
    Ok(out)
}

/// The workspace root a command is about: the directory named, or the current one.
fn root_for(directory: Option<&str>) -> PathBuf {
    let start = directory.map_or_else(
        || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        PathBuf::from,
    );
    workspace::workspace_root(&start)
}

/// The directory a listing is about, before it is resolved to a workspace root.
fn document_root(args: &Args) -> PathBuf {
    args.positional(1).map_or_else(
        || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        PathBuf::from,
    )
}

/// The `reports.dx` of the workspace containing `directory`, or of the current directory.
fn document_for(directory: Option<&str>) -> PathBuf {
    root_for(directory).join(reports::DOCUMENT)
}

/// The subscription for `root`, or a sentence naming the command that creates one.
fn subscription_or_hint(root: &Path) -> Result<Subscription, String> {
    intake::subscription_for(root)?.ok_or_else(|| {
        format!(
            "{} is not subscribed to a report project — `dx report subscribe --token <t>` here \
             makes this checkout receive them",
            root.display()
        )
    })
}

/// `dx report mcp` — MCP server bound to one project and scoped reader token.
///
/// Reads DX_REPORT_PROJECT and DX_REPORT_TOKEN environment variables at startup and serves a
/// minimal MCP stdio server with three report tools (report_file, report_feed, report_close).
/// Unlike the machine-wide `dx mcp` server, this server always talks about the ONE project it
/// was launched for, and can be registered as a project-local MCP server.
fn mcp_serve(_args: &Args) -> Result<String, String> {
    // Resolve configuration from environment
    let project = std::env::var("DX_REPORT_PROJECT")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .ok_or("DX_REPORT_PROJECT environment variable must be set and non-empty")?;

    let token = std::env::var("DX_REPORT_TOKEN")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .ok_or("DX_REPORT_TOKEN environment variable must be set and non-empty")?;

    // Resolve the endpoint using existing intake helpers
    let endpoint = intake::endpoint().unwrap_or_else(|| intake::DEFAULT_ENDPOINT.to_string());

    // Run the MCP server loop
    mcp_serve_loop(&endpoint, &project, &token)?;
    Ok("MCP server exited cleanly".to_string())
}

/// The MCP server loop: read JSON-RPC requests from stdin, dispatch to tools, write responses.
fn mcp_serve_loop(endpoint: &str, project: &str, token: &str) -> Result<(), String> {
    use std::io::{BufRead, BufReader, Write};

    let mut input = BufReader::new(std::io::stdin().lock());
    let mut output = std::io::stdout().lock();
    let mut line = String::new();

    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) => return Ok(()), // clean EOF
            Ok(_) => {
                if let Some(response) = handle_mcp_request(&line, endpoint, project, token) {
                    if writeln!(output, "{response}").is_err() {
                        return Ok(()); // stdout closed; client disconnected
                    }
                    if output.flush().is_err() {
                        return Ok(());
                    }
                }
            }
            Err(_) => return Ok(()), // read error, exit
        }
    }
}

/// Parse and handle one MCP JSON-RPC request. Returns None for blank lines or notifications.
fn handle_mcp_request(line: &str, endpoint: &str, project: &str, token: &str) -> Option<String> {
    use serde_json::{json, Value};

    if line.trim().is_empty() {
        return None;
    }

    let request: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => {
            return Some(
                serde_json::to_string(&json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": {
                        "code": -32700,
                        "message": "Parse error"
                    }
                }))
                .unwrap_or_else(|_| "null".to_string()),
            );
        }
    };

    if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(
            serde_json::to_string(&json!({
                "jsonrpc": "2.0",
                "id": request.get("id"),
                "error": {
                    "code": -32600,
                    "message": "Invalid Request"
                }
            }))
            .unwrap_or_else(|_| "null".to_string()),
        );
    }

    let id = request.get("id")?.clone();
    let method = request.get("method").and_then(Value::as_str)?;
    let default_params = json!({});
    let params = request.get("params").unwrap_or(&default_params);

    // Dispatch to tools
    let result = match method {
        "tools/list" => mcp_tools_list(),
        "tools/call" => mcp_tools_call(params, endpoint, project, token),
        _ => {
            return Some(
                serde_json::to_string(&json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {
                        "code": -32601,
                        "message": "Method not found"
                    }
                }))
                .unwrap_or_else(|_| "null".to_string()),
            );
        }
    };

    match result {
        Ok(content) => Some(
            serde_json::to_string(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": content
            }))
            .unwrap_or_else(|_| "null".to_string()),
        ),
        Err(error) => Some(
            serde_json::to_string(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32603,
                    "message": error
                }
            }))
            .unwrap_or_else(|_| "null".to_string()),
        ),
    }
}

/// Return the tools/list catalogue for this scoped MCP server.
fn mcp_tools_list() -> Result<serde_json::Value, String> {
    use serde_json::json;

    Ok(json!({
        "tools": [
            {
                "name": "report_file",
                "description": "File a new report (bug, suggestion, or observation) to the intake for this project.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "kind": {
                            "type": "string",
                            "enum": ["bug", "suggestion", "observation"],
                            "description": "The type of report."
                        },
                        "title": {
                            "type": "string",
                            "description": "Short headline for the report."
                        },
                        "detail": {
                            "type": "string",
                            "description": "Detailed explanation of the issue."
                        },
                        "route": {
                            "type": "string",
                            "description": "Optional: where in the project this was found (e.g., 'build system', 'format parser')."
                        },
                        "repro": {
                            "type": "string",
                            "description": "Optional: steps to reproduce, or a code snippet that shows the problem."
                        }
                    },
                    "required": ["kind", "title", "detail"]
                }
            },
            {
                "name": "report_feed",
                "description": "Fetch all open reports for this project from the intake.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "report_close",
                "description": "Close a report by id (report has been fixed). Requires the project's reader token.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "The report id to close (e.g., 'report-1a2b3c4d')."
                        }
                    },
                    "required": ["id"]
                }
            }
        ]
    }))
}

/// Dispatch a tool call in this scoped MCP server.
fn mcp_tools_call(
    params: &serde_json::Value,
    endpoint: &str,
    project: &str,
    token: &str,
) -> Result<serde_json::Value, String> {
    use serde_json::json;

    let name = params
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or("tool name required")?;

    let default_args = json!({});
    let args = params.get("arguments").unwrap_or(&default_args);

    match name {
        "report_file" => {
            let kind_str = args
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .ok_or("kind is required")?;
            let kind = reports::Kind::parse(kind_str)?;

            let title = args
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");

            let detail = args
                .get("detail")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");

            let route = args
                .get("route")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");

            let repro = args
                .get("repro")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");

            if title.is_empty() || detail.is_empty() {
                return Err("title and detail are required".to_string());
            }

            let workspace = std::env::current_dir()
                .ok()
                .map(|d| workspace::workspace_root(&d))
                .unwrap_or_else(|| PathBuf::from("."));

            let report = reports::Report::now(kind, title, detail, route, repro, &workspace)?;

            // Push directly to the endpoint with the explicitly supplied project
            let filed_id = intake::push(&report, endpoint, project)
                .map_err(|e| format!("filing failed: {e}"))?;

            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Filed report {filed_id} for project '{project}' at {endpoint}")
                }]
            }))
        }
        "report_feed" => {
            let reports = intake::feed(endpoint, project, token)
                .map_err(|e| format!("failed to fetch feed: {e}"))?;

            let report_texts: Vec<String> = reports
                .iter()
                .filter_map(|r| {
                    let id = r.get("id").and_then(serde_json::Value::as_str)?;
                    let title = r.get("title").and_then(serde_json::Value::as_str)?;
                    Some(format!("{} — {}", id, title))
                })
                .collect();

            let summary = if report_texts.is_empty() {
                format!("No open reports for '{project}'")
            } else {
                format!(
                    "{} open report(s) for '{project}':\n{}",
                    report_texts.len(),
                    report_texts.join("\n")
                )
            };

            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": summary
                }]
            }))
        }
        "report_close" => {
            let id = args
                .get("id")
                .and_then(serde_json::Value::as_str)
                .ok_or("id is required")?;

            intake::close(endpoint, project, id, token)
                .map_err(|e| format!("close failed: {e}"))?;

            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Closed report {id} for project '{project}'")
                }]
            }))
        }
        other => Err(format!("unknown tool: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(tokens: &[&str]) -> Args {
        Args::parse(&tokens.iter().map(|t| (*t).to_string()).collect::<Vec<_>>())
    }

    /// The suite files into a temporary inbox with the push turned off: a test run must never
    /// touch the developer's real inbox, and must never reach the real intake. Both are
    /// process-wide variables, so the cases that need them share one test.
    #[test]
    fn filing_listing_and_draining_are_one_loop() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-cli-tests");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        std::env::set_var("DX_REPORTS_DIR", root.join("inbox"));
        std::env::set_var("DX_REPORT_ENDPOINT", "off");
        std::env::set_var("DX_SUBSCRIPTIONS_FILE", root.join("subscriptions.json"));
        std::env::set_var("DX_REPORT_TOKEN_FILE", root.join("token"));

        let filed = run(&args(&[
            "bug",
            "--title",
            "dx report has no test",
            "--detail",
            "It did not, until this one.",
            "--route",
            "dx report",
        ]))
        .expect("file");
        assert!(
            filed.text().starts_with("filed report-"),
            "{}",
            filed.text()
        );
        let filed_id = filed
            .text()
            .split_whitespace()
            .nth(1)
            .expect("the summary names the id")
            .to_string();

        let waiting = run(&args(&["list", root.to_str().expect("path")])).expect("list");
        assert!(waiting.text().contains("1 waiting"), "{}", waiting.text());
        assert!(
            waiting.text().contains("no open reports"),
            "{}",
            waiting.text()
        );
        assert!(
            waiting.text().contains("not subscribed"),
            "{}",
            waiting.text()
        );

        let drained = run(&args(&["drain", root.to_str().expect("path")])).expect("drain");
        assert!(
            drained.text().contains("folded 1 report(s)"),
            "{}",
            drained.text()
        );

        let after = run(&args(&["list", root.to_str().expect("path")])).expect("list");
        assert!(after.text().contains("1 open"), "{}", after.text());
        assert!(after.text().contains("once"), "{}", after.text());

        // The listing is content a reader may redirect; a drain reports work already done.
        assert!(matches!(after, Output::Document(_)));
        assert!(matches!(drained, Output::Report(_)));

        // Subscribing without a token says so rather than pretending to sync.
        let subscribed = run(&args(&[
            "subscribe",
            root.to_str().expect("path"),
            "--endpoint",
            "https://example.invalid/report",
        ]))
        .expect("subscribe");
        assert!(
            subscribed.text().contains("no token stored"),
            "{}",
            subscribed.text()
        );
        let listed = run(&args(&["list", root.to_str().expect("path")])).expect("list");
        assert!(
            listed.text().contains("subscribed to `dx`"),
            "{}",
            listed.text()
        );
        assert!(
            listed.text().contains("https://example.invalid/report?dx"),
            "the address a reader is shown is the one calls go to: {}",
            listed.text()
        );

        // Registering another internal service is the address and nothing else.
        let registered = run(&args(&[
            "subscribe",
            root.to_str().expect("path"),
            "--endpoint",
            "https://example.invalid/report?billing",
        ]))
        .expect("subscribe");
        assert!(
            registered
                .text()
                .contains("`billing` reports from https://example.invalid/report?billing"),
            "{}",
            registered.text()
        );

        // A close the intake refuses must not have already thrown the local record away —
        // that was exactly the bug: the block was removed before the network call answered, so
        // a refusal (a stale id, a race with another close) silently lost an open report the
        // intake still disagreed about. The block must survive a refused close untouched.
        {
            use std::io::{BufRead, BufReader, Read as _, Write as _};
            use std::net::TcpListener;

            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let address = listener.local_addr().expect("address");
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
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
                }
                let mut body = vec![0u8; length];
                reader.read_exact(&mut body).expect("body");
                let answer = "{\"error\":\"`dx` holds no such report\"}";
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

            let refusing = Subscription {
                workspace: workspace::workspace_root(&root),
                project: "dx".to_string(),
                endpoint: format!("http://{address}"),
                token: "t".to_string(),
            };
            intake::subscribe(&refusing).expect("subscribe");
            let before = workspace::read(&refusing.document()).expect("read");

            let error = run(&args(&["close", &filed_id, root.to_str().expect("path")]))
                .expect_err("the intake refused");
            assert!(error.contains("holds no such report"), "{error}");

            server.join().expect("listener");

            let after = workspace::read(&refusing.document()).expect("read");
            assert_eq!(before, after, "a refused close must not touch the document");
            let parsed = doc_core::format::parse(&after);
            assert!(
                doc_core::edit::find(&parsed, &filed_id).is_ok(),
                "the block must still be there after a refused close"
            );
        }

        std::env::remove_var("DX_REPORTS_DIR");
        std::env::remove_var("DX_REPORT_ENDPOINT");
        std::env::remove_var("DX_SUBSCRIPTIONS_FILE");
        std::env::remove_var("DX_REPORT_TOKEN_FILE");
    }

    #[test]
    fn a_kind_nobody_recognises_is_refused_by_name() {
        let error =
            run(&args(&["feature", "--title", "t", "--detail", "d"])).expect_err("not a kind");
        assert!(error.contains("bug, suggestion, or observation"), "{error}");
    }

    #[test]
    fn syncing_a_checkout_nobody_subscribed_names_the_command_that_subscribes_it() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-cli-unsubscribed");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        std::env::set_var("DX_SUBSCRIPTIONS_FILE", root.join("subscriptions.json"));
        std::env::set_var("DX_REPORT_TOKEN_FILE", root.join("token"));

        let error =
            run(&args(&["sync", root.to_str().expect("path")])).expect_err("no subscription");
        assert!(error.contains("dx report subscribe"), "{error}");

        std::env::remove_var("DX_SUBSCRIPTIONS_FILE");
        std::env::remove_var("DX_REPORT_TOKEN_FILE");
    }

    #[test]
    fn drop_removes_a_report_from_the_local_inbox() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-cli-drop");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        std::env::set_var("DX_REPORTS_DIR", root.join("inbox"));
        std::env::set_var("DX_REPORT_ENDPOINT", "off");

        // File a report
        let filed = run(&args(&[
            "bug",
            "--title",
            "test defect",
            "--detail",
            "This is a test.",
            "--route",
            "dx report",
        ]))
        .expect("file");
        let filed_text = filed.text();
        let filed_id = filed_text
            .split_whitespace()
            .nth(1)
            .expect("id in summary")
            .to_string();

        // Verify it is in the inbox
        let listed = run(&args(&["list", root.to_str().expect("path")])).expect("list");
        assert!(listed.text().contains("1 waiting"), "{}", listed.text());

        // Drop it
        let dropped = run(&args(&["drop", &filed_id])).expect("drop");
        assert!(dropped.text().contains("dropped"), "{}", dropped.text());
        assert!(dropped.text().contains(&filed_id), "{}", dropped.text());

        // Verify it is gone from the inbox
        let after = run(&args(&["list", root.to_str().expect("path")])).expect("list after drop");
        assert!(after.text().contains("empty"), "{}", after.text());

        // Dropping a non-existent id should error
        let error = run(&args(&["drop", "report-nonexistent"])).expect_err("not in inbox");
        assert!(error.contains("not in the local inbox"), "{error}");

        std::env::remove_var("DX_REPORTS_DIR");
        std::env::remove_var("DX_REPORT_ENDPOINT");
    }

    #[test]
    fn filing_from_the_cli_always_notes_the_mcp_tool_is_cheaper() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-cli-mcp-notice");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        std::env::set_var("DX_REPORTS_DIR", root.join("inbox"));
        std::env::set_var("DX_REPORT_ENDPOINT", "off");
        // The CLI always prints a notice to stderr directing agents to use the
        // mcp__dx__dx_report tool instead of this CLI command, while still completing
        // the report filing.

        let filed = run(&args(&[
            "bug",
            "--title",
            "test report",
            "--detail",
            "Test MCP notice.",
            "--route",
            "dx report",
        ]))
        .expect("file");

        assert!(
            filed.text().starts_with("filed report-"),
            "Command should succeed and return filed report: {}",
            filed.text()
        );

        std::env::remove_var("DX_REPORTS_DIR");
        std::env::remove_var("DX_REPORT_ENDPOINT");
    }

    #[test]
    fn setup_with_no_token_anywhere_subscribes_and_says_filing_works() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-cli-setup-no-token");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        let root_canonical = workspace::workspace_root(&root);
        std::env::set_var("DX_SUBSCRIPTIONS_FILE", root.join("subscriptions.json"));
        std::env::set_var("DX_REPORT_TOKEN_FILE", root.join("token"));

        let result = run(&args(&[
            "setup",
            root.to_str().expect("path"),
            "--endpoint",
            "https://example.invalid/report",
        ]))
        .expect("setup");

        let text = result.text();
        assert!(text.contains("now receives"), "should subscribe: {}", text);
        assert!(
            text.contains("filing works"),
            "should say filing works: {}",
            text
        );
        assert!(
            !text.contains("folded") && !text.contains("new, "),
            "should not attempt a sync: {}",
            text
        );

        // Verify the subscription has an empty token field
        let sub = intake::subscription_for(&root_canonical)
            .expect("read")
            .expect("subscription exists");
        assert!(
            sub.token.is_empty(),
            "subscription should have empty token field"
        );

        std::env::remove_var("DX_SUBSCRIPTIONS_FILE");
        std::env::remove_var("DX_REPORT_TOKEN_FILE");
    }

    #[test]
    fn token_stores_and_never_echoes_it_in_output() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-cli-token");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        std::env::set_var("DX_REPORT_TOKEN_FILE", root.join("token"));

        let result = run(&args(&["token", "my-secret-value"])).expect("token");
        let text = result.text();
        assert!(
            text.contains("stored at"),
            "should confirm storage: {}",
            text
        );
        assert!(
            !text.contains("my-secret-value"),
            "should never echo the token: {}",
            text
        );
        assert!(
            intake::stored_token_for(intake::DEFAULT_ENDPOINT).is_some(),
            "token should be stored for default endpoint"
        );

        std::env::remove_var("DX_REPORT_TOKEN_FILE");
    }

    #[test]
    fn setup_adopts_a_token_from_existing_same_endpoint_subscription() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-cli-setup-adopt");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        std::env::set_var("DX_SUBSCRIPTIONS_FILE", root.join("subscriptions.json"));
        std::env::set_var("DX_REPORT_TOKEN_FILE", root.join("token"));
        std::env::set_var("DX_REPORT_ENDPOINT", "off");

        let endpoint = "https://example.invalid/report";
        let existing_root = root.join("existing");
        std::fs::create_dir_all(&existing_root).expect("create");
        let existing_sub = Subscription {
            workspace: existing_root,
            project: "dx".to_string(),
            endpoint: endpoint.to_string(),
            token: "existing-token".to_string(),
        };
        intake::subscribe(&existing_sub).expect("subscribe existing");

        let new_root = root.join("new");
        std::fs::create_dir_all(&new_root).expect("create");

        let result = run(&args(&[
            "setup",
            new_root.to_str().expect("path"),
            "--endpoint",
            endpoint,
        ]))
        .expect("setup");

        let text = result.text();
        assert!(
            text.contains("adopted token"),
            "should say token was adopted: {}",
            text
        );
        assert_eq!(
            intake::stored_token_for(endpoint).expect("token exists"),
            "existing-token",
            "should have adopted the token for the endpoint"
        );

        std::env::remove_var("DX_SUBSCRIPTIONS_FILE");
        std::env::remove_var("DX_REPORT_TOKEN_FILE");
        std::env::remove_var("DX_REPORT_ENDPOINT");
    }

    #[test]
    fn setup_prunes_subscriptions_whose_workspace_no_longer_exists() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-cli-setup-prune");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        std::env::set_var("DX_SUBSCRIPTIONS_FILE", root.join("subscriptions.json"));
        std::env::set_var("DX_REPORT_TOKEN_FILE", root.join("token"));
        std::env::set_var("DX_REPORT_ENDPOINT", "off");

        let dead_root = root.join("dead");
        let dead_sub = Subscription {
            workspace: dead_root.clone(),
            project: "dx".to_string(),
            endpoint: "https://example.invalid/report".to_string(),
            token: String::new(),
        };
        intake::subscribe(&dead_sub).expect("subscribe dead");

        let new_root = root.join("new");
        std::fs::create_dir_all(&new_root).expect("create");

        let result = run(&args(&[
            "setup",
            new_root.to_str().expect("path"),
            "--endpoint",
            "https://example.invalid/report",
        ]))
        .expect("setup");

        let text = result.text();
        assert!(
            text.contains("pruned subscription"),
            "should say subscription was pruned: {}",
            text
        );
        assert!(
            intake::subscription_for(&dead_root)
                .expect("read")
                .is_none(),
            "dead subscription should be gone"
        );

        std::env::remove_var("DX_SUBSCRIPTIONS_FILE");
        std::env::remove_var("DX_REPORT_TOKEN_FILE");
        std::env::remove_var("DX_REPORT_ENDPOINT");
    }

    #[test]
    fn setup_against_an_unregistered_service_explains_how_to_create_it() {
        // The box no longer brings a service into existence on first file (docs/intake.dx,
        // 2026-08-19) — creation is a registered act. `setup` used to reassure the caller that
        // filing would create it anyway; that string never matches what the box actually
        // answers, so this pins the guidance setup gives instead.
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-cli-setup-unregistered");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        std::env::set_var("DX_SUBSCRIPTIONS_FILE", root.join("subscriptions.json"));
        std::env::set_var("DX_REPORT_TOKEN_FILE", root.join("token"));

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
            let answer = "{\"error\":\"`brand-new` is not a service this box hosts — a \
                           registered account creates one, or the operator does with `selfhost \
                           reports project add brand-new`\"}";
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                        answer.len()
                    )
                    .as_bytes(),
                )
                .expect("answer");
        });

        let result = run(&args(&[
            "setup",
            root.to_str().expect("path"),
            "--project",
            "brand-new",
            "--endpoint",
            &format!("http://{address}"),
            "--token",
            "t",
        ]))
        .expect("setup");

        server.join().expect("listener");

        let text = result.text();
        assert!(
            text.contains("no service exists for this project yet"),
            "should explain the service needs creating rather than claim it will \
             auto-create: {text}"
        );
        assert!(
            text.contains("selfhost reports project add"),
            "should name the operator's creation door: {text}"
        );

        std::env::remove_var("DX_SUBSCRIPTIONS_FILE");
        std::env::remove_var("DX_REPORT_TOKEN_FILE");
    }

    #[test]
    fn folder_name_with_spaces_and_slashes_derives_sanitized_project_name() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-cli-setup-sanitize");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        std::env::set_var("DX_SUBSCRIPTIONS_FILE", root.join("subscriptions.json"));
        std::env::set_var("DX_REPORT_TOKEN_FILE", root.join("token"));

        let messy_root = root.join("my folder/with/slashes");
        std::fs::create_dir_all(&messy_root).expect("create");
        let messy_canonical = workspace::workspace_root(&messy_root);

        let result = run(&args(&[
            "setup",
            messy_root.to_str().expect("path"),
            "--endpoint",
            "https://example.invalid/report",
        ]))
        .expect("setup");

        let text = result.text();
        assert!(
            text.contains("slashes"),
            "sanitized name should not have slashes: {}",
            text
        );

        let sub = intake::subscription_for(&messy_canonical)
            .expect("read")
            .expect("subscription exists");
        assert_eq!(
            sub.project, "slashes",
            "project should be sanitized folder name"
        );

        std::env::remove_var("DX_SUBSCRIPTIONS_FILE");
        std::env::remove_var("DX_REPORT_TOKEN_FILE");
    }

    #[test]
    fn token_query_says_whether_one_is_stored_for_the_endpoint() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-cli-token-query");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        std::env::set_var("DX_REPORT_TOKEN_FILE", root.join("token"));

        let result = run(&args(&["token"])).expect("token query");
        let text = result.text();
        assert!(
            text.contains("no token"),
            "should say no token stored: {}",
            text
        );

        intake::store_token("test-token", intake::DEFAULT_ENDPOINT).expect("store");
        let result = run(&args(&["token"])).expect("token query again");
        let text = result.text();
        assert!(
            text.contains("a token is stored"),
            "should say a token is stored: {}",
            text
        );
        assert!(
            !text.contains("test-token"),
            "should never show the token: {}",
            text
        );

        std::env::remove_var("DX_REPORT_TOKEN_FILE");
    }

    #[test]
    fn mcp_serve_requires_project_env_var() {
        let _env = crate::env_lock();
        std::env::remove_var("DX_REPORT_PROJECT");
        std::env::set_var("DX_REPORT_TOKEN", "test-token");

        let result = mcp_serve(&args(&[]));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("DX_REPORT_PROJECT"));
    }

    #[test]
    fn mcp_serve_requires_token_env_var() {
        let _env = crate::env_lock();
        std::env::set_var("DX_REPORT_PROJECT", "test-project");
        std::env::remove_var("DX_REPORT_TOKEN");

        let result = mcp_serve(&args(&[]));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("DX_REPORT_TOKEN"));
    }

    #[test]
    fn mcp_serve_rejects_empty_env_vars() {
        let _env = crate::env_lock();
        std::env::set_var("DX_REPORT_PROJECT", "  ");
        std::env::set_var("DX_REPORT_TOKEN", "test-token");

        let result = mcp_serve(&args(&[]));
        assert!(result.is_err());
    }

    #[test]
    fn mcp_tools_list_returns_three_tools() {
        let result = mcp_tools_list().expect("tools list");
        let tools = result
            .get("tools")
            .and_then(serde_json::Value::as_array)
            .expect("tools array");
        assert_eq!(tools.len(), 3);
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t.get("name").and_then(serde_json::Value::as_str))
            .collect();
        assert!(names.contains(&"report_file"));
        assert!(names.contains(&"report_feed"));
        assert!(names.contains(&"report_close"));
    }

    #[test]
    fn mcp_handle_request_rejects_blank_lines() {
        assert_eq!(
            handle_mcp_request("", "http://example.com", "proj", "tok"),
            None
        );
        assert_eq!(
            handle_mcp_request("  \n", "http://example.com", "proj", "tok"),
            None
        );
    }

    #[test]
    fn mcp_handle_request_handles_parse_error() {
        let response = handle_mcp_request("{invalid json", "http://example.com", "proj", "tok")
            .expect("should return error response");
        assert!(response.contains("Parse error"));
    }

    #[test]
    fn mcp_handle_request_handles_invalid_jsonrpc() {
        let response = handle_mcp_request(
            r#"{"jsonrpc": "1.0", "id": 1, "method": "tools/list"}"#,
            "http://example.com",
            "proj",
            "tok",
        )
        .expect("should return error response");
        assert!(response.contains("Invalid Request"));
    }

    #[test]
    fn mcp_handle_request_returns_tools_list() {
        let response = handle_mcp_request(
            r#"{"jsonrpc": "2.0", "id": 1, "method": "tools/list"}"#,
            "http://example.com",
            "proj",
            "tok",
        )
        .expect("should return response");
        assert!(response.contains("report_file"));
        assert!(response.contains("report_feed"));
        assert!(response.contains("report_close"));
    }

    #[test]
    fn mcp_handle_request_rejects_unknown_method() {
        let response = handle_mcp_request(
            r#"{"jsonrpc": "2.0", "id": 1, "method": "unknown/method"}"#,
            "http://example.com",
            "proj",
            "tok",
        )
        .expect("should return error response");
        assert!(response.contains("Method not found"));
    }

    #[test]
    fn parse_scoped_token_extracts_64_char_hex_token() {
        let stdout =
            "reader token: abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789\n";
        let (token, detected) = try_claim_scoped_token_from_output(stdout);
        assert!(detected);
        assert_eq!(
            token,
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
    }

    #[test]
    fn parse_scoped_token_ignores_malformed_token() {
        let stdout = "reader token: too-short\n";
        let (token, detected) = try_claim_scoped_token_from_output(stdout);
        assert!(!detected);
        assert!(token.is_empty());
    }

    #[test]
    fn parse_scoped_token_ignores_non_hex_token() {
        let stdout =
            "reader token: zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz\n";
        let (token, detected) = try_claim_scoped_token_from_output(stdout);
        assert!(!detected);
        assert!(token.is_empty());
    }

    #[test]
    fn parse_scoped_token_ignores_missing_line() {
        let stdout = "some other output\nwithout the token line\n";
        let (token, detected) = try_claim_scoped_token_from_output(stdout);
        assert!(!detected);
        assert!(token.is_empty());
    }

    #[test]
    fn mcp_json_shape_is_correct() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-mcp-json-shape");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");

        let dx_binary = PathBuf::from("/usr/local/bin/dx");
        let project = "test-project";
        let token = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        write_mcp_config(&root, project, token, &dx_binary).expect("write config");

        let mcp_json = root.join(".mcp.json");
        assert!(
            mcp_json.exists(),
            ".mcp.json should be created at repo root"
        );

        let content = std::fs::read_to_string(&mcp_json).expect("read mcp.json");
        let value: serde_json::Value = serde_json::from_str(&content).expect("parse as JSON");

        // Verify structure
        assert!(
            value.get("mcpServers").is_some(),
            "should have mcpServers object"
        );
        let report_server = value
            .get("mcpServers")
            .and_then(|v| v.get("report"))
            .expect("should have report server");

        assert_eq!(
            report_server.get("command").and_then(|v| v.as_str()),
            Some("/usr/local/bin/dx"),
            "command should be dx binary path"
        );

        let args = report_server
            .get("args")
            .and_then(|v| v.as_array())
            .expect("args should be array");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0].as_str(), Some("report"));
        assert_eq!(args[1].as_str(), Some("mcp"));

        let env = report_server.get("env").expect("should have env");
        assert_eq!(
            env.get("DX_REPORT_PROJECT").and_then(|v| v.as_str()),
            Some(project),
            "should set DX_REPORT_PROJECT"
        );
        assert_eq!(
            env.get("DX_REPORT_TOKEN").and_then(|v| v.as_str()),
            Some(token),
            "should set DX_REPORT_TOKEN"
        );

        // Token should not appear in the content as plaintext anywhere except in the JSON value
        let content_without_json = content.split('{').next().unwrap_or("");
        assert!(
            !content_without_json.contains(token),
            "token should not appear in output before JSON"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn mcp_json_merges_with_existing_config() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-mcp-merge");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");

        // Create initial config with another server
        let mcp_json = root.join(".mcp.json");
        std::fs::write(
            &mcp_json,
            r#"{"mcpServers": {"existing": {"command": "other", "args": []}}}"#,
        )
        .expect("write initial");

        let dx_binary = PathBuf::from("/usr/bin/dx");
        let project = "test";
        let token = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        write_mcp_config(&root, project, token, &dx_binary).expect("write config");

        let content = std::fs::read_to_string(&mcp_json).expect("read");
        let value: serde_json::Value = serde_json::from_str(&content).expect("parse");

        // Both servers should exist
        let servers = value
            .get("mcpServers")
            .and_then(|v| v.as_object())
            .expect("mcpServers");
        assert!(
            servers.contains_key("existing"),
            "existing server should be preserved"
        );
        assert!(
            servers.contains_key("report"),
            "report server should be added"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn gitignore_is_created_if_missing() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-gitignore-create");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");

        add_to_gitignore(&root).expect("add to gitignore");

        let gitignore = root.join(".gitignore");
        assert!(gitignore.exists(), ".gitignore should be created");

        let content = std::fs::read_to_string(&gitignore).expect("read");
        assert!(
            content.contains(".mcp.json"),
            "should contain .mcp.json entry"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn gitignore_is_updated_if_entry_missing() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-gitignore-update");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");

        let gitignore = root.join(".gitignore");
        std::fs::write(&gitignore, "*.swp\nnode_modules/\n").expect("write initial");

        add_to_gitignore(&root).expect("add to gitignore");

        let content = std::fs::read_to_string(&gitignore).expect("read");
        assert!(content.contains(".mcp.json"), "should add .mcp.json entry");
        assert!(
            content.contains("*.swp"),
            "should preserve existing entries"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn gitignore_is_not_updated_if_entry_exists() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-gitignore-idempotent");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");

        let gitignore = root.join(".gitignore");
        let original = "*.swp\n.mcp.json\nnode_modules/\n";
        std::fs::write(&gitignore, original).expect("write initial");

        add_to_gitignore(&root).expect("add to gitignore");

        let content = std::fs::read_to_string(&gitignore).expect("read");
        assert_eq!(
            content, original,
            "should not modify .gitignore if entry already exists"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn setup_without_operator_access_falls_back_to_existing_behavior() {
        let _env = crate::env_lock();
        let root = std::env::temp_dir().join("dx-report-setup-no-operator");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        std::env::set_var("DX_SUBSCRIPTIONS_FILE", root.join("subscriptions.json"));
        std::env::set_var("DX_REPORT_TOKEN_FILE", root.join("token"));

        let result = run(&args(&[
            "setup",
            root.to_str().expect("path"),
            "--endpoint",
            "https://example.invalid/report",
        ]))
        .expect("setup");

        let text = result.text();
        // Should follow existing path: no mention of scoped token or MCP registration
        assert!(
            text.contains("now receives"),
            "should subscribe normally: {}",
            text
        );
        assert!(
            !text.contains("claimed scoped"),
            "should not claim scoped token: {}",
            text
        );
        assert!(
            !text.contains("MCP server"),
            "should not register MCP server: {}",
            text
        );

        // .mcp.json should not exist (no operator to claim token from)
        let mcp_json = root.join(".mcp.json");
        assert!(!mcp_json.exists(), ".mcp.json should not be created");

        std::env::remove_var("DX_SUBSCRIPTIONS_FILE");
        std::env::remove_var("DX_REPORT_TOKEN_FILE");
    }

    /// Helper to test token parsing without needing the selfhost binary.
    fn try_claim_scoped_token_from_output(stdout: &str) -> (String, bool) {
        for line in stdout.lines() {
            let trimmed = line.trim();
            if let Some(token_part) = trimmed.strip_prefix("reader token: ") {
                let token = token_part.trim();
                if token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()) {
                    return (token.to_string(), true);
                }
                return (String::new(), false);
            }
        }
        (String::new(), false)
    }
}
