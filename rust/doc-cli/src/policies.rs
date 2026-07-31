//! Handing Firefox the extension with no clicks, through the policy file it reads at startup.
//!
//! Firefox is the one browser family that lets a program on this machine install an extension
//! without the person clicking through a browser page: a `policies.json` beside the
//! application names an add-on and Firefox installs it at the next start. That file is *also*
//! how an administrator configures everything else about Firefox, so this must merge into it
//! and never replace it — clobbering someone's proxy or certificate policy to add a document
//! renderer would be a poor trade.
//!
//! # The one thing that is not optional
//! Release and Beta Firefox refuse an unsigned add-on, and no policy overrides that. The
//! `install_url` therefore has to name an XPI that Mozilla signed — an *unlisted* signature
//! from addons.mozilla.org, which is free and is a release step, not something this machine
//! can do. Developer Edition, Nightly, and LibreWolf will take an unsigned one, which is why
//! [`force_install`] reports what it wrote rather than promising it worked: whether Firefox
//! accepts the add-on is Firefox's decision, made at its next start, and the honest thing is
//! to say what was configured and let `dx doctor` report what actually happened.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::extension::GECKO_ID;

/// The policy key Firefox reads per-extension settings from.
const SETTINGS: &str = "ExtensionSettings";

/// Where a Firefox installation at `application` reads its policy file from.
///
/// Every platform puts it beside the application rather than in the profile, because a policy
/// is meant to apply to every profile the person has.
#[must_use]
pub fn policy_file(application: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        // `application` is the .app bundle itself.
        return application
            .join("Contents")
            .join("Resources")
            .join("distribution")
            .join("policies.json");
    }
    // Elsewhere it is the executable, and the distribution directory sits beside it. The path
    // is canonicalized first because a packaged Firefox is usually reached through a symlink
    // on `PATH` (`/usr/bin/firefox` → `/usr/lib/firefox/firefox`), and the policy has to land
    // next to the real thing.
    let executable =
        std::fs::canonicalize(application).unwrap_or_else(|_| application.to_path_buf());
    executable
        .parent()
        .unwrap_or(Path::new("."))
        .join("distribution")
        .join("policies.json")
}

/// Add — or update — the entry that force-installs the add-on at `xpi`.
///
/// Returns whether anything changed, so an installer can say "already configured" rather than
/// implying work it did not do.
///
/// # Errors
/// When the existing policy file is not JSON, is not an object, or cannot be written. A file
/// that cannot be parsed is reported and left exactly as it was: it is somebody's
/// configuration, and guessing at what they meant is worse than saying it needs a look.
pub fn force_install(file: &Path, xpi: &Path) -> Result<bool, String> {
    let mut policy = read(file)?;
    let entry = json!({
        // The reader may still turn it off; force_installed only means it arrives installed.
        // Taking that choice away would be a browser-management posture dx has no business
        // adopting to render documents.
        "installation_mode": "force_installed",
        "install_url": file_url(xpi),
        // A file:// install has no update URL to poll, so update checks are turned off
        // rather than left to fail on every start.
        "updates_disabled": true,
    });

    let policies = object_at(&mut policy, "policies")?;
    let settings = object_at_map(policies, SETTINGS)?;
    if settings.get(GECKO_ID) == Some(&entry) {
        return Ok(false);
    }
    settings.insert(GECKO_ID.to_string(), entry);
    write(file, &policy)?;
    Ok(true)
}

/// Remove the add-on's entry, leaving every other policy exactly as it was.
///
/// Returns whether anything was there to remove.
///
/// # Errors
/// When the policy file exists but cannot be parsed or rewritten.
pub fn remove(file: &Path) -> Result<bool, String> {
    if !file.exists() {
        return Ok(false);
    }
    let mut policy = read(file)?;
    let policies = object_at(&mut policy, "policies")?;
    let settings = object_at_map(policies, SETTINGS)?;
    if settings.remove(GECKO_ID).is_none() {
        return Ok(false);
    }
    // Leave no empty scaffolding behind: a `policies.json` holding nothing but empty objects
    // is indistinguishable from one somebody meant to write, and it is ours to clean up.
    let emptied = settings.is_empty();
    if emptied {
        if let Some(map) = policy.get_mut("policies").and_then(Value::as_object_mut) {
            map.remove(SETTINGS);
        }
    }
    if policy["policies"].as_object().is_some_and(Map::is_empty) {
        std::fs::remove_file(file)
            .map_err(|error| format!("could not remove {}: {error}", file.display()))?;
        return Ok(true);
    }
    write(file, &policy)?;
    Ok(true)
}

/// Whether the policy file already force-installs the add-on at `xpi`.
#[must_use]
pub fn installed(file: &Path, xpi: &Path) -> bool {
    let Ok(policy) = read(file) else {
        return false;
    };
    policy["policies"][SETTINGS][GECKO_ID]["install_url"] == json!(file_url(xpi))
}

/// The policy file's contents, or an empty policy when there is no file yet.
fn read(file: &Path) -> Result<Value, String> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(_) => return Ok(json!({ "policies": {} })),
    };
    if text.trim().is_empty() {
        return Ok(json!({ "policies": {} }));
    }
    serde_json::from_str(&text)
        .map_err(|error| format!("{} is not valid JSON: {error}", file.display()))
}

/// Write a policy file, creating the distribution directory if Firefox has none.
fn write(file: &Path, policy: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(policy)
        .map_err(|error| format!("could not write the policy: {error}"))?;
    crate::state::write_file(file, format!("{text}\n").as_bytes())
}

/// The object at `key`, creating it when absent.
fn object_at<'a>(value: &'a mut Value, key: &str) -> Result<&'a mut Value, String> {
    let root = value
        .as_object_mut()
        .ok_or_else(|| "a Firefox policy file must be a JSON object".to_string())?;
    Ok(root
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new())))
}

/// The map at `key` within `value`, creating it when absent.
fn object_at_map<'a>(
    value: &'a mut Value,
    key: &str,
) -> Result<&'a mut Map<String, Value>, String> {
    object_at(value, key)?
        .as_object_mut()
        .ok_or_else(|| format!("`{key}` in the Firefox policy file is not an object"))
}

/// A `file://` URL naming `path`.
///
/// Only the characters that would end the path or change its meaning are escaped; a URL is
/// what Firefox wants here, and a bare path with a space in it is not one.
fn file_url(path: &Path) -> String {
    let mut url = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                url.push(byte as char);
            }
            _ => url.push_str(&format!("%{byte:02X}")),
        }
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dx-policies-tests-{label}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        root
    }

    #[test]
    fn installing_writes_an_entry_naming_the_signed_add_on() {
        let root = scratch("write");
        let file = root.join("policies.json");
        let xpi = root.join("dx.xpi");

        assert!(force_install(&file, &xpi).expect("install"));
        assert!(installed(&file, &xpi));

        let saved = read(&file).expect("read");
        assert_eq!(
            saved["policies"][SETTINGS][GECKO_ID]["installation_mode"],
            "force_installed"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn installing_twice_changes_nothing_the_second_time() {
        let root = scratch("idempotent");
        let file = root.join("policies.json");
        let xpi = root.join("dx.xpi");
        assert!(force_install(&file, &xpi).expect("first"));
        assert!(!force_install(&file, &xpi).expect("second"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The defect this pins would be somebody's whole Firefox configuration: a policy file is
    /// where certificates, proxies, and update settings live, and dx is a guest in it.
    #[test]
    fn every_other_policy_survives_installing_and_removing() {
        let root = scratch("merge");
        let file = root.join("policies.json");
        std::fs::write(
            &file,
            r#"{"policies":{"DisableTelemetry":true,"ExtensionSettings":{"other@example":{"installation_mode":"blocked"}}}}"#,
        )
        .expect("seed");

        let xpi = root.join("dx.xpi");
        force_install(&file, &xpi).expect("install");
        let saved = read(&file).expect("read");
        assert_eq!(saved["policies"]["DisableTelemetry"], true);
        assert_eq!(
            saved["policies"][SETTINGS]["other@example"]["installation_mode"],
            "blocked"
        );

        assert!(remove(&file).expect("remove"));
        let after = read(&file).expect("read");
        assert_eq!(after["policies"]["DisableTelemetry"], true);
        assert_eq!(
            after["policies"][SETTINGS]["other@example"]["installation_mode"],
            "blocked"
        );
        assert!(after["policies"][SETTINGS][GECKO_ID].is_null());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A policy file dx created and dx emptied should not be left behind: an orphan file
    /// beside Firefox looks like configuration somebody chose.
    #[test]
    fn removing_the_only_entry_takes_the_file_dx_created_with_it() {
        let root = scratch("cleanup");
        let file = root.join("policies.json");
        let xpi = root.join("dx.xpi");
        force_install(&file, &xpi).expect("install");
        assert!(remove(&file).expect("remove"));
        assert!(!file.exists(), "an empty policy file was left behind");
        assert!(!remove(&file).expect("again"), "nothing left to remove");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_policy_file_that_is_not_json_is_reported_and_left_alone() {
        let root = scratch("corrupt");
        let file = root.join("policies.json");
        std::fs::write(&file, "{ not json").expect("seed");
        let error = force_install(&file, &root.join("dx.xpi")).expect_err("should fail");
        assert!(error.contains("not valid JSON"), "{error}");
        assert_eq!(
            std::fs::read_to_string(&file).expect("read"),
            "{ not json",
            "the file was modified despite the failure"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_path_with_spaces_becomes_a_url_firefox_can_read() {
        let url = file_url(Path::new("/Applications/DX.app/Contents/Resources/dx.xpi"));
        assert!(url.starts_with("file:///Applications/"), "{url}");
        assert!(!file_url(Path::new("/a b/dx.xpi")).contains(' '));
        assert!(file_url(Path::new("/a b/dx.xpi")).contains("%20"));
    }

    #[test]
    fn the_policy_file_sits_where_this_platform_looks_for_it() {
        let file = policy_file(Path::new("/Applications/Firefox.app"));
        assert!(file.ends_with("distribution/policies.json"), "{file:?}");
        if cfg!(target_os = "macos") {
            assert!(file.starts_with("/Applications/Firefox.app/Contents/Resources"));
        }
    }
}
