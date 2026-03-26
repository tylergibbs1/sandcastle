mod serve;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use sandcastle::capability::{CapabilityRegistry, MethodSchema, TypeScriptGenerator};
use sandcastle::limits::Limits;
use sandcastle::namespace::NamespaceManager;
use sandcastle::registry::ScriptRegistry;
use sandcastle::runtime::{Config, SandCastle};
use sandcastle::sandbox::ExecutionRequest;
use sandcastle::types::Artifact;
use tracing_subscriber::EnvFilter;

#[cfg(feature = "builtins")]
use sandcastle::capabilities::HttpCapability;

#[derive(Parser)]
#[command(name = "sandcastle", version, about = "Lightweight agent sandbox runtime")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Execute a JavaScript file in a sandbox
    Run {
        /// Path to the JavaScript file
        script: PathBuf,

        /// JSON input to pass to the script
        #[arg(short, long)]
        input: Option<String>,

        /// JSON input from a file
        #[arg(long)]
        input_file: Option<PathBuf>,

        /// Memory limit in MB
        #[arg(long, default_value = "32")]
        memory_mb: u32,

        /// Timeout in seconds
        #[arg(long, default_value = "10")]
        timeout: u64,

        /// Fuel limit (instruction count, 0 = unlimited)
        #[arg(long, default_value = "1000000000")]
        fuel: u64,

        /// Input artifact files (name=path)
        #[arg(long = "artifact")]
        artifacts: Vec<String>,

        /// Path to the guest WASM module (env: SANDCASTLE_GUEST_MODULE)
        #[arg(long)]
        guest_module: Option<PathBuf>,

        /// Output the execution transcript as JSON
        #[arg(long)]
        transcript: bool,

        /// Allow HTTP capability (comma-separated allowed domains, or * for all)
        #[arg(long)]
        allow_http: Option<String>,

        /// Environment variables to inject into process.env (KEY=VALUE)
        #[arg(long = "env", short = 'e')]
        env_vars: Vec<String>,
    },

    /// Start the HTTP server (dispatch mode)
    Serve {
        /// HTTP listen address
        #[arg(long, default_value = "0.0.0.0:8080")]
        http: String,

        /// Path to the guest WASM module
        #[arg(long)]
        guest_module: Option<PathBuf>,

        /// Allow HTTP capability for sandboxed code
        #[arg(long)]
        allow_http: Option<String>,

        /// Watch a directory for .js scripts and auto-register
        #[arg(long)]
        watch: Option<PathBuf>,
    },

    /// Print version and runtime info
    Info,

    /// Initialize a new SandCastle project
    Init {
        /// Project directory (default: current directory)
        #[arg(default_value = ".")]
        dir: PathBuf,
    },

    /// Interactive JavaScript REPL
    Repl {
        #[arg(long)]
        guest_module: Option<PathBuf>,
        #[arg(long, default_value = "64")]
        memory_mb: u32,
    },

    /// Generate TypeScript declarations from capability definitions
    Codegen {
        /// Path to JSON capability definitions file
        input: PathBuf,
        /// Output .d.ts file (default: stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        EnvFilter::new("sandcastle=debug")
    } else {
        EnvFilter::new("sandcastle=info")
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    match cli.command {
        Commands::Run {
            script,
            input,
            input_file,
            memory_mb,
            timeout,
            fuel,
            artifacts,
            guest_module,
            transcript,
            allow_http,
            env_vars,
        } => {
            run_script(
                script, input, input_file, memory_mb, timeout, fuel, artifacts, guest_module,
                transcript, allow_http, env_vars,
            )
            .await
        }
        Commands::Serve {
            http,
            guest_module,
            allow_http,
            watch,
        } => run_serve(http, guest_module, allow_http, watch).await,
        Commands::Info => {
            println!("SandCastle v{}", env!("CARGO_PKG_VERSION"));
            println!("Runtime: Wasmtime 29 + QuickJS-NG (ES2024+)");
            println!("Security: Standard mode (in-process WASM sandbox)");
            println!("Platform: {} {}", std::env::consts::OS, std::env::consts::ARCH);
            Ok(())
        }
        Commands::Init { dir } => run_init(dir),
        Commands::Repl {
            guest_module,
            memory_mb,
        } => run_repl(guest_module, memory_mb).await,
        Commands::Codegen { input, output } => run_codegen(input, output),
    }
}

async fn run_serve(
    http_addr: String,
    guest_module_path: Option<PathBuf>,
    allow_http: Option<String>,
    watch_dir: Option<PathBuf>,
) -> Result<()> {
    let guest_bytes = load_guest_module(guest_module_path)?;
    let config = Config::new(guest_bytes);
    let runtime = SandCastle::new(config)?;

    let mut capabilities = CapabilityRegistry::new();

    #[cfg(feature = "builtins")]
    if let Some(domains) = allow_http {
        let allowed: Vec<String> = if domains == "*" {
            vec![]
        } else {
            domains.split(',').map(|s| s.trim().to_string()).collect()
        };
        capabilities.register(Box::new(HttpCapability::new(allowed, 10 * 1024 * 1024)));
    }

    #[cfg(not(feature = "builtins"))]
    let _ = allow_http;

    let default_caps = Arc::new(capabilities);

    let state = Arc::new(serve::AppState {
        runtime,
        registry: ScriptRegistry::new(10_000),
        namespaces: NamespaceManager::new(1_000),
        default_capabilities: default_caps.clone(),
    });

    // --watch: scan directory for .js files and auto-register, then watch for changes
    if let Some(dir) = watch_dir {
        let dir = std::fs::canonicalize(&dir)
            .with_context(|| format!("watch directory not found: {}", dir.display()))?;

        // Initial scan
        let default_limits = Limits::default();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("js") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    let code = std::fs::read_to_string(&path)?;
                    state
                        .registry
                        .register(stem, code, default_caps.clone(), default_limits.clone())
                        .ok();
                    tracing::info!("watch: registered script '{stem}' from {}", path.display());
                }
            }
        }

        // Spawn watcher task
        let watch_state = state.clone();
        let watch_caps = default_caps.clone();
        let watch_dir_clone = dir.clone();
        tokio::task::spawn_blocking(move || {
            use notify::{Event, EventKind, RecursiveMode, Watcher};

            let (tx, rx) = std::sync::mpsc::channel::<notify::Result<Event>>();
            let mut watcher =
                notify::recommended_watcher(tx).expect("failed to create file watcher");
            watcher
                .watch(&watch_dir_clone, RecursiveMode::NonRecursive)
                .expect("failed to watch directory");

            tracing::info!("watch: monitoring {} for .js changes", watch_dir_clone.display());

            for event in rx {
                let event = match event {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!("watch error: {e}");
                        continue;
                    }
                };

                for path in &event.paths {
                    if path.extension().and_then(|e| e.to_str()) != Some("js") {
                        continue;
                    }

                    let stem = match path.file_stem().and_then(|s| s.to_str()) {
                        Some(s) => s.to_string(),
                        None => continue,
                    };

                    match event.kind {
                        EventKind::Create(_) | EventKind::Modify(_) => {
                            match std::fs::read_to_string(path) {
                                Ok(code) => {
                                    let limits = Limits::default();
                                    watch_state
                                        .registry
                                        .register(
                                            &stem,
                                            code,
                                            watch_caps.clone(),
                                            limits,
                                        )
                                        .ok();
                                    tracing::info!(
                                        "watch: registered/updated script '{stem}'"
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "watch: failed to read {}: {e}",
                                        path.display()
                                    );
                                }
                            }
                        }
                        EventKind::Remove(_) => {
                            watch_state.registry.remove(&stem);
                            tracing::info!("watch: removed script '{stem}'");
                        }
                        _ => {}
                    }
                }
            }
        });
    }

    serve::start(state, &http_addr).await
}

async fn run_script(
    script: PathBuf,
    input: Option<String>,
    input_file: Option<PathBuf>,
    memory_mb: u32,
    timeout: u64,
    fuel: u64,
    artifact_args: Vec<String>,
    guest_module_path: Option<PathBuf>,
    show_transcript: bool,
    allow_http: Option<String>,
    env_vars: Vec<String>,
) -> Result<()> {
    let code = std::fs::read_to_string(&script)
        .with_context(|| format!("Failed to read {}", script.display()))?;

    let input_value = if let Some(input_str) = input {
        serde_json::from_str(&input_str).context("Invalid JSON input")?
    } else if let Some(input_path) = input_file {
        let input_str = std::fs::read_to_string(&input_path)
            .with_context(|| format!("Failed to read input file {}", input_path.display()))?;
        serde_json::from_str(&input_str).context("Invalid JSON in input file")?
    } else {
        serde_json::Value::Null
    };

    let mut artifacts = Vec::new();
    for arg in &artifact_args {
        let (name, path) = arg
            .split_once('=')
            .with_context(|| format!("Invalid artifact format '{arg}', expected name=path"))?;
        let data =
            std::fs::read(path).with_context(|| format!("Failed to read artifact file {path}"))?;
        artifacts.push(Artifact::new(name, data));
    }

    let guest_bytes = load_guest_module(guest_module_path)?;
    let config = Config::new(guest_bytes);
    let runtime = SandCastle::new(config)?;

    let mut registry = CapabilityRegistry::new();

    #[cfg(feature = "builtins")]
    if let Some(domains) = allow_http {
        let allowed: Vec<String> = if domains == "*" {
            vec![]
        } else {
            domains.split(',').map(|s| s.trim().to_string()).collect()
        };
        registry.register(Box::new(HttpCapability::new(allowed, 10 * 1024 * 1024)));
    }

    #[cfg(not(feature = "builtins"))]
    let _ = allow_http;

    let limits = Limits {
        memory_mb,
        timeout: std::time::Duration::from_secs(timeout),
        fuel,
        ..Limits::default()
    };

    let mut env_map = std::collections::HashMap::new();
    for arg in &env_vars {
        let (key, value) = arg
            .split_once('=')
            .with_context(|| format!("Invalid env format '{arg}', expected KEY=VALUE"))?;
        env_map.insert(key.to_string(), value.to_string());
    }

    let request = ExecutionRequest::new(code)
        .with_input(input_value)
        .with_capabilities(Arc::new(registry))
        .with_limits(limits)
        .with_artifacts(artifacts)
        .with_env_map(env_map);

    let result = runtime.execute(request).await?;

    if show_transcript {
        let transcript_json = serde_json::to_string_pretty(&result.transcript)?;
        println!("{transcript_json}");
    } else {
        for msg in &result.transcript.console {
            let prefix = match msg.level {
                sandcastle::types::ConsoleLevel::Log => "",
                sandcastle::types::ConsoleLevel::Warn => "[warn] ",
                sandcastle::types::ConsoleLevel::Error => "[error] ",
                sandcastle::types::ConsoleLevel::Debug => "[debug] ",
            };
            eprintln!("{prefix}{}", msg.message);
        }

        match &result.output {
            sandcastle::types::OutputValue::Json(v) => {
                println!("{}", serde_json::to_string_pretty(v)?);
            }
            sandcastle::types::OutputValue::String(s) => println!("{s}"),
            sandcastle::types::OutputValue::Bytes(b) => {
                use std::io::Write;
                std::io::stdout().write_all(b)?;
            }
            sandcastle::types::OutputValue::Null => {}
        }

        if !result.is_success() {
            bail!("Execution failed: {:?}", result.status);
        }
    }

    Ok(())
}

fn run_init(dir: PathBuf) -> Result<()> {
    // Create directory structure
    std::fs::create_dir_all(dir.join("scripts"))?;
    std::fs::create_dir_all(dir.join("types"))?;

    // Write sandcastle.config.json
    let config = serde_json::json!({
        "guest_module": "guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm",
        "defaults": {
            "memory_mb": 32,
            "timeout_s": 10,
            "fuel": 1_000_000_000u64,
        }
    });
    let config_path = dir.join("sandcastle.config.json");
    if !config_path.exists() {
        std::fs::write(&config_path, serde_json::to_string_pretty(&config)?)?;
    }

    // Write scripts/hello.js
    let hello_js = r#"// hello.js — a simple SandCastle script
const name = typeof input !== "undefined" && input && input.name ? input.name : "world";
console.log("Hello, " + name + "!");
"Hello from SandCastle!";
"#;
    let hello_path = dir.join("scripts/hello.js");
    if !hello_path.exists() {
        std::fs::write(&hello_path, hello_js)?;
    }

    // Write types/sandbox.d.ts
    let dts = r#"// SandCastle guest type declarations
// Auto-generated — do not edit manually.

/** JSON input passed to the script via --input / request body. */
declare const input: any;

/** Standard console for logging (log, warn, error, debug). */
declare const console: {
  log(...args: any[]): void;
  warn(...args: any[]): void;
  error(...args: any[]): void;
  debug(...args: any[]): void;
};

/**
 * Low-level host call bridge.
 * Prefer capability-specific APIs when available.
 */
declare function __sandcastle_host_call(
  capability: string,
  method: string,
  input: any,
): any;
"#;
    let dts_path = dir.join("types/sandbox.d.ts");
    if !dts_path.exists() {
        std::fs::write(&dts_path, dts)?;
    }

    println!("Initialized SandCastle project in {}", dir.display());
    println!();
    println!("  sandcastle.config.json   — project configuration");
    println!("  scripts/hello.js         — example script");
    println!("  types/sandbox.d.ts       — TypeScript declarations for the sandbox");
    println!();
    println!("Get started:");
    println!("  sandcastle run scripts/hello.js");
    println!("  sandcastle run scripts/hello.js --input '{{\"name\": \"Alice\"}}'");
    println!("  sandcastle serve --watch scripts/");

    Ok(())
}

async fn run_repl(guest_module_path: Option<PathBuf>, memory_mb: u32) -> Result<()> {
    use std::io::{BufRead, Write};

    let guest_bytes = load_guest_module(guest_module_path)?;
    let config = Config::new(guest_bytes);
    let runtime = SandCastle::new(config)?;
    let capabilities = Arc::new(CapabilityRegistry::new());

    println!("SandCastle REPL v{}", env!("CARGO_PKG_VERSION"));
    println!("Type JavaScript expressions. Ctrl-D to exit.");
    println!();

    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let mut stdout = std::io::stdout();

    loop {
        print!("sandcastle> ");
        stdout.flush()?;

        let mut buffer = String::new();
        let mut brace_depth: i32 = 0;

        // Read first line
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // EOF (Ctrl-D)
                println!();
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }

        buffer.push_str(&line);
        brace_depth += count_braces(&line);

        // Multi-line: keep reading while braces are unclosed
        while brace_depth > 0 {
            print!("...> ");
            stdout.flush()?;
            let mut cont_line = String::new();
            match reader.read_line(&mut cont_line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(_) => break,
            }
            brace_depth += count_braces(&cont_line);
            buffer.push_str(&cont_line);
        }

        let code = buffer.trim();
        if code.is_empty() {
            continue;
        }

        let limits = Limits {
            memory_mb,
            timeout: std::time::Duration::from_secs(10),
            fuel: 10_000_000,
            ..Limits::default()
        };

        let request = ExecutionRequest::new(code)
            .with_capabilities(capabilities.clone())
            .with_limits(limits);

        match runtime.execute(request).await {
            Ok(result) => {
                // Print console output
                for msg in &result.transcript.console {
                    let prefix = match msg.level {
                        sandcastle::types::ConsoleLevel::Log => "",
                        sandcastle::types::ConsoleLevel::Warn => "[warn] ",
                        sandcastle::types::ConsoleLevel::Error => "[error] ",
                        sandcastle::types::ConsoleLevel::Debug => "[debug] ",
                    };
                    println!("{prefix}{}", msg.message);
                }

                // Print result
                match &result.output {
                    sandcastle::types::OutputValue::Json(v) => {
                        println!("{}", serde_json::to_string_pretty(v)?);
                    }
                    sandcastle::types::OutputValue::String(s) => println!("{s}"),
                    sandcastle::types::OutputValue::Bytes(b) => {
                        println!("<{} bytes>", b.len());
                    }
                    sandcastle::types::OutputValue::Null => {}
                }
            }
            Err(e) => {
                eprintln!("Error: {e}");
            }
        }
    }

    Ok(())
}

fn count_braces(s: &str) -> i32 {
    let mut depth = 0i32;
    for ch in s.chars() {
        match ch {
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            _ => {}
        }
    }
    depth
}

fn run_codegen(input_path: PathBuf, output: Option<PathBuf>) -> Result<()> {
    use async_trait::async_trait;
    use sandcastle::capability::Capability;

    let json_str = std::fs::read_to_string(&input_path)
        .with_context(|| format!("Failed to read {}", input_path.display()))?;

    #[derive(serde::Deserialize)]
    struct CapabilityDef {
        name: String,
        methods: Vec<MethodSchema>,
    }

    #[derive(serde::Deserialize)]
    struct CodegenInput {
        capabilities: Vec<CapabilityDef>,
    }

    let defs: CodegenInput =
        serde_json::from_str(&json_str).context("Failed to parse capability definitions JSON")?;

    // Build a capability registry from the definitions
    let mut registry = CapabilityRegistry::new();

    for cap_def in defs.capabilities {
        struct StubCapability {
            cap_name: String,
            cap_methods: Vec<MethodSchema>,
        }

        #[async_trait]
        impl Capability for StubCapability {
            fn name(&self) -> &str {
                &self.cap_name
            }

            fn methods(&self) -> Vec<MethodSchema> {
                self.cap_methods.clone()
            }

            async fn call(
                &self,
                _method: &str,
                _input: serde_json::Value,
            ) -> std::result::Result<serde_json::Value, sandcastle::CapabilityError> {
                unreachable!("stub capability should not be called")
            }
        }

        registry.register(Box::new(StubCapability {
            cap_name: cap_def.name,
            cap_methods: cap_def.methods,
        }));
    }

    let ts_output = TypeScriptGenerator::generate(&registry);

    match output {
        Some(path) => {
            std::fs::write(&path, &ts_output)
                .with_context(|| format!("Failed to write {}", path.display()))?;
            println!("Wrote TypeScript declarations to {}", path.display());
        }
        None => {
            print!("{ts_output}");
        }
    }

    Ok(())
}

fn load_guest_module(path: Option<PathBuf>) -> Result<Vec<u8>> {
    if let Some(p) = path {
        return std::fs::read(&p)
            .with_context(|| format!("Failed to read guest module {}", p.display()));
    }

    if let Ok(env_path) = std::env::var("SANDCASTLE_GUEST_MODULE") {
        let p = PathBuf::from(&env_path);
        return std::fs::read(&p).with_context(|| {
            format!(
                "Failed to read guest module from SANDCASTLE_GUEST_MODULE={env_path}"
            )
        });
    }

    let candidates = [
        PathBuf::from("guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm"),
        PathBuf::from("guest/target/wasm32-wasip2/release/sandcastle_guest_js.wasm"),
        PathBuf::from("sandcastle-guest-js.wasm"),
        PathBuf::from("/usr/local/share/sandcastle/guest-js.wasm"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return std::fs::read(candidate)
                .with_context(|| format!("Failed to read guest module {}", candidate.display()));
        }
    }

    bail!(
        "Guest WASM module not found. Tried:\n{}\n\
         Set SANDCASTLE_GUEST_MODULE or use --guest-module to specify the path.\n\
         Build the guest with: cd guest && cargo build --target wasm32-wasip1 --release",
        candidates
            .iter()
            .map(|p| format!("  - {}", p.display()))
            .collect::<Vec<_>>()
            .join("\n")
    )
}
