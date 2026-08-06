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
        "typescript" => typescript(code, deps, dirs),
        "deno" => deno(code, dirs),
        "bash" => bash(code, dirs),
        "rust" => rust(code, deps, dirs),
        "go" => go(code, deps, dirs),
        "ruby" => ruby(code, deps, dirs),
        "c" => c_family(code, deps, dirs, CFamily::C),
        "cpp" => c_family(code, deps, dirs, CFamily::Cpp),
        "java" => java(code, deps, dirs),
        "swift" => swift(code, deps, dirs),
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

/// TypeScript via the machine's own Node toolchain: setup installs `typescript`, `tsx`,
/// and the block's declared packages into the block directory with npm; the run is
/// `node --import tsx block.ts`, offline, against what setup already fetched.
///
/// `tsx` over `ts-node` is deliberate: it runs both ESM and CJS TypeScript with zero
/// configuration (its esbuild transform needs no `tsconfig.json`, no `--esm` flag, and no
/// loader-hook incantations that shift between Node versions), which is what a code block
/// pasted into a document needs. `typescript` itself is installed alongside so `tsc` and
/// its APIs are present for blocks that want them. Types are stripped, not checked — the
/// block runs the way `node --import tsx script.ts` would in a terminal.
///
/// Loader mode rather than the `tsx` CLI, on evidence: the CLI wraps the script in a
/// process manager that listens on a unix socket under `$TMPDIR`, and the sandbox points
/// `$TMPDIR` at the block directory — a path long enough to overflow the kernel's socket
/// path limit. The one-line `boot.mjs` exists because `--import`'s bare `tsx` specifier
/// resolves from the *importing file*, and boot sits beside the block's own
/// `node_modules`; the script does too, so every installed package resolves for ESM and
/// CJS alike, with `NODE_PATH` set as well, matching the JavaScript runner.
fn typescript(code: &str, deps: &[String], dirs: &Dirs) -> Result<Plan, String> {
    // Both are required; the sentence names the first one actually absent — telling a
    // machine that already has node to install node sends its owner nowhere.
    for tool in ["node", "npm"] {
        if !have(tool) {
            return Err(missing_toolchain_message("typescript", &[tool]));
        }
    }
    let script = dirs.block_path("block.ts");
    let boot = dirs.block_path("boot.mjs");
    let modules = dirs.block_path("node_modules");
    let files = vec![
        ("block.ts".to_string(), code.to_string()),
        ("boot.mjs".to_string(), "import \"tsx\";\n".to_string()),
        (
            "package.json".to_string(),
            "{\n  \"name\": \"dx-block\",\n  \"private\": true,\n  \"type\": \"module\"\n}\n"
                .to_string(),
        ),
    ];

    let mut install = CommandSpec::new(
        "npm",
        &[
            "install",
            "--silent",
            "--no-audit",
            "--no-fund",
            "--save",
            "typescript",
            "tsx",
        ],
    );
    install.args.extend(deps.iter().cloned());

    Ok(Plan {
        files,
        setup: vec![with_cache(install, "npm_config_cache", dirs.cache("npm"))],
        run: CommandSpec::new("node", &["--import", &boot, &script]).with_env("NODE_PATH", modules),
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

/// Which of the two C-family languages a block speaks; decides compiler, file name, and
/// language standard.
#[derive(Debug, Clone, Copy)]
enum CFamily {
    /// C, compiled by the first of `cc`, `clang`, `gcc`.
    C,
    /// C++, compiled by the first of `c++`, `clang++`, `g++`.
    Cpp,
}

impl CFamily {
    /// Compiler candidates, in preference order — the platform alias first, so the block
    /// uses whatever the machine considers its C compiler.
    fn compilers(self) -> &'static [&'static str] {
        match self {
            CFamily::C => &["cc", "clang", "gcc"],
            CFamily::Cpp => &["c++", "clang++", "g++"],
        }
    }

    /// Name the language is known by, for messages.
    fn name(self) -> &'static str {
        match self {
            CFamily::C => "c",
            CFamily::Cpp => "c++",
        }
    }

    /// Source file the code is written to.
    fn source_file(self) -> &'static str {
        match self {
            CFamily::C => "block.c",
            CFamily::Cpp => "block.cpp",
        }
    }
}

/// C and C++: the system compiler builds the block in setup, and the run is the binary.
///
/// The compiler links against what the machine already provides (`-lm` and friends via the
/// standard library); there is no package fetch, so `deps=` is refused with a sentence
/// rather than half-supported — a C library cannot be installed in a way that keeps the
/// run offline and the reader's system untouched.
fn c_family(code: &str, deps: &[String], dirs: &Dirs, family: CFamily) -> Result<Plan, String> {
    if !deps.is_empty() {
        return Err(deps_unsupported(family.name()));
    }
    let compiler = first_available(family.compilers())
        .ok_or_else(|| missing_toolchain_message(family.name(), family.compilers()))?;
    let source = dirs.block_path(family.source_file());
    let binary = dirs.block_path(&format!("dx-block{}", exe_suffix()));
    Ok(Plan {
        files: vec![(family.source_file().to_string(), code.to_string())],
        setup: vec![compile_env(
            CommandSpec::new(compiler, &["-O2", "-o", &binary, &source]),
            dirs,
        )],
        run: CommandSpec::new(binary, &[]),
    })
}

/// Point a compiler's scratch space into the block directory.
///
/// A compile step runs inside the same sandbox as everything else in setup, and system
/// compilers write intermediates (and, on macOS, an `xcrun` cache) into `$TMPDIR` — which
/// still names the reader's real temp directory during setup, where the sandbox refuses
/// writes. The block directory is the one place a compile is allowed to scribble.
fn compile_env(spec: CommandSpec, dirs: &Dirs) -> CommandSpec {
    let block = dirs.block.to_string_lossy().into_owned();
    spec.with_env("TMPDIR", block.clone())
        .with_env("TEMP", block.clone())
        .with_env("HOME", block)
}

/// The sentence refusing `deps=` for a language whose libraries cannot be fetched into an
/// offline run.
fn deps_unsupported(language: &str) -> String {
    format!(
        "`deps=` is not supported for {language} blocks — this runner compiles against \
         what the system toolchain already provides. Remove the deps attribute, or use a \
         language with a package manager (python, node, typescript, rust, go, ruby)."
    )
}

/// Java: `javac` compiles the block in setup, and `java` runs the classes offline.
///
/// The block's entry point must be a class named `Main` with a `public static void main`
/// — the source is written as `Main.java`, which is also what the Java language requires
/// of a public class. `deps=` is refused: fetching jars would need a build tool this
/// runner does not impose.
fn java(code: &str, deps: &[String], dirs: &Dirs) -> Result<Plan, String> {
    if !deps.is_empty() {
        return Err(deps_unsupported("java"));
    }
    let jdk = locate_jdk().ok_or_else(|| missing_toolchain_message("java", &["javac"]))?;
    let source = dirs.block_path("Main.java");
    let classes = dirs.block_path("classes");
    Ok(Plan {
        files: vec![("Main.java".to_string(), code.to_string())],
        setup: vec![compile_env(
            CommandSpec::new(jdk.javac, &["-d", &classes, &source]),
            dirs,
        )],
        run: CommandSpec::new(jdk.java, &["-cp", &classes, "Main"]),
    })
}

/// The `javac`/`java` pair a block will actually run.
struct Jdk {
    /// Compiler invoked in setup.
    javac: String,
    /// Runtime invoked offline in the run.
    java: String,
}

/// Find a working JDK, seeing through macOS's forwarding stubs.
///
/// A bare path probe is not enough on macOS: every machine ships stub `javac`/`java`
/// binaries in `/usr/bin` that forward to an installed JDK through a framework lookup the
/// sandbox denies — and fail with a "visit java.com" message when no JDK exists at all.
/// So when the probe lands on the stub, the real JDK is resolved here, outside the
/// sandbox, by the same `/usr/libexec/java_home` the stub consults, and the plan names
/// that JDK's own binaries. This runs a resolver process, which is fine where it is
/// called: building a plan is already part of executing a block, never of reading one.
fn locate_jdk() -> Option<Jdk> {
    let javac = crate::toolchain::locate("javac")?;
    if !have("java") {
        return None;
    }
    if !(cfg!(target_os = "macos") && javac == std::path::Path::new("/usr/bin/javac")) {
        return Some(Jdk {
            javac: "javac".to_string(),
            java: "java".to_string(),
        });
    }
    let resolved = std::process::Command::new("/usr/libexec/java_home")
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let home = PathBuf::from(String::from_utf8_lossy(&resolved.stdout).trim());
    let javac = home.join("bin/javac");
    let java = home.join("bin/java");
    (javac.is_file() && java.is_file()).then(|| Jdk {
        javac: javac.to_string_lossy().into_owned(),
        java: java.to_string_lossy().into_owned(),
    })
}

/// Whether a working JDK is installed — the guard toolchain-dependent tests share.
#[cfg(test)]
pub(crate) fn java_toolchain_present() -> bool {
    locate_jdk().is_some()
}

/// Swift: `swiftc` builds the block in setup, and the run is the binary it produced.
///
/// Present wherever Xcode's command line tools are (and on Linux Swift installs). `deps=`
/// is refused: Swift packages resolve through SwiftPM projects, which is more machinery
/// than a code block should carry.
fn swift(code: &str, deps: &[String], dirs: &Dirs) -> Result<Plan, String> {
    if !deps.is_empty() {
        return Err(deps_unsupported("swift"));
    }
    if !have("swiftc") {
        return Err(missing_toolchain_message("swift", &["swiftc"]));
    }
    let source = dirs.block_path("block.swift");
    let binary = dirs.block_path(&format!("dx-block{}", exe_suffix()));
    // The module cache is pointed into the block directory: its default is under the
    // reader's home, which the sandbox keeps read-only even during setup.
    let module_cache = dirs.block_path("module-cache");
    Ok(Plan {
        files: vec![("block.swift".to_string(), code.to_string())],
        setup: vec![compile_env(
            CommandSpec::new(
                "swiftc",
                &[
                    "-O",
                    "-module-cache-path",
                    &module_cache,
                    "-o",
                    &binary,
                    &source,
                ],
            ),
            dirs,
        )],
        run: CommandSpec::new(binary, &[]),
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
        for runner in [
            "python",
            "node",
            "typescript",
            "deno",
            "bash",
            "rust",
            "go",
            "ruby",
            "c",
            "cpp",
            "java",
            "swift",
        ] {
            // Both shapes: with dependencies, and without. A runner that refuses `deps`
            // (the compiled ones do) would otherwise skip this check entirely — leaving the
            // one test that enforces the offline run silent about exactly those languages.
            for deps in [vec!["some-dep".to_string()], Vec::new()] {
                let Ok(plan) = build(runner, "x", &deps, &dirs()) else {
                    continue; // Toolchain absent (or deps refused) on this machine.
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
            ("typescript", "npm_config_cache"),
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

    /// `lang=ts` runs on the machine's own Node toolchain now — setup installs `tsx` (and
    /// `typescript`) with npm, and the run is the installed launcher, never deno.
    #[test]
    fn typescript_installs_tsx_in_setup_and_runs_it_offline() {
        if !(have("node") && have("npm")) {
            eprintln!("skipping: node/npm not installed");
            return;
        }
        let plan = build("typescript", "console.log(1)", &[], &dirs()).expect("typescript plan");
        let install = plan.setup[0].display();
        assert!(install.contains("npm"), "{install}");
        assert!(install.contains("tsx"), "{install}");
        assert!(install.contains("typescript"), "{install}");
        assert_eq!(plan.run.program, "node");
        assert!(plan.run.display().contains("--import"));
        assert!(plan.run.display().contains("block.ts"));
        assert!(!plan.run.display().contains("deno"));
    }

    /// Declared packages ride the same npm install as the runner itself.
    #[test]
    fn typescript_dependencies_join_the_npm_install() {
        if !(have("node") && have("npm")) {
            eprintln!("skipping: node/npm not installed");
            return;
        }
        let plan = build("typescript", "import 'chalk'", &["chalk".into()], &dirs())
            .expect("typescript plan with deps");
        assert!(plan.setup[0].display().contains("chalk"));
        assert!(plan.run.args.iter().all(|argument| argument != "chalk"));
    }

    /// Each direct-toolchain language compiles in setup and runs its artifact — the run
    /// names neither a compiler nor a fetch.
    #[test]
    fn direct_toolchain_languages_compile_in_setup_and_run_the_artifact() {
        for (runner, compilers) in [
            ("c", &["cc", "clang", "gcc"][..]),
            ("cpp", &["c++", "clang++", "g++"][..]),
            ("java", &["javac"][..]),
            ("swift", &["swiftc"][..]),
        ] {
            let available = if runner == "java" {
                java_toolchain_present()
            } else {
                first_available(compilers).is_some()
            };
            if !available {
                eprintln!("skipping {runner}: no toolchain installed");
                continue;
            }
            let plan = build(runner, "x", &[], &dirs()).expect(runner);
            assert_eq!(plan.setup.len(), 1, "{runner} compiles once in setup");
            for compiler in compilers {
                assert!(
                    !plan.run.display().contains(compiler),
                    "{runner} runs its compiler: {}",
                    plan.run.display()
                );
            }
        }
    }

    /// A language whose libraries cannot be fetched into an offline run refuses `deps=`
    /// with a sentence, instead of half-supporting them.
    #[test]
    fn compiled_system_languages_refuse_deps_with_a_sentence() {
        for runner in ["c", "cpp", "java", "swift"] {
            let refusal =
                build(runner, "x", &["somelib".into()], &dirs()).expect_err("deps must be refused");
            assert!(refusal.contains("deps="), "{runner}: {refusal}");
            assert!(refusal.contains("not supported"), "{runner}: {refusal}");
        }
    }

    /// A missing toolchain is a sentence naming what to install, never a panic.
    #[test]
    fn a_missing_direct_toolchain_is_named_in_the_error() {
        // swift is the likeliest to be absent; when present, the message path is still
        // covered by the unknown-runner and deps-refusal tests above.
        if have("swiftc") {
            return;
        }
        let message = build("swift", "print(1)", &[], &dirs()).expect_err("no swiftc");
        assert!(message.contains("swiftc"));
        assert!(message.contains("PATH"));
    }
}
