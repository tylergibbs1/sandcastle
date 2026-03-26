mod serve;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use sandcastle::capability::CapabilityRegistry;
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

        /// Fuel limit (instruction count)
        #[arg(long, default_value = "10000000")]
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
    },

    /// Print version and runtime info
    Info,
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
        } => {
            run_script(
                script, input, input_file, memory_mb, timeout, fuel, artifacts, guest_module,
                transcript, allow_http,
            )
            .await
        }
        Commands::Serve {
            http,
            guest_module,
            allow_http,
        } => run_serve(http, guest_module, allow_http).await,
        Commands::Info => {
            println!("SandCastle v{}", env!("CARGO_PKG_VERSION"));
            println!("Runtime: Wasmtime + QuickJS (WASM)");
            println!("Security: Standard mode (in-process)");
            println!("Platform: {} {}", std::env::consts::OS, std::env::consts::ARCH);
            Ok(())
        }
    }
}

async fn run_serve(
    http_addr: String,
    guest_module_path: Option<PathBuf>,
    allow_http: Option<String>,
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

    let state = Arc::new(serve::AppState {
        runtime,
        registry: ScriptRegistry::new(10_000),
        namespaces: NamespaceManager::new(1_000),
        default_capabilities: Arc::new(capabilities),
    });

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

    let request = ExecutionRequest::new(code)
        .with_input(input_value)
        .with_capabilities(Arc::new(registry))
        .with_limits(limits)
        .with_artifacts(artifacts);

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
