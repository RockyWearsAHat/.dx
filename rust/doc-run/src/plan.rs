//! Turning a code block into an execution plan: files to write, libraries to install, then
//! the command that runs the code.
//!
//! Every language is described the same way, and the shape is what makes the sandbox
//! possible:
//!
//! | Phase | Gets the network | Runs in |
//! |-------|------------------|---------|
//! | `setup` — install the declared libraries, compile the project | yes | the block directory |
//! | `run` — the author's code | **no** | the document's directory |
//!
//! Splitting them is not tidiness. A block's own code must never reach the network (see
//! [`crate::confine`]), and `uv`, `npm`, `cargo`, `go`, and `gem` all have to. So every
//! language is arranged to do its fetching *first*, under its own phase, and to run with
//! everything already on disk — which is why `rust` builds a binary in setup and then runs
//! that binary, rather than calling `cargo run` and needing the network mid-execution.
//!
//! # Where dependencies come from
//! A block declares libraries with `deps`:
//!
//! ```text
//! ::code id=chart lang=python run deps="matplotlib numpy"
//! ```
//!
//! Each language installs them the way its own users would. Installs happen in a
//! per-fingerprint block directory, so the second run of an unchanged block skips setup
//! entirely, and each toolchain's *download* cache is shared across blocks in a directory dx
//! owns — never in the reader's home, which the sandbox keeps read-only.

use std::path::PathBuf;

use crate::process::CommandSpec;
use crate::toolchain::{first_available, have, missing_toolchain_message};

/// The two directories a plan is built against.
#[derive(Debug, Clone)]
pub struct Dirs {
    /// This block's own directory, named after its fingerprint. Writable; nothing else is.
    pub block: PathBuf,
    /// Where the toolchains keep what they download, shared across blocks.
    ///
    /// Every toolchain is pointed here explicitly, because its default is somewhere under
    /// the reader's home directory and the sandbox does not let a block write there. A
    /// shared cache is also the difference between one download and one per block.
    pub toolchains: PathBuf,
}

impl Dirs {
    /// Absolute path to a file inside the block directory, as a string.
    fn block_path(&self, name: &str) -> String {
        self.block.join(name).to_string_lossy().into_owned()
    }

    /// Absolute path to one toolchain's cache, as a string.
    fn cache(&self, tool: &str) -> String {
        self.toolchains.join(tool).to_string_lossy().into_owned()
    }
}

/// A prepared execution for one code block.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Files to materialize in the block directory, as `(relative path, contents)`.
    pub files: Vec<(String, String)>,
    /// Dependency installation and compilation, run once in the block directory with the
    /// network available.
    pub setup: Vec<CommandSpec>,
    /// The command that runs the block's code, offline, in the document's directory.
    pub run: CommandSpec,
}

/// Build the execution plan for `runner`, or explain which toolchain is missing.
///
/// Absolute paths into `dirs.block` are baked into the returned commands, so the code can be
/// run from the document's directory and still find the files that were written for it.
///
/// # Errors
/// Returns a sentence naming what to install when the language's toolchain is absent.
pub fn build(runner: &str, code: &str, deps: &[String], dirs: &Dirs) -> Result<Plan, String> {
    match runner {
        "python" => python(code, deps, dirs),
        "node" => node(code, deps, dirs),
        "deno" => deno(code, dirs),
        "bash" => bash(code, dirs),
        "rust" => rust(code, deps, dirs),
        "go" => go(code, deps, dirs),
        "ruby" => ruby(code, deps, dirs),
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

/// Python: a virtual environment built in setup, and the interpreter inside it at run time.
///
/// `uv` builds it when present because it is far faster; `python -m venv` when it is not.
/// Either way the run command is the same interpreter inside the block's own `.venv`, which
/// is what lets the code run with no network and no writes anywhere but its own directory.
fn python(code: &str, deps: &[String], dirs: &Dirs) -> Result<Plan, String> {
    let script = dirs.block_path("block.py");
    let venv = dirs.block_path(".venv");
    let venv_python = dirs.block_path(venv_relative_python());
    let files = vec![("block.py".to_string(), code.to_string())];

    // No libraries to install: the system interpreter can run the script as it is, and
    // building a virtual environment for it would be a download for nothing.
    if deps.is_empty() {
        let interpreter = first_available(&["python3", "python"])
            .ok_or_else(|| missing_toolchain_message("python", &["python3", "uv"]))?;
        return Ok(Plan {
            files,
            setup: Vec::new(),
            run: CommandSpec::new(interpreter, &[&script]),
        });
    }

    let mut setup = Vec::new();
    if have("uv") {
        setup.push(with_cache(
            CommandSpec::new("uv", &["venv", "--quiet", &venv]),
            "UV_CACHE_DIR",
            dirs.cache("uv"),
        ));
        let mut install = CommandSpec::new(
            "uv",
            &["pip", "install", "--quiet", "--python", &venv_python],
        );
        install.args.extend(deps.iter().cloned());
        setup.push(with_cache(install, "UV_CACHE_DIR", dirs.cache("uv")));
    } else {
        let interpreter = first_available(&["python3", "python"])
            .ok_or_else(|| missing_toolchain_message("python", &["python3", "uv"]))?;
        setup.push(CommandSpec::new(interpreter, &["-m", "venv", &venv]));
        let mut install =
            CommandSpec::new(venv_python.clone(), &["-m", "pip", "install", "--quiet"]);
        install.args.extend(deps.iter().cloned());
        setup.push(with_cache(install, "PIP_CACHE_DIR", dirs.cache("pip")));
    }

    Ok(Plan {
        files,
        setup,
        run: CommandSpec::new(venv_python, &[&script]),
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

/// Point a toolchain's download cache at a directory dx owns and the sandbox allows.
fn with_cache(spec: CommandSpec, variable: &str, directory: String) -> CommandSpec {
    spec.with_env(variable, directory)
}

/// Node: dependencies install into the block directory with npm; the script runs as an ES
/// module against what setup already fetched.
fn node(code: &str, deps: &[String], dirs: &Dirs) -> Result<Plan, String> {
    if !have("node") {
        return Err(missing_toolchain_message("javascript", &["node"]));
    }
    let script = dirs.block_path("block.mjs");
    let modules = dirs.block_path("node_modules");
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
        setup.push(with_cache(install, "npm_config_cache", dirs.cache("npm")));
    }

    Ok(Plan {
        files,
        setup,
        run: CommandSpec::new("node", &[&script]).with_env("NODE_PATH", modules),
    })
}

/// TypeScript via Deno: setup fetches every remote import, and the run is offline.
///
/// Deno's own permission flags are set as narrowly as the sandbox already is. They are
/// redundant with it on purpose — two independent things have to fail before a block gets
/// out, and `--allow-all` (which this used to pass) is the wrong default to leave lying
/// around for whoever reads this next.
fn deno(code: &str, dirs: &Dirs) -> Result<Plan, String> {
    if !have("deno") {
        return Err(missing_toolchain_message("typescript", &["deno"]));
    }
    let script = dirs.block_path("block.ts");
    let cache = dirs.cache("deno");
    Ok(Plan {
        files: vec![("block.ts".to_string(), code.to_string())],
        setup: vec![with_cache(
            CommandSpec::new("deno", &["cache", "--quiet", &script]),
            "DENO_DIR",
            cache.clone(),
        )],
        run: with_cache(
            CommandSpec::new(
                "deno",
                &[
                    "run",
                    "--quiet",
                    "--cached-only",
                    "--allow-read",
                    "--allow-env",
                    "--allow-sys",
                    &script,
                ],
            ),
            "DENO_DIR",
            cache,
        ),
    })
}

/// Shell scripts run under bash, which exists on macOS and Linux and ships with Git on
/// Windows.
fn bash(code: &str, dirs: &Dirs) -> Result<Plan, String> {
    let shell = first_available(&["bash", "sh"])
        .ok_or_else(|| missing_toolchain_message("shell", &["bash"]))?;
    let script = dirs.block_path("block.sh");
    Ok(Plan {
        files: vec![("block.sh".to_string(), code.to_string())],
        setup: Vec::new(),
        run: CommandSpec::new(shell, &[&script]),
    })
}

/// Rust compiles as a tiny Cargo project in setup, and the run is the binary that produced.
///
/// `cargo run` would have to resolve, download, and compile *during* the run, which is
/// exactly the phase that has no network. Building first also means a re-run of an unchanged
/// block starts a program rather than starting cargo.
fn rust(code: &str, deps: &[String], dirs: &Dirs) -> Result<Plan, String> {
    if !have("cargo") {
        return Err(missing_toolchain_message("rust", &["cargo"]));
    }
    let binary = dirs.block_path(&format!("target/release/dx-block{}", exe_suffix()));
    Ok(Plan {
        files: vec![
            ("Cargo.toml".to_string(), cargo_manifest(deps)),
            ("src/main.rs".to_string(), code.to_string()),
        ],
        setup: vec![
            CommandSpec::new("cargo", &["build", "--quiet", "--release"])
                .with_env("CARGO_HOME", dirs.cache("cargo")),
        ],
        run: CommandSpec::new(binary, &[]),
    })
}

/// Executable suffix for a compiled block, which Windows adds and nothing else does.
fn exe_suffix() -> &'static str {
    if cfg!(windows) {
        ".exe"
    } else {
        ""
    }
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

/// Go builds a module in setup — fetching declared packages — and runs the built binary.
fn go(code: &str, deps: &[String], dirs: &Dirs) -> Result<Plan, String> {
    if !have("go") {
        return Err(missing_toolchain_message("go", &["go"]));
    }
    let binary = dirs.block_path(&format!("dx-block{}", exe_suffix()));
    let mut setup = Vec::new();
    if !deps.is_empty() {
        let mut get = CommandSpec::new("go", &["get"]);
        get.args.extend(deps.iter().cloned());
        setup.push(go_env(get, dirs));
    }
    setup.push(go_env(
        CommandSpec::new("go", &["build", "-o", &binary, "."]),
        dirs,
    ));

    Ok(Plan {
        files: vec![
            (
                "go.mod".to_string(),
                "module dxblock\n\ngo 1.21\n".to_string(),
            ),
            ("main.go".to_string(), code.to_string()),
        ],
        setup,
        run: CommandSpec::new(binary, &[]),
    })
}

/// Point Go's three caches into dx's own directory, since the reader's home is read-only.
fn go_env(spec: CommandSpec, dirs: &Dirs) -> CommandSpec {
    spec.with_env("GOPATH", dirs.cache("go"))
        .with_env("GOMODCACHE", dirs.cache("go-mod"))
        .with_env("GOCACHE", dirs.cache("go-build"))
}

/// Ruby installs gems into the block directory so a block cannot disturb the system gem set.
fn ruby(code: &str, deps: &[String], dirs: &Dirs) -> Result<Plan, String> {
    if !have("ruby") {
        return Err(missing_toolchain_message("ruby", &["ruby"]));
    }
    let script = dirs.block_path("block.rb");
    let gem_home = dirs.block_path("gems");
    let mut setup = Vec::new();
    if !deps.is_empty() {
        let mut install = CommandSpec::new(
            "gem",
            &["install", "--no-document", "--install-dir", &gem_home],
        );
        install.args.extend(deps.iter().cloned());
        setup.push(install.with_env("GEM_SPEC_CACHE", dirs.cache("gem")));
    }
    Ok(Plan {
        files: vec![("block.rb".to_string(), code.to_string())],
        setup,
        run: CommandSpec::new("ruby", &[&script]).with_env("GEM_HOME", gem_home),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> Dirs {
        Dirs {
            block: PathBuf::from("/tmp/dx-block"),
            toolchains: PathBuf::from("/tmp/dx-toolchains"),
        }
    }

    #[test]
    fn deps_split_on_commas_and_whitespace() {
        assert_eq!(parse_deps("requests, rich"), vec!["requests", "rich"]);
        assert_eq!(parse_deps("  a\tb\nc "), vec!["a", "b", "c"]);
        assert!(parse_deps("   ").is_empty());
    }

    #[test]
    fn python_plans_write_the_script_and_run_it() {
        let plan = build("python", "print(1)", &[], &dirs()).expect("python plan");
        assert_eq!(plan.files[0].0, "block.py");
        assert!(plan.run.display().contains("block.py"));
        // Nothing to install, so nothing needs the network at all.
        assert!(plan.setup.is_empty());
    }

    /// The rule the whole sandbox rests on: fetching happens in `setup`, which is the only
    /// phase given the network. A `run` command that would have to download something is a
    /// block that fails offline — or, worse, a reason to hand the network back to it.
    #[test]
    fn every_language_fetches_in_setup_and_never_in_the_run() {
        // Whole arguments, not substrings: `--cached-only` is deno being told the opposite
        // — use what is already on disk and fail rather than reach for the network.
        let fetching = [
            "install", "get", "add", "cache", "fetch", "download", "sync",
        ];
        for runner in ["python", "node", "deno", "bash", "rust", "go", "ruby"] {
            let Ok(plan) = build(runner, "x", &["some-dep".into()], &dirs()) else {
                continue; // Toolchain absent on this machine; nothing to check.
            };
            for argument in &plan.run.args {
                assert!(
                    !fetching.contains(&argument.as_str()),
                    "{runner} would fetch during its run: {}",
                    plan.run.display()
                );
            }
        }
    }

    #[test]
    fn python_dependencies_are_installed_into_a_venv_the_run_then_uses() {
        let plan = build("python", "import rich", &["rich".into()], &dirs())
            .expect("python plan with deps");
        assert!(
            plan.setup
                .iter()
                .any(|step| step.display().contains("rich")),
            "dependency never gets installed"
        );
        assert!(
            plan.run.display().contains(".venv"),
            "{}",
            plan.run.display()
        );
    }

    #[test]
    fn cargo_manifest_pins_versions_when_asked() {
        let manifest = cargo_manifest(&["serde=1.0".into(), "rand".into()]);
        assert!(manifest.contains("serde = \"1.0\""));
        assert!(manifest.contains("rand = \"*\""));
    }

    /// A compiled language builds in setup and runs the artifact, so the run needs neither a
    /// compiler nor a network.
    #[test]
    fn compiled_languages_run_the_binary_they_built() {
        if have("cargo") {
            let plan = build("rust", "fn main() {}", &[], &dirs()).expect("rust plan");
            assert!(plan.setup[0].display().contains("build"));
            assert!(plan.run.display().contains("dx-block"));
            assert!(!plan.run.display().contains("cargo"));
        }
        if have("go") {
            let plan = build("go", "package main\nfunc main() {}", &[], &dirs()).expect("go plan");
            assert!(plan
                .setup
                .last()
                .expect("build step")
                .display()
                .contains("build"));
            assert!(!plan.run.display().contains("go build"));
        }
    }

    /// Every toolchain's downloads land in a directory dx owns. The default is under the
    /// reader's home, which the sandbox keeps read-only — a block would fail on a cache miss.
    #[test]
    fn toolchain_caches_are_pointed_away_from_the_readers_home() {
        for (runner, variable) in [
            ("python", "UV_CACHE_DIR"),
            ("node", "npm_config_cache"),
            ("rust", "CARGO_HOME"),
            ("go", "GOMODCACHE"),
        ] {
            let Ok(plan) = build(runner, "x", &["dep".into()], &dirs()) else {
                continue;
            };
            let named = plan.setup.iter().any(|step| {
                step.env
                    .iter()
                    .any(|(key, value)| key == variable && value.starts_with("/tmp/dx-toolchains"))
            });
            // uv may be absent, in which case python uses PIP_CACHE_DIR instead.
            let alternative = plan.setup.iter().any(|step| {
                step.env
                    .iter()
                    .any(|(_, value)| value.starts_with("/tmp/dx-toolchains"))
            });
            assert!(named || alternative, "{runner} keeps its cache in $HOME");
        }
    }

    #[test]
    fn deno_no_longer_runs_with_every_permission_granted() {
        if have("deno") {
            let plan = build("deno", "console.log(1)", &[], &dirs()).expect("deno plan");
            assert!(!plan.run.display().contains("--allow-all"));
            assert!(!plan.run.display().contains("--allow-write"));
            assert!(!plan.run.display().contains("--allow-net"));
        }
    }

    #[test]
    fn an_unknown_runner_is_an_error_not_a_panic() {
        assert!(build("cobol", "x", &[], &dirs()).is_err());
    }
}
