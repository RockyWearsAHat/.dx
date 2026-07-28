//! Turning a code block into an execution plan: files to write, then commands to run.
//!
//! Every language is described the same way — some files, an optional one-time setup step
//! that installs libraries, and the command that actually runs the code. Adding a language
//! means adding one function here, not a new code path through the engine.
//!
//! # Where dependencies come from
//! A block declares libraries with `deps`:
//!
//! ```text
//! ::code id=chart lang=python run deps="matplotlib numpy"
//! ```
//!
//! Each language installs them the way its own users would: `uv` (or a virtualenv) for
//! Python, `npm install` for Node, `Cargo.toml` for Rust, `go get` for Go, `gem install`
//! for Ruby. Installs happen in a per-fingerprint cache directory, so the second run of an
//! unchanged block skips setup entirely.

use std::path::Path;

use crate::process::CommandSpec;
use crate::toolchain::{first_available, have, missing_toolchain_message};

/// A prepared execution for one code block.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Files to materialize in the sandbox, as `(relative path, contents)`.
    pub files: Vec<(String, String)>,
    /// One-time dependency installation commands, run in the sandbox.
    pub setup: Vec<CommandSpec>,
    /// The command that runs the block's code.
    pub run: CommandSpec,
    /// Run in the sandbox rather than beside the document.
    ///
    /// Most languages run in the document's own directory, so `open("data.csv")` finds the
    /// file sitting next to the `.dx`. Project-shaped toolchains (Cargo, Go modules) must
    /// run from their project root instead.
    pub run_in_sandbox: bool,
}

/// Build the execution plan for `runner`, or explain which toolchain is missing.
///
/// `sandbox` is the per-block cache directory; absolute paths into it are baked into the
/// returned commands so the code can run from anywhere.
pub fn build(runner: &str, code: &str, deps: &[String], sandbox: &Path) -> Result<Plan, String> {
    match runner {
        "python" => python(code, deps, sandbox),
        "node" => node(code, deps, sandbox),
        "deno" => deno(code, sandbox),
        "bash" => bash(code, sandbox),
        "rust" => rust(code, deps),
        "go" => go(code, deps),
        "ruby" => ruby(code, deps, sandbox),
        other => Err(format!("no runner for `{other}`")),
    }
}

/// Split a `deps` attribute into individual package specs.
///
/// Commas and whitespace both separate, so `deps="requests, rich"` and
/// `deps="requests rich"` mean the same thing.
#[must_use]
pub fn parse_deps(deps: &str) -> Vec<String> {
    deps.split([',', ' ', '\t', '\n'])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

/// Absolute path to a file inside the sandbox, as a string.
fn sandbox_path(sandbox: &Path, name: &str) -> String {
    sandbox.join(name).to_string_lossy().into_owned()
}

/// Python: `uv` when available (it resolves dependencies per script), else a cached
/// virtualenv, else the bare interpreter for dependency-free blocks.
fn python(code: &str, deps: &[String], sandbox: &Path) -> Result<Plan, String> {
    let script = sandbox_path(sandbox, "block.py");

    if have("uv") {
        return Ok(Plan {
            files: vec![("block.py".to_string(), with_inline_metadata(code, deps))],
            setup: Vec::new(),
            run: CommandSpec::new("uv", &["run", "--quiet", "--no-project", &script]),
            run_in_sandbox: false,
        });
    }

    let interpreter = first_available(&["python3", "python"])
        .ok_or_else(|| missing_toolchain_message("python", &["uv", "python3"]))?;

    if deps.is_empty() {
        return Ok(Plan {
            files: vec![("block.py".to_string(), code.to_string())],
            setup: Vec::new(),
            run: CommandSpec::new(interpreter, &[&script]),
            run_in_sandbox: false,
        });
    }

    let venv = sandbox_path(sandbox, ".venv");
    let venv_python = sandbox_path(sandbox, venv_relative_python());
    let mut install = CommandSpec::new(venv_python.clone(), &["-m", "pip", "install", "--quiet"]);
    install.args.extend(deps.iter().cloned());

    Ok(Plan {
        files: vec![("block.py".to_string(), code.to_string())],
        setup: vec![
            CommandSpec::new(interpreter, &["-m", "venv", &venv]),
            install,
        ],
        run: CommandSpec::new(venv_python, &[&script]),
        run_in_sandbox: false,
    })
}

/// Path of the interpreter inside a virtualenv, which differs on Windows.
fn venv_relative_python() -> &'static str {
    if cfg!(windows) {
        ".venv\\Scripts\\python.exe"
    } else {
        ".venv/bin/python"
    }
}

/// Prepend PEP 723 inline script metadata so `uv` installs the declared dependencies.
fn with_inline_metadata(code: &str, deps: &[String]) -> String {
    if deps.is_empty() {
        return code.to_string();
    }
    let listed = deps
        .iter()
        .map(|dep| format!("#   \"{dep}\","))
        .collect::<Vec<_>>()
        .join("\n");
    format!("# /// script\n# dependencies = [\n{listed}\n# ]\n# ///\n{code}")
}

/// Node: dependencies install into the sandbox with npm; the script runs as an ES module.
fn node(code: &str, deps: &[String], sandbox: &Path) -> Result<Plan, String> {
    if !have("node") {
        return Err(missing_toolchain_message("javascript", &["node"]));
    }
    let script = sandbox_path(sandbox, "block.mjs");
    let modules = sandbox_path(sandbox, "node_modules");
    let mut files = vec![("block.mjs".to_string(), code.to_string())];
    let mut setup = Vec::new();

    if !deps.is_empty() {
        if !have("npm") {
            return Err(missing_toolchain_message(
                "javascript dependencies",
                &["npm"],
            ));
        }
        files.push((
            "package.json".to_string(),
            "{\n  \"name\": \"dx-block\",\n  \"private\": true,\n  \"type\": \"module\"\n}\n"
                .to_string(),
        ));
        let mut install = CommandSpec::new(
            "npm",
            &["install", "--silent", "--no-audit", "--no-fund", "--save"],
        );
        install.args.extend(deps.iter().cloned());
        setup.push(install);
    }

    Ok(Plan {
        files,
        setup,
        run: CommandSpec::new("node", &[&script]).with_env("NODE_PATH", modules),
        run_in_sandbox: false,
    })
}

/// TypeScript via Deno, which resolves `npm:` and `jsr:` imports from the code itself.
fn deno(code: &str, sandbox: &Path) -> Result<Plan, String> {
    if !have("deno") {
        return Err(missing_toolchain_message("typescript", &["deno"]));
    }
    let script = sandbox_path(sandbox, "block.ts");
    Ok(Plan {
        files: vec![("block.ts".to_string(), code.to_string())],
        setup: Vec::new(),
        run: CommandSpec::new("deno", &["run", "--allow-all", "--quiet", &script]),
        run_in_sandbox: false,
    })
}

/// Shell scripts run under bash, which exists on macOS and Linux and ships with Git on
/// Windows.
fn bash(code: &str, sandbox: &Path) -> Result<Plan, String> {
    let shell = first_available(&["bash", "sh"])
        .ok_or_else(|| missing_toolchain_message("shell", &["bash"]))?;
    let script = sandbox_path(sandbox, "block.sh");
    Ok(Plan {
        files: vec![("block.sh".to_string(), code.to_string())],
        setup: Vec::new(),
        run: CommandSpec::new(shell, &[&script]),
        run_in_sandbox: false,
    })
}

/// Rust compiles as a tiny Cargo project so `deps` become real crate dependencies.
fn rust(code: &str, deps: &[String]) -> Result<Plan, String> {
    if !have("cargo") {
        return Err(missing_toolchain_message("rust", &["cargo"]));
    }
    Ok(Plan {
        files: vec![
            ("Cargo.toml".to_string(), cargo_manifest(deps)),
            ("src/main.rs".to_string(), code.to_string()),
        ],
        setup: Vec::new(),
        run: CommandSpec::new("cargo", &["run", "--quiet", "--release"]),
        run_in_sandbox: true,
    })
}

/// Build a `Cargo.toml` whose `[dependencies]` mirror the block's `deps`.
///
/// A bare name takes any version; `name=1.2` pins one, matching how `cargo add` reads.
fn cargo_manifest(deps: &[String]) -> String {
    let entries = deps
        .iter()
        .map(|dep| match dep.split_once('=') {
            Some((name, version)) => format!("{name} = \"{version}\""),
            None => format!("{dep} = \"*\""),
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "[package]\nname = \"dx-block\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n\
         [[bin]]\nname = \"dx-block\"\npath = \"src/main.rs\"\n\n[dependencies]\n{entries}\n"
    )
}

/// Go builds as a module so `go get` can fetch declared packages.
fn go(code: &str, deps: &[String]) -> Result<Plan, String> {
    if !have("go") {
        return Err(missing_toolchain_message("go", &["go"]));
    }
    let mut setup = Vec::new();
    if !deps.is_empty() {
        let mut get = CommandSpec::new("go", &["get"]);
        get.args.extend(deps.iter().cloned());
        setup.push(get);
    }
    Ok(Plan {
        files: vec![
            (
                "go.mod".to_string(),
                "module dxblock\n\ngo 1.21\n".to_string(),
            ),
            ("main.go".to_string(), code.to_string()),
        ],
        setup,
        run: CommandSpec::new("go", &["run", "."]),
        run_in_sandbox: true,
    })
}

/// Ruby installs gems into the sandbox so a block cannot disturb the system gem set.
fn ruby(code: &str, deps: &[String], sandbox: &Path) -> Result<Plan, String> {
    if !have("ruby") {
        return Err(missing_toolchain_message("ruby", &["ruby"]));
    }
    let script = sandbox_path(sandbox, "block.rb");
    let gem_home = sandbox_path(sandbox, "gems");
    let mut setup = Vec::new();
    if !deps.is_empty() {
        let mut install = CommandSpec::new(
            "gem",
            &["install", "--no-document", "--install-dir", &gem_home],
        );
        install.args.extend(deps.iter().cloned());
        setup.push(install);
    }
    Ok(Plan {
        files: vec![("block.rb".to_string(), code.to_string())],
        setup,
        run: CommandSpec::new("ruby", &[&script]).with_env("GEM_HOME", gem_home),
        run_in_sandbox: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sandbox() -> PathBuf {
        PathBuf::from("/tmp/dx-sandbox")
    }

    #[test]
    fn deps_split_on_commas_and_whitespace() {
        assert_eq!(parse_deps("requests, rich"), vec!["requests", "rich"]);
        assert_eq!(parse_deps("  a\tb\nc "), vec!["a", "b", "c"]);
        assert!(parse_deps("   ").is_empty());
    }

    #[test]
    fn python_plans_write_the_script_and_run_beside_the_document() {
        let plan = build("python", "print(1)", &[], &sandbox()).expect("python plan");
        assert_eq!(plan.files[0].0, "block.py");
        assert!(!plan.run_in_sandbox);
        assert!(plan.run.display().contains("block.py"));
    }

    #[test]
    fn python_dependencies_reach_the_runner_one_way_or_another() {
        let plan = build("python", "import rich", &["rich".into()], &sandbox())
            .expect("python plan with deps");
        let declares_dependency = plan.files[0].1.contains("\"rich\"")
            || plan
                .setup
                .iter()
                .any(|step| step.display().contains("rich"));
        assert!(declares_dependency, "dependency never gets installed");
    }

    #[test]
    fn cargo_manifest_pins_versions_when_asked() {
        let manifest = cargo_manifest(&["serde=1.0".into(), "rand".into()]);
        assert!(manifest.contains("serde = \"1.0\""));
        assert!(manifest.contains("rand = \"*\""));
    }

    #[test]
    fn project_shaped_languages_run_from_their_own_root() {
        if have("cargo") {
            let plan = build("rust", "fn main() {}", &[], &sandbox()).expect("rust plan");
            assert!(plan.run_in_sandbox);
            assert!(plan.files.iter().any(|(name, _)| name == "Cargo.toml"));
        }
    }

    #[test]
    fn an_unknown_runner_is_an_error_not_a_panic() {
        assert!(build("cobol", "x", &[], &sandbox()).is_err());
    }

    #[test]
    fn inline_metadata_is_only_added_when_there_are_dependencies() {
        assert_eq!(with_inline_metadata("x", &[]), "x");
        let with = with_inline_metadata("x", &["rich".into()]);
        assert!(with.starts_with("# /// script"));
        assert!(with.contains("\"rich\""));
        assert!(with.ends_with("x"));
    }
}
