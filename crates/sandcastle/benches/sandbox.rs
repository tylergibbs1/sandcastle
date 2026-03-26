use std::sync::Arc;
use std::time::{Duration, Instant};

use sandcastle::capability::CapabilityRegistry;
use sandcastle::limits::Limits;
use sandcastle::runtime::{Config, SandCastle};
use sandcastle::sandbox::ExecutionRequest;

fn load_guest_module() -> Vec<u8> {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let candidates = [
        workspace_root
            .join("guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm")
            .to_string_lossy()
            .to_string(),
        "sandcastle-guest-js.wasm".to_string(),
    ];
    for path in &candidates {
        if let Ok(bytes) = std::fs::read(&path) {
            return bytes;
        }
    }
    panic!(
        "Guest WASM module not found. Build with: cd guest && ./build.sh\nTried: {:?}",
        candidates
    );
}

fn bench_sandbox_creation(runtime: &SandCastle, iterations: usize) -> Duration {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let start = Instant::now();

    for _ in 0..iterations {
        rt.block_on(async {
            let request = ExecutionRequest::new("return null;");
            let _ = runtime.execute(request).await;
        });
    }

    start.elapsed()
}

fn bench_simple_execution(runtime: &SandCastle, iterations: usize) -> Duration {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let start = Instant::now();

    for _ in 0..iterations {
        rt.block_on(async {
            let request = ExecutionRequest::new("return 1 + 1;");
            let _ = runtime.execute(request).await;
        });
    }

    start.elapsed()
}

fn bench_json_processing(runtime: &SandCastle, iterations: usize) -> Duration {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let input = serde_json::json!({
        "items": (0..100).map(|i| serde_json::json!({
            "id": i,
            "name": format!("item_{i}"),
            "value": i * 10
        })).collect::<Vec<_>>()
    });

    let code = r#"
        const data = globalThis.__sandcastle_input;
        const result = data.items
            .filter(item => item.value > 500)
            .map(item => ({ ...item, doubled: item.value * 2 }));
        return { count: result.length, items: result };
    "#;

    let start = Instant::now();

    for _ in 0..iterations {
        rt.block_on(async {
            let request = ExecutionRequest::new(code).with_input(input.clone());
            let _ = runtime.execute(request).await;
        });
    }

    start.elapsed()
}

fn bench_concurrent_sandboxes(runtime: Arc<SandCastle>, count: usize) -> Duration {
    let rt = tokio::runtime::Runtime::new().unwrap();

    let start = Instant::now();

    rt.block_on(async {
        let mut handles = Vec::new();
        for i in 0..count {
            let rt = runtime.clone();
            handles.push(tokio::spawn(async move {
                let code = format!("return {{ id: {i} }};");
                let request = ExecutionRequest::new(code);
                rt.execute(request).await
            }));
        }
        for handle in handles {
            let _ = handle.await;
        }
    });

    start.elapsed()
}

fn main() {
    let guest_module = load_guest_module();
    let config = Config::new(guest_module);
    let runtime = Arc::new(SandCastle::new(config).expect("Failed to create runtime"));

    println!("SandCastle Benchmark Suite");
    println!("=========================\n");

    // Warmup
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let _ = runtime.execute(ExecutionRequest::new("return null;")).await;
    });

    // Sandbox creation + minimal execution
    let iterations = 1000;
    let elapsed = bench_sandbox_creation(&runtime, iterations);
    let per_op = elapsed / iterations as u32;
    println!(
        "Sandbox creation + minimal exec ({iterations} iterations):"
    );
    println!("  Total: {elapsed:?}");
    println!("  Per operation: {per_op:?}");
    println!("  Ops/sec: {:.0}", iterations as f64 / elapsed.as_secs_f64());
    println!();

    // Simple expression evaluation
    let elapsed = bench_simple_execution(&runtime, iterations);
    let per_op = elapsed / iterations as u32;
    println!("Simple expression eval ({iterations} iterations):");
    println!("  Total: {elapsed:?}");
    println!("  Per operation: {per_op:?}");
    println!("  Ops/sec: {:.0}", iterations as f64 / elapsed.as_secs_f64());
    println!();

    // JSON processing
    let elapsed = bench_json_processing(&runtime, 100);
    let per_op = elapsed / 100;
    println!("JSON processing (100 iterations, 100 items each):");
    println!("  Total: {elapsed:?}");
    println!("  Per operation: {per_op:?}");
    println!();

    // Concurrent sandboxes
    for count in [10, 100, 500] {
        let elapsed = bench_concurrent_sandboxes(runtime.clone(), count);
        println!("Concurrent sandboxes ({count}):");
        println!("  Total: {elapsed:?}");
        println!(
            "  Per sandbox: {:?}",
            elapsed / count as u32
        );
        println!();
    }
}
