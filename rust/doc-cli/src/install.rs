//! Registering the `dx` MCP server with whatever assistants are on this machine.
//!
//! Each assistant keeps its MCP servers in its own file, in its own shape. Rather than
//! asking a person to hand-edit five configs, the table below records where each one lives
//! and how it stores a server, and [`Registration::write`] merges an entry in — leaving
//! every other server in the file untouched.
//!
//! Registration is additive and idempotent: running `dx install` twice changes nothing the
//! second time, and removing the entry by hand is enough to undo it.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

/// Name the server is registered under, in every assistant.
pub const SERVER_KEY: &str = "dx";

/// Language runtimes `dx doctor` probes for, with the programs that satisfy each.
pub const RUNTIME_PROBES: &[(&str, &[&str])] = &[
    ("python", &["uv", "python3", "python"]),
    ("node", &["node"]),
    ("typescript", &["deno"]),
    ("bash", &["bash", "sh"]),
    ("rust", &["cargo"]),
    ("go", &["go"]),
    ("ruby", &["ruby"]),
];

/// How an assistant stores MCP servers in its config file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// JSON with servers under a top-level `mcpServers` object.
    JsonUnderMcpServers,
    /// JSON with servers under a top-level `servers` object (VS Code's format).
    JsonUnderServers,
    /// TOML with a `[mcp_servers.<name>]` table (Codex's format).
    Toml,
}

/// One assistant's MCP registration.
#[derive(Debug, Clone)]
pub struct Registration {
    /// Short assistant name, for reports.
    pub agent: &'static str,
    /// The config file to merge into.
    pub config: PathBuf,
    /// Path whose existence means the assistant is installed.
    pub marker: PathBuf,
    /// How this assistant stores servers.
    pub shape: Shape,
}

impl Registration {
    /// Whether this assistant appears to be installed.
    #[must_use]
    pub fn config_exists(&self) -> bool {
        self.marker.exists() || self.config.exists()
    }

    /// Whether `dx` is already registered in this config.
    #[must_use]
    pub fn is_registered(&self) -> bool {
        let Ok(text) = std::fs::read_to_string(&self.config) else {
            return false;
        };
        match self.shape {
            Shape::Toml => text.contains(&format!("[mcp_servers.{SERVER_KEY}]")),
            _ => serde_json::from_str::<Value>(&text).is_ok_and(|config| {
                config
                    .get(self.json_key())
                    .and_then(|servers| servers.get(SERVER_KEY))
                    .is_some()
            }),
        }
    }

    /// Merge a `dx` entry into this config, returning whether anything changed.
    pub fn write(&mut self, binary: &Path) -> Result<bool, String> {
        if self.is_registered() {
            return Ok(false);
        }
        let command = binary.to_string_lossy().into_owned();
        match self.shape {
            Shape::Toml => self.write_toml(&command),
            _ => self.write_json(&command),
        }
    }

    /// The top-level JSON key this assistant keeps servers under.
    fn json_key(&self) -> &'static str {
        match self.shape {
            Shape::JsonUnderServers => "servers",
            _ => "mcpServers",
        }
    }

    /// Merge into a JSON config, preserving every other key and server.
    fn write_json(&self, command: &str) -> Result<bool, String> {
        let mut config = match std::fs::read_to_string(&self.config) {
            Ok(text) if !text.trim().is_empty() => serde_json::from_str::<Value>(&text)
                .map_err(|error| format!("{} is not valid JSON: {error}", self.config.display()))?,
            _ => Value::Object(Map::new()),
        };

        if !config.is_object() {
            return Err(format!("{} is not a JSON object", self.config.display()));
        }
        let key = self.json_key();
        let root = config.as_object_mut().expect("checked object");
        let servers = root
            .entry(key.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        let servers = servers
            .as_object_mut()
            .ok_or_else(|| format!("`{key}` in {} is not an object", self.config.display()))?;

        servers.insert(SERVER_KEY.to_string(), server_entry(command));
        write_file(
            &self.config,
            &format!(
                "{}\n",
                serde_json::to_string_pretty(&config).map_err(|error| error.to_string())?
            ),
        )?;
        Ok(true)
    }

    /// Append a `[mcp_servers.dx]` table to a TOML config.
    fn write_toml(&self, command: &str) -> Result<bool, String> {
        let existing = std::fs::read_to_string(&self.config).unwrap_or_default();
        let separator = if existing.is_empty() || existing.ends_with("\n\n") {
            ""
        } else if existing.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        let block = format!(
            "{separator}[mcp_servers.{SERVER_KEY}]\ncommand = \"{command}\"\nargs = [\"mcp\"]\n"
        );
        write_file(&self.config, &format!("{existing}{block}"))?;
        Ok(true)
    }
}

/// The JSON entry describing how to start the server.
fn server_entry(command: &str) -> Value {
    json!({ "command": command, "args": ["mcp"] })
}

/// Write a config file, creating its directory.
fn write_file(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    std::fs::write(path, contents)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

/// Every assistant registration this machine could have.
#[must_use]
pub fn registrations() -> Vec<Registration> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };

    vec![
        Registration {
            agent: "claude",
            config: home.join(".claude.json"),
            marker: home.join(".claude"),
            shape: Shape::JsonUnderMcpServers,
        },
        Registration {
            agent: "codex",
            config: home.join(".codex").join("config.toml"),
            marker: home.join(".codex"),
            shape: Shape::Toml,
        },
        Registration {
            agent: "cursor",
            config: home.join(".cursor").join("mcp.json"),
            marker: home.join(".cursor"),
            shape: Shape::JsonUnderMcpServers,
        },
        Registration {
            agent: "vscode",
            config: vscode_user_dir(&home).join("mcp.json"),
            marker: vscode_user_dir(&home),
            shape: Shape::JsonUnderServers,
        },
        Registration {
            agent: "windsurf",
            config: home
                .join(".codeium")
                .join("windsurf")
                .join("mcp_config.json"),
            marker: home.join(".codeium"),
            shape: Shape::JsonUnderMcpServers,
        },
        Registration {
            agent: "gemini",
            config: home.join(".gemini").join("settings.json"),
            marker: home.join(".gemini"),
            shape: Shape::JsonUnderMcpServers,
        },
    ]
}

/// VS Code's per-user configuration directory for this operating system.
fn vscode_user_dir(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        return home
            .join("Library")
            .join("Application Support")
            .join("Code")
            .join("User");
    }
    if cfg!(windows) {
        return std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Roaming"))
            .join("Code")
            .join("User");
    }
    home.join(".config").join("Code").join("User")
}

/// Where `dx install` puts the binary when the caller does not choose.
#[must_use]
pub fn default_bin_dir() -> PathBuf {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join("AppData").join("Local"))
            .join("Programs")
            .join("dx");
    }
    home.join(".local").join("bin")
}

/// The current user's home directory.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// Configuration text for assistants this installer does not know about.
#[must_use]
pub fn manual_instructions() -> String {
    format!(
        "Add this to any MCP-capable assistant's config:\n\n{}\n\n\
         For an assistant with shell access and no MCP support, no config is needed —\n\
         it can call the CLI directly:\n\n  \
         dx text <file>        read a document\n  \
         dx outline <file>     list its blocks\n  \
         dx png <file>         render it to an image\n  \
         dx run <file>         execute its code blocks\n",
        serde_json::to_string_pretty(&json!({
            "mcpServers": { SERVER_KEY: server_entry("dx") }
        }))
        .unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-install-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        root
    }

    fn registration(root: &Path, name: &str, shape: Shape) -> Registration {
        Registration {
            agent: "test",
            config: root.join(name),
            marker: root.to_path_buf(),
            shape,
        }
    }

    #[test]
    fn registering_into_json_preserves_other_servers_and_keys() {
        let root = scratch("json-merge");
        let config = root.join("mcp.json");
        std::fs::write(
            &config,
            r#"{"theme":"dark","mcpServers":{"other":{"command":"other"}}}"#,
        )
        .expect("seed");

        let mut entry = registration(&root, "mcp.json", Shape::JsonUnderMcpServers);
        assert!(entry.write(Path::new("/usr/local/bin/dx")).expect("write"));

        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(&config).expect("read")).expect("json");
        assert_eq!(saved["theme"], "dark");
        assert_eq!(saved["mcpServers"]["other"]["command"], "other");
        assert_eq!(saved["mcpServers"]["dx"]["command"], "/usr/local/bin/dx");
        assert_eq!(saved["mcpServers"]["dx"]["args"][0], "mcp");
    }

    #[test]
    fn registering_is_idempotent() {
        let root = scratch("idempotent");
        let mut entry = registration(&root, "mcp.json", Shape::JsonUnderMcpServers);
        assert!(entry.write(Path::new("dx")).expect("first"));
        assert!(!entry.write(Path::new("dx")).expect("second"));
        assert!(entry.is_registered());
    }

    #[test]
    fn vscode_uses_its_own_servers_key() {
        let root = scratch("vscode");
        let mut entry = registration(&root, "mcp.json", Shape::JsonUnderServers);
        entry.write(Path::new("dx")).expect("write");
        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(&entry.config).expect("read"))
                .expect("json");
        assert!(saved["servers"]["dx"].is_object());
        assert!(saved["mcpServers"].is_null());
    }

    #[test]
    fn codex_gets_a_toml_table_appended_after_existing_content() {
        let root = scratch("toml");
        let config = root.join("config.toml");
        std::fs::write(&config, "model = \"gpt\"\n").expect("seed");

        let mut entry = registration(&root, "config.toml", Shape::Toml);
        assert!(entry.write(Path::new("dx")).expect("write"));

        let saved = std::fs::read_to_string(&config).expect("read");
        assert!(saved.starts_with("model = \"gpt\"\n"));
        assert!(saved.contains("[mcp_servers.dx]"));
        assert!(saved.contains("args = [\"mcp\"]"));
        assert!(entry.is_registered());
        assert!(!entry.write(Path::new("dx")).expect("again"));
    }

    #[test]
    fn a_corrupt_json_config_is_reported_not_overwritten() {
        let root = scratch("corrupt");
        let config = root.join("mcp.json");
        std::fs::write(&config, "{ not json").expect("seed");
        let mut entry = registration(&root, "mcp.json", Shape::JsonUnderMcpServers);
        let error = entry.write(Path::new("dx")).expect_err("should fail");
        assert!(error.contains("not valid JSON"));
        assert_eq!(
            std::fs::read_to_string(&config).expect("read"),
            "{ not json"
        );
    }

    #[test]
    fn the_known_assistants_all_have_distinct_configs() {
        let entries = registrations();
        assert!(entries.len() >= 5);
        let mut paths: Vec<String> = entries
            .iter()
            .map(|entry| entry.config.display().to_string())
            .collect();
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), entries.len());
    }

    #[test]
    fn manual_instructions_cover_both_mcp_and_plain_shell_agents() {
        let text = manual_instructions();
        assert!(text.contains("mcpServers"));
        assert!(text.contains("dx text <file>"));
        assert!(text.contains("dx run <file>"));
    }
}
