use std::sync::Arc;
use std::time::{Duration, Instant};

use sandcastle::capability::{CapabilityRegistry, SimpleCapability};
use sandcastle::limits::{CapabilityLimits, Limits};
use sandcastle::namespace::{NamespaceLimits, NamespaceManager};
use sandcastle::registry::ScriptRegistry;
use sandcastle::runtime::{Config, SandCastle};
use sandcastle::sandbox::ExecutionRequest;
use sandcastle::types::*;

fn load_guest_module() -> Vec<u8> {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let path = workspace_root.join("guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm");
    std::fs::read(&path).unwrap_or_else(|_| panic!("Guest WASM not found at {:?}", path))
}

fn create_runtime() -> SandCastle {
    SandCastle::new(Config::new(load_guest_module())).expect("Failed to create runtime")
}

fn create_runtime_with_concurrency(max: usize) -> SandCastle {
    let mut config = Config::new(load_guest_module());
    config.max_concurrent_sandboxes = max;
    SandCastle::new(config).expect("Failed to create runtime")
}

// ============================================================================
// HELPERS
// ============================================================================

struct ExperimentResult {
    id: usize,
    category: &'static str,
    name: String,
    status: &'static str,
    details: Vec<String>,
}

impl ExperimentResult {
    fn print(&self) {
        let marker = match self.status {
            "PASS" => "\x1b[32mPASS\x1b[0m",
            "FAIL" => "\x1b[31mFAIL\x1b[0m",
            "WARN" => "\x1b[33mWARN\x1b[0m",
            "INFO" => "\x1b[36mINFO\x1b[0m",
            _ => self.status,
        };
        println!(
            "  [{:>2}] [{}] [{}] {}",
            self.id, marker, self.category, self.name
        );
        for detail in &self.details {
            println!("       {}", detail);
        }
    }
}

async fn bench_iterations(
    runtime: &SandCastle,
    code: &str,
    input: Option<serde_json::Value>,
    limits: Option<Limits>,
    iterations: usize,
) -> (Duration, Vec<Duration>) {
    let mut times = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let mut req = ExecutionRequest::new(code);
        if let Some(ref inp) = input {
            req = req.with_input(inp.clone());
        }
        if let Some(ref lim) = limits {
            req = req.with_limits(lim.clone());
        }
        let start = Instant::now();
        let _ = runtime.execute(req).await;
        times.push(start.elapsed());
    }
    let total: Duration = times.iter().sum();
    (total, times)
}

fn stats(times: &[Duration]) -> (Duration, Duration, Duration, Duration) {
    let mut sorted: Vec<Duration> = times.to_vec();
    sorted.sort();
    let min = sorted[0];
    let max = *sorted.last().unwrap();
    let median = sorted[sorted.len() / 2];
    let avg = sorted.iter().sum::<Duration>() / sorted.len() as u32;
    (min, max, median, avg)
}

// ============================================================================
// PERFORMANCE EXPERIMENTS (1-15)
// ============================================================================

async fn exp01_baseline_throughput(runtime: &SandCastle) -> ExperimentResult {
    let iters = 500;
    let (total, times) = bench_iterations(runtime, "return null;", None, None, iters).await;
    let (min, max, median, avg) = stats(&times);
    let ops_sec = iters as f64 / total.as_secs_f64();
    ExperimentResult {
        id: 1,
        category: "PERF",
        name: "Baseline throughput (return null)".into(),
        status: "INFO",
        details: vec![
            format!("{} iterations in {:?}", iters, total),
            format!("avg={:?} median={:?} min={:?} max={:?}", avg, median, min, max),
            format!("Throughput: {:.0} ops/sec", ops_sec),
        ],
    }
}

async fn exp02_simple_expression_throughput(runtime: &SandCastle) -> ExperimentResult {
    let iters = 500;
    let (total, times) =
        bench_iterations(runtime, "return 1 + 2 * 3 - 4 / 2;", None, None, iters).await;
    let (min, max, median, avg) = stats(&times);
    let ops_sec = iters as f64 / total.as_secs_f64();
    ExperimentResult {
        id: 2,
        category: "PERF",
        name: "Simple arithmetic expression throughput".into(),
        status: "INFO",
        details: vec![
            format!("{} iterations in {:?}", iters, total),
            format!("avg={:?} median={:?} min={:?} max={:?}", avg, median, min, max),
            format!("Throughput: {:.0} ops/sec", ops_sec),
        ],
    }
}

async fn exp03_memory_limit_impact(runtime: &SandCastle) -> ExperimentResult {
    let iters = 100;
    let mut details = vec![];
    for mb in [1, 2, 4, 8, 16, 32, 64, 128] {
        let limits = Limits {
            memory_mb: mb,
            ..Limits::default()
        };
        let (total, _) =
            bench_iterations(runtime, "return null;", None, Some(limits), iters).await;
        let avg = total / iters as u32;
        details.push(format!("{}MB: avg={:?} ({:.0} ops/sec)", mb, avg, iters as f64 / total.as_secs_f64()));
    }
    ExperimentResult {
        id: 3,
        category: "PERF",
        name: "Memory limit impact on creation time".into(),
        status: "INFO",
        details,
    }
}

async fn exp04_fuel_limit_impact(runtime: &SandCastle) -> ExperimentResult {
    let iters = 100;
    let mut details = vec![];
    // Code that does moderate work
    let code = r#"
        let sum = 0;
        for (let i = 0; i < 1000; i++) { sum += i; }
        return sum;
    "#;
    for fuel in [100_000_000u64, 250_000_000, 500_000_000, 1_000_000_000, 0] {
        let limits = Limits {
            fuel,
            ..Limits::default()
        };
        let (total, _) = bench_iterations(runtime, code, None, Some(limits), iters).await;
        let avg = total / iters as u32;
        let label = if fuel == 0 { "unlimited".to_string() } else { format!("{}M", fuel / 1_000_000) };
        details.push(format!("fuel={}: avg={:?}", label, avg));
    }
    ExperimentResult {
        id: 4,
        category: "PERF",
        name: "Fuel limit impact on execution time".into(),
        status: "INFO",
        details,
    }
}

async fn exp05_concurrency_scaling(runtime: Arc<SandCastle>) -> ExperimentResult {
    let mut details = vec![];
    for count in [1, 5, 10, 25, 50, 100, 250, 500, 1000] {
        let start = Instant::now();
        let mut handles = Vec::new();
        for i in 0..count {
            let rt = runtime.clone();
            handles.push(tokio::spawn(async move {
                let code = format!("return {};", i);
                rt.execute(ExecutionRequest::new(code)).await
            }));
        }
        let mut successes = 0;
        let mut failures = 0;
        for h in handles {
            match h.await {
                Ok(Ok(r)) if r.is_success() => successes += 1,
                _ => failures += 1,
            }
        }
        let elapsed = start.elapsed();
        let per = elapsed / count as u32;
        details.push(format!(
            "n={:>4}: total={:?} per={:?} ok={} fail={}",
            count, elapsed, per, successes, failures
        ));
    }
    ExperimentResult {
        id: 5,
        category: "PERF",
        name: "Concurrency scaling (1 to 1000)".into(),
        status: "INFO",
        details,
    }
}

async fn exp06_input_payload_size(runtime: &SandCastle) -> ExperimentResult {
    let iters = 50;
    let mut details = vec![];
    let code = "const d = globalThis.__sandcastle_input; return d.length || Object.keys(d).length;";
    for size_label in ["1KB", "10KB", "100KB", "1MB"] {
        let n = match size_label {
            "1KB" => 1_000,
            "10KB" => 10_000,
            "100KB" => 100_000,
            "1MB" => 1_000_000,
            _ => 0,
        };
        let input = serde_json::json!({ "data": "x".repeat(n) });
        let (total, _) = bench_iterations(runtime, code, Some(input), None, iters).await;
        let avg = total / iters as u32;
        details.push(format!("{}: avg={:?}", size_label, avg));
    }
    ExperimentResult {
        id: 6,
        category: "PERF",
        name: "Input payload size impact".into(),
        status: "INFO",
        details,
    }
}

async fn exp07_output_payload_size(runtime: &SandCastle) -> ExperimentResult {
    let iters = 50;
    let mut details = vec![];
    for (label, code) in [
        ("tiny", "return 42;"),
        ("1KB", "return 'x'.repeat(1000);"),
        ("10KB", "return 'x'.repeat(10000);"),
        ("100KB", "return 'x'.repeat(100000);"),
    ] {
        let (total, _) = bench_iterations(runtime, code, None, None, iters).await;
        let avg = total / iters as u32;
        details.push(format!("{}: avg={:?}", label, avg));
    }
    ExperimentResult {
        id: 7,
        category: "PERF",
        name: "Output payload size impact".into(),
        status: "INFO",
        details,
    }
}

async fn exp08_code_size_impact(runtime: &SandCastle) -> ExperimentResult {
    let iters = 50;
    let mut details = vec![];
    for n_lines in [1, 10, 100, 500, 1000] {
        let mut code = String::new();
        for i in 0..n_lines {
            code.push_str(&format!("var x{} = {};\n", i, i));
        }
        code.push_str(&format!("return x{};", n_lines - 1));
        let (total, _) = bench_iterations(runtime, &code, None, None, iters).await;
        let avg = total / iters as u32;
        details.push(format!("{} lines: avg={:?} (code_bytes={})", n_lines, avg, code.len()));
    }
    ExperimentResult {
        id: 8,
        category: "PERF",
        name: "Code size impact on execution time".into(),
        status: "INFO",
        details,
    }
}

async fn exp09_json_processing_complexity(runtime: &SandCastle) -> ExperimentResult {
    let iters = 50;
    let mut details = vec![];
    for n_items in [10, 100, 500, 1000, 5000] {
        let input = serde_json::json!({
            "items": (0..n_items).map(|i| serde_json::json!({
                "id": i, "name": format!("item_{}", i), "value": i * 10
            })).collect::<Vec<_>>()
        });
        let code = r#"
            const d = globalThis.__sandcastle_input;
            return d.items.filter(x => x.value > 500).map(x => ({...x, doubled: x.value * 2})).length;
        "#;
        let (total, _) = bench_iterations(runtime, code, Some(input), None, iters).await;
        let avg = total / iters as u32;
        details.push(format!("{} items: avg={:?}", n_items, avg));
    }
    ExperimentResult {
        id: 9,
        category: "PERF",
        name: "JSON processing complexity scaling".into(),
        status: "INFO",
        details,
    }
}

async fn exp10_console_output_overhead(runtime: &SandCastle) -> ExperimentResult {
    let iters = 50;
    let mut details = vec![];
    for n_logs in [0, 10, 100, 500, 1000] {
        let mut code = String::new();
        for i in 0..n_logs {
            code.push_str(&format!("console.log('msg {}');\n", i));
        }
        code.push_str("return null;");
        let (total, _) = bench_iterations(runtime, &code, None, None, iters).await;
        let avg = total / iters as u32;
        details.push(format!("{} logs: avg={:?}", n_logs, avg));
    }
    ExperimentResult {
        id: 10,
        category: "PERF",
        name: "Console output volume overhead".into(),
        status: "INFO",
        details,
    }
}

async fn exp11_artifact_io_throughput(runtime: &SandCastle) -> ExperimentResult {
    let iters = 50;
    let mut details = vec![];

    // Read artifacts
    for size_label in ["1KB", "10KB", "100KB"] {
        let n = match size_label {
            "1KB" => 1_000,
            "10KB" => 10_000,
            "100KB" => 100_000,
            _ => 0,
        };
        let artifact = Artifact::new("data.bin", vec![b'x'; n]);
        let code = r#"
            const d = globalThis.__sandcastle_read_artifact("data.bin");
            return d ? d.length : -1;
        "#;
        let mut total = Duration::ZERO;
        for _ in 0..iters {
            let req = ExecutionRequest::new(code).with_artifacts(vec![artifact.clone()]);
            let start = Instant::now();
            let _ = runtime.execute(req).await;
            total += start.elapsed();
        }
        let avg = total / iters as u32;
        details.push(format!("read {}: avg={:?}", size_label, avg));
    }

    // Write artifacts
    for size_label in ["1KB", "10KB", "100KB"] {
        let n = match size_label {
            "1KB" => 1_000,
            "10KB" => 10_000,
            "100KB" => 100_000,
            _ => 0,
        };
        let code = format!(
            r#"globalThis.__sandcastle_write_artifact("out.bin", "x".repeat({})); return null;"#,
            n
        );
        let (total, _) = bench_iterations(runtime, &code, None, None, iters).await;
        let avg = total / iters as u32;
        details.push(format!("write {}: avg={:?}", size_label, avg));
    }

    ExperimentResult {
        id: 11,
        category: "PERF",
        name: "Artifact I/O throughput".into(),
        status: "INFO",
        details,
    }
}

async fn exp12_capability_call_overhead(runtime: &SandCastle) -> ExperimentResult {
    let iters = 50;
    let mut details = vec![];

    let mut registry = CapabilityRegistry::new();
    registry.register(Box::new(SimpleCapability::new("bench", |method, input| {
        match method {
            "echo" => Ok(input),
            "heavy" => {
                // Simulate some processing
                let data: Vec<i32> = (0..1000).collect();
                Ok(serde_json::json!({"count": data.len()}))
            }
            _ => Ok(serde_json::Value::Null),
        }
    })));
    let caps = Arc::new(registry);

    for n_calls in [1, 5, 10, 25, 50] {
        let mut code = String::new();
        for _ in 0..n_calls {
            code.push_str(r#"__sandcastle_host_call("bench", "echo", '{"x":1}');"#);
            code.push('\n');
        }
        code.push_str("return null;");

        let mut total = Duration::ZERO;
        for _ in 0..iters {
            let req = ExecutionRequest::new(&code).with_capabilities(caps.clone());
            let start = Instant::now();
            let _ = runtime.execute(req).await;
            total += start.elapsed();
        }
        let avg = total / iters as u32;
        details.push(format!("{} calls: avg={:?}", n_calls, avg));
    }

    ExperimentResult {
        id: 12,
        category: "PERF",
        name: "Capability call overhead".into(),
        status: "INFO",
        details,
    }
}

async fn exp13_fuel_consumption_correlation(runtime: &SandCastle) -> ExperimentResult {
    let mut details = vec![];
    let test_cases = [
        ("noop", "return null;"),
        ("arithmetic", "let s=0; for(let i=0;i<100;i++) s+=i; return s;"),
        ("string_ops", "let s=''; for(let i=0;i<100;i++) s+='x'; return s.length;"),
        ("json_parse", "return JSON.parse(JSON.stringify({a:1,b:[2,3],c:{d:4}}));"),
        ("array_ops", "return [1,2,3,4,5].map(x=>x*x).filter(x=>x>5).reduce((a,b)=>a+b,0);"),
        ("regex", "return 'hello world foo bar'.match(/\\w+/g).length;"),
        ("nested_fn", "function f(n){return n<=1?1:f(n-1)+f(n-2);} return f(15);"),
    ];

    for (label, code) in test_cases {
        let result = runtime
            .execute(ExecutionRequest::new(code))
            .await
            .unwrap();
        let fuel = result.transcript.fuel_consumed;
        let mem = result.transcript.peak_memory_bytes;
        details.push(format!("{}: fuel={} peak_mem={}KB", label, fuel, mem / 1024));
    }

    ExperimentResult {
        id: 13,
        category: "PERF",
        name: "Fuel consumption vs code complexity".into(),
        status: "INFO",
        details,
    }
}

async fn exp14_sequential_vs_concurrent(runtime: Arc<SandCastle>) -> ExperimentResult {
    let n = 100;
    let code = "let s=0; for(let i=0;i<100;i++) s+=i; return s;";
    let mut details = vec![];

    // Sequential
    let start = Instant::now();
    for _ in 0..n {
        let _ = runtime.execute(ExecutionRequest::new(code)).await;
    }
    let seq_time = start.elapsed();
    details.push(format!("Sequential ({}): {:?} ({:?}/op)", n, seq_time, seq_time / n as u32));

    // Concurrent
    let start = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..n {
        let rt = runtime.clone();
        handles.push(tokio::spawn(async move {
            rt.execute(ExecutionRequest::new(code)).await
        }));
    }
    for h in handles {
        let _ = h.await;
    }
    let conc_time = start.elapsed();
    details.push(format!("Concurrent  ({}): {:?} ({:?}/op)", n, conc_time, conc_time / n as u32));

    let speedup = seq_time.as_secs_f64() / conc_time.as_secs_f64();
    details.push(format!("Speedup: {:.2}x", speedup));

    ExperimentResult {
        id: 14,
        category: "PERF",
        name: "Sequential vs concurrent execution".into(),
        status: "INFO",
        details,
    }
}

async fn exp15_sustained_throughput(runtime: &SandCastle) -> ExperimentResult {
    let duration_secs = 5;
    let mut details = vec![];
    let code = "return 42;";

    let start = Instant::now();
    let mut count = 0u64;
    let mut window_count = 0u64;
    let mut last_window = start;

    while start.elapsed() < Duration::from_secs(duration_secs) {
        let _ = runtime.execute(ExecutionRequest::new(code)).await;
        count += 1;
        window_count += 1;

        if last_window.elapsed() >= Duration::from_secs(1) {
            details.push(format!(
                "Second {}: {} ops ({:.0} ops/sec)",
                (start.elapsed().as_secs()),
                window_count,
                window_count as f64 / last_window.elapsed().as_secs_f64()
            ));
            window_count = 0;
            last_window = Instant::now();
        }
    }
    let total = start.elapsed();
    details.push(format!(
        "Total: {} ops in {:?} ({:.0} ops/sec sustained)",
        count,
        total,
        count as f64 / total.as_secs_f64()
    ));

    ExperimentResult {
        id: 15,
        category: "PERF",
        name: format!("Sustained throughput over {}s", duration_secs),
        status: "INFO",
        details,
    }
}

// ============================================================================
// FEATURE EXPERIMENTS (16-27)
// ============================================================================

async fn exp16_closures_and_scoping(runtime: &SandCastle) -> ExperimentResult {
    let tests = vec![
        ("closure capture", r#"
            function makeCounter() { let c=0; return ()=>++c; }
            const inc = makeCounter();
            return [inc(), inc(), inc()];
        "#, serde_json::json!([1,2,3])),
        ("IIFE", r#"
            return (function(x){ return x*x; })(7);
        "#, serde_json::json!(49)),
        ("nested closure", r#"
            function outer(x) { return function(y) { return x + y; }; }
            return outer(10)(20);
        "#, serde_json::json!(30)),
        ("closure over loop var", r#"
            const fns = [];
            for (let i = 0; i < 5; i++) { fns.push(() => i); }
            return fns.map(f => f());
        "#, serde_json::json!([0,1,2,3,4])),
    ];
    run_feature_tests(16, "Closures and scoping", tests, runtime).await
}

async fn exp17_generators_and_iterators(runtime: &SandCastle) -> ExperimentResult {
    let tests = vec![
        ("generator basic", r#"
            function* gen() { yield 1; yield 2; yield 3; }
            return [...gen()];
        "#, serde_json::json!([1,2,3])),
        ("generator with state", r#"
            function* fib() { let a=0,b=1; while(true) { yield a; [a,b]=[b,a+b]; } }
            const it = fib();
            const r = [];
            for (let i=0;i<8;i++) r.push(it.next().value);
            return r;
        "#, serde_json::json!([0,1,1,2,3,5,8,13])),
        ("Symbol.iterator", r#"
            const range = { [Symbol.iterator]() { let i=0; return { next() { return i<5 ? {value:i++,done:false} : {done:true}; }}; }};
            return [...range];
        "#, serde_json::json!([0,1,2,3,4])),
    ];
    run_feature_tests(17, "Generators and iterators", tests, runtime).await
}

async fn exp18_regexp_capabilities(runtime: &SandCastle) -> ExperimentResult {
    let tests = vec![
        ("basic match", r#"return "hello123".match(/\d+/)[0];"#, serde_json::json!("123")),
        ("global match", r#"return "a1b2c3".match(/\d/g);"#, serde_json::json!(["1","2","3"])),
        ("named groups", r#"
            const m = "2024-01-15".match(/(?<y>\d{4})-(?<m>\d{2})-(?<d>\d{2})/);
            return { year: m.groups.y, month: m.groups.m, day: m.groups.d };
        "#, serde_json::json!({"year":"2024","month":"01","day":"15"})),
        ("replace", r#"return "hello world".replace(/(\w+)/g, (m) => m.toUpperCase());"#, serde_json::json!("HELLO WORLD")),
        ("split", r#"return "a,,b,,c".split(/,+/);"#, serde_json::json!(["a","b","c"])),
    ];
    run_feature_tests(18, "RegExp capabilities", tests, runtime).await
}

async fn exp19_json_edge_cases(runtime: &SandCastle) -> ExperimentResult {
    let tests = vec![
        ("deeply nested", r#"
            let o = {a:1};
            for(let i=0;i<20;i++) o = {nested: o};
            return JSON.parse(JSON.stringify(o)).nested.nested.nested.a === undefined;
        "#, serde_json::json!(true)),
        ("special values", r#"
            const o = { nan: NaN, inf: Infinity, ninf: -Infinity, undef: undefined, nul: null };
            const parsed = JSON.parse(JSON.stringify(o));
            return { nan: parsed.nan, inf: parsed.inf, nul: parsed.nul, hasUndef: "undef" in parsed };
        "#, serde_json::json!({"nan": null, "inf": null, "nul": null, "hasUndef": false})),
        ("unicode in json", r#"
            return JSON.parse(JSON.stringify({emoji: "Hello 🌍", cjk: "你好世界"}));
        "#, serde_json::json!({"emoji": "Hello 🌍", "cjk": "你好世界"})),
        ("large array", r#"
            const arr = Array.from({length: 10000}, (_, i) => i);
            return JSON.parse(JSON.stringify(arr)).length;
        "#, serde_json::json!(10000)),
    ];
    run_feature_tests(19, "JSON edge cases", tests, runtime).await
}

async fn exp20_math_library(runtime: &SandCastle) -> ExperimentResult {
    let tests = vec![
        ("constants", r#"
            return {
                pi: Math.PI === 3.141592653589793,
                e: Math.E === 2.718281828459045,
                ln2: typeof Math.LN2 === 'number'
            };
        "#, serde_json::json!({"pi": true, "e": true, "ln2": true})),
        ("functions", r#"
            return {
                abs: Math.abs(-5),
                ceil: Math.ceil(1.1),
                floor: Math.floor(1.9),
                round: Math.round(1.5),
                max: Math.max(1,2,3),
                min: Math.min(1,2,3),
                pow: Math.pow(2,10),
                sqrt: Math.sqrt(144),
                trunc: Math.trunc(3.7)
            };
        "#, serde_json::json!({"abs":5,"ceil":2,"floor":1,"round":2,"max":3,"min":1,"pow":1024,"sqrt":12,"trunc":3})),
        ("trig", r#"
            return {
                sin: Math.round(Math.sin(Math.PI/2) * 1000) / 1000,
                cos: Math.round(Math.cos(0) * 1000) / 1000,
                atan2: Math.round(Math.atan2(1,1) * 1000) / 1000
            };
        "#, serde_json::json!({"sin": 1, "cos": 1, "atan2": 0.785})),
        ("random", r#"
            const vals = Array.from({length:100}, () => Math.random());
            const allInRange = vals.every(v => v >= 0 && v < 1);
            const allDifferent = new Set(vals).size > 90;
            return { allInRange, allDifferent };
        "#, serde_json::json!({"allInRange": true, "allDifferent": true})),
    ];
    run_feature_tests(20, "Math library completeness", tests, runtime).await
}

async fn exp21_string_array_methods(runtime: &SandCastle) -> ExperimentResult {
    let tests = vec![
        ("string methods", r#"
            return {
                includes: "hello".includes("ell"),
                startsWith: "hello".startsWith("hel"),
                endsWith: "hello".endsWith("llo"),
                padStart: "5".padStart(3, "0"),
                padEnd: "5".padEnd(3, "0"),
                repeat: "ab".repeat(3),
                trimmed: "  hi  ".trim(),
                at: "hello".at(-1)
            };
        "#, serde_json::json!({
            "includes": true, "startsWith": true, "endsWith": true,
            "padStart": "005", "padEnd": "500", "repeat": "ababab",
            "trimmed": "hi", "at": "o"
        })),
        ("array methods", r#"
            const a = [1,2,3,4,5];
            return {
                find: a.find(x => x > 3),
                findIndex: a.findIndex(x => x > 3),
                flat: [[1],[2,[3]]].flat(Infinity),
                flatMap: a.flatMap(x => [x, x*2]),
                every: a.every(x => x > 0),
                some: a.some(x => x > 4),
                includes: a.includes(3),
                at: a.at(-1),
                fill: [0,0,0].fill(7)
            };
        "#, serde_json::json!({
            "find": 4, "findIndex": 3, "flat": [1,2,3],
            "flatMap": [1,2,2,4,3,6,4,8,5,10],
            "every": true, "some": true, "includes": true, "at": 5, "fill": [7,7,7]
        })),
    ];
    run_feature_tests(21, "String and Array methods", tests, runtime).await
}

async fn exp22_map_set_weakref(runtime: &SandCastle) -> ExperimentResult {
    let tests = vec![
        ("Map", r#"
            const m = new Map();
            m.set("a", 1); m.set("b", 2);
            return { size: m.size, get: m.get("a"), has: m.has("c"), keys: [...m.keys()] };
        "#, serde_json::json!({"size": 2, "get": 1, "has": false, "keys": ["a","b"]})),
        ("Set", r#"
            const s = new Set([1,2,3,2,1]);
            return { size: s.size, has: s.has(2), arr: [...s] };
        "#, serde_json::json!({"size": 3, "has": true, "arr": [1,2,3]})),
        ("WeakMap exists", r#"
            const wm = new WeakMap();
            const key = {};
            wm.set(key, 42);
            return { has: wm.has(key), get: wm.get(key) };
        "#, serde_json::json!({"has": true, "get": 42})),
        ("WeakSet exists", r#"
            const ws = new WeakSet();
            const obj = {};
            ws.add(obj);
            return ws.has(obj);
        "#, serde_json::json!(true)),
    ];
    run_feature_tests(22, "Map, Set, WeakMap, WeakSet", tests, runtime).await
}

async fn exp23_promise_behavior(runtime: &SandCastle) -> ExperimentResult {
    // Test that returning a Promise directly resolves it
    let tests = vec![
        ("return Promise.resolve", r#"
            return Promise.resolve(42);
        "#, serde_json::json!(42)),
        ("return Promise.all", r#"
            return Promise.all([Promise.resolve(1), Promise.resolve(2), Promise.resolve(3)]);
        "#, serde_json::json!([1,2,3])),
        ("return async function", r#"
            async function fetchVal() { return 99; }
            return fetchVal();
        "#, serde_json::json!(99)),
    ];
    run_feature_tests(23, "Promise behavior in QuickJS", tests, runtime).await
}

async fn exp24_typed_arrays(runtime: &SandCastle) -> ExperimentResult {
    let tests = vec![
        ("Uint8Array", r#"
            const a = new Uint8Array([1, 2, 255, 256]);
            return { len: a.length, last: a[3], overflow: a[2] };
        "#, serde_json::json!({"len": 4, "last": 0, "overflow": 255})),
        ("Float64Array", r#"
            const a = new Float64Array([1.5, 2.7, 3.14]);
            return { len: a.length, sum: a[0] + a[1] + a[2] };
        "#, serde_json::json!({"len": 3, "sum": 7.34})),
        ("ArrayBuffer", r#"
            const buf = new ArrayBuffer(16);
            const view = new DataView(buf);
            view.setInt32(0, 42);
            return view.getInt32(0);
        "#, serde_json::json!(42)),
        ("TypedArray methods", r#"
            const a = new Int32Array([5,3,1,4,2]);
            const sorted = new Int32Array(a).sort();
            return { sorted: [...sorted], slice: [...a.slice(1,3)] };
        "#, serde_json::json!({"sorted": [1,2,3,4,5], "slice": [3,1]})),
    ];
    run_feature_tests(24, "TypedArray support", tests, runtime).await
}

async fn exp25_error_types(runtime: &SandCastle) -> ExperimentResult {
    let mut details = vec![];
    let error_tests = [
        ("TypeError", "null.foo"),
        ("ReferenceError", "undeclaredVar"),
        ("SyntaxError", "eval('{{{{')"),
        ("RangeError", "new Array(-1)"),
        ("URIError", "decodeURI('%')"),
    ];
    for (expected_type, code) in error_tests {
        let full_code = format!(
            r#"try {{ {} }} catch(e) {{ return {{ type: e.constructor.name, message: e.message }}; }}"#,
            code
        );
        let result = runtime.execute(ExecutionRequest::new(full_code)).await.unwrap();
        let output = match &result.output {
            OutputValue::Json(v) => v.clone(),
            _ => serde_json::json!(null),
        };
        let got_type = output.get("type").and_then(|v| v.as_str()).unwrap_or("unknown");
        let status = if got_type == expected_type { "ok" } else { "MISMATCH" };
        details.push(format!("[{}] {} => got {}: {}", status, expected_type, got_type,
            output.get("message").and_then(|v| v.as_str()).unwrap_or("?")));
    }
    ExperimentResult {
        id: 25,
        category: "FEAT",
        name: "Error types and stack traces".into(),
        status: if details.iter().all(|d| d.starts_with("[ok]")) { "PASS" } else { "WARN" },
        details,
    }
}

async fn exp26_destructuring_and_spread(runtime: &SandCastle) -> ExperimentResult {
    let tests = vec![
        ("object destructuring", r#"
            const {a, b, ...rest} = {a:1, b:2, c:3, d:4};
            return {a, b, rest};
        "#, serde_json::json!({"a":1, "b":2, "rest":{"c":3,"d":4}})),
        ("array destructuring", r#"
            const [x, y, ...rest] = [1,2,3,4,5];
            return {x, y, rest};
        "#, serde_json::json!({"x":1, "y":2, "rest":[3,4,5]})),
        ("template literals", r#"
            const name = "world";
            return `hello ${name} ${1+2}`;
        "#, serde_json::json!("hello world 3")),
        ("optional chaining", r#"
            const o = {a: {b: {c: 42}}};
            return { deep: o?.a?.b?.c, missing: o?.x?.y?.z ?? "default" };
        "#, serde_json::json!({"deep": 42, "missing": "default"})),
        ("nullish coalescing", r#"
            return { a: null ?? "fallback", b: 0 ?? "fallback", c: undefined ?? "fallback" };
        "#, serde_json::json!({"a": "fallback", "b": 0, "c": "fallback"})),
    ];
    run_feature_tests(26, "Destructuring, spread, modern syntax", tests, runtime).await
}

async fn exp27_date_handling(runtime: &SandCastle) -> ExperimentResult {
    let tests = vec![
        ("Date constructor", r#"
            const d = new Date(2024, 0, 15);
            return { year: d.getFullYear(), month: d.getMonth(), day: d.getDate() };
        "#, serde_json::json!({"year": 2024, "month": 0, "day": 15})),
        ("Date.now", r#"
            const t = Date.now();
            return typeof t === 'number' && t > 0;
        "#, serde_json::json!(true)),
        ("ISO string", r#"
            const d = new Date("2024-06-15T12:00:00Z");
            return d.toISOString();
        "#, serde_json::json!("2024-06-15T12:00:00.000Z")),
    ];
    run_feature_tests(27, "Date handling", tests, runtime).await
}

// ============================================================================
// STRESS/RELIABILITY EXPERIMENTS (28-40)
// ============================================================================

async fn exp28_infinite_loop_fuel(runtime: &SandCastle) -> ExperimentResult {
    let limits = Limits {
        fuel: 200_000_000,
        timeout: Duration::from_secs(5),
        ..Limits::default()
    };
    let start = Instant::now();
    let result = runtime
        .execute(ExecutionRequest::new("while(true){}").with_limits(limits))
        .await
        .unwrap();
    let elapsed = start.elapsed();
    let status_str = format!("{:?}", result.status);
    ExperimentResult {
        id: 28,
        category: "STRESS",
        name: "Infinite loop caught by fuel limit".into(),
        status: if matches!(result.status, ExecutionStatus::FuelExhausted | ExecutionStatus::Timeout) {
            "PASS"
        } else {
            "FAIL"
        },
        details: vec![
            format!("Status: {}", status_str),
            format!("Time to halt: {:?}", elapsed),
            format!("Fuel consumed: {}", result.transcript.fuel_consumed),
        ],
    }
}

async fn exp29_infinite_recursion(runtime: &SandCastle) -> ExperimentResult {
    let start = Instant::now();
    let result = runtime
        .execute(ExecutionRequest::new("function f(){f()} f();"))
        .await
        .unwrap();
    let elapsed = start.elapsed();
    ExperimentResult {
        id: 29,
        category: "STRESS",
        name: "Infinite recursion (stack overflow)".into(),
        status: if !result.is_success() { "PASS" } else { "FAIL" },
        details: vec![
            format!("Status: {:?}", result.status),
            format!("Time: {:?}", elapsed),
        ],
    }
}

async fn exp30_memory_bomb(runtime: &SandCastle) -> ExperimentResult {
    let limits = Limits {
        memory_mb: 8,
        timeout: Duration::from_secs(5),
        ..Limits::default()
    };
    let code = r#"
        const arrays = [];
        while(true) { arrays.push(new Array(100000).fill('x')); }
    "#;
    let start = Instant::now();
    let result = runtime
        .execute(ExecutionRequest::new(code).with_limits(limits))
        .await
        .unwrap();
    let elapsed = start.elapsed();
    ExperimentResult {
        id: 30,
        category: "STRESS",
        name: "Memory bomb (8MB limit)".into(),
        status: if !result.is_success() { "PASS" } else { "FAIL" },
        details: vec![
            format!("Status: {:?}", result.status),
            format!("Time: {:?}", elapsed),
            format!("Peak memory: {}KB", result.transcript.peak_memory_bytes / 1024),
        ],
    }
}

async fn exp31_deeply_nested_objects(runtime: &SandCastle) -> ExperimentResult {
    let code = r#"
        let o = {v: 1};
        for (let i = 0; i < 1000; i++) { o = {child: o}; }
        return typeof o;
    "#;
    let result = runtime.execute(ExecutionRequest::new(code)).await.unwrap();
    ExperimentResult {
        id: 31,
        category: "STRESS",
        name: "Deeply nested objects (1000 levels)".into(),
        status: if result.is_success() { "PASS" } else { "WARN" },
        details: vec![
            format!("Status: {:?}", result.status),
            format!("Fuel: {}", result.transcript.fuel_consumed),
        ],
    }
}

async fn exp32_redos_attack(runtime: &SandCastle) -> ExperimentResult {
    let limits = Limits {
        fuel: 500_000_000,
        timeout: Duration::from_secs(3),
        ..Limits::default()
    };
    // Classic ReDoS pattern
    let code = r#"
        const evil = "a".repeat(25) + "!";
        const re = /^(a+)+$/;
        return re.test(evil);
    "#;
    let start = Instant::now();
    let result = runtime
        .execute(ExecutionRequest::new(code).with_limits(limits))
        .await
        .unwrap();
    let elapsed = start.elapsed();
    ExperimentResult {
        id: 32,
        category: "STRESS",
        name: "ReDoS catastrophic backtracking".into(),
        status: if elapsed < Duration::from_secs(5) { "PASS" } else { "WARN" },
        details: vec![
            format!("Status: {:?}", result.status),
            format!("Time: {:?}", elapsed),
            format!("Contained: {}", if elapsed < Duration::from_secs(5) { "yes" } else { "no" }),
        ],
    }
}

async fn exp33_rapid_create_destroy(runtime: &SandCastle) -> ExperimentResult {
    let n = 2000;
    let start = Instant::now();
    let mut successes = 0u64;
    for _ in 0..n {
        let r = runtime.execute(ExecutionRequest::new("return 1;")).await;
        if r.is_ok() && r.unwrap().is_success() {
            successes += 1;
        }
    }
    let elapsed = start.elapsed();
    ExperimentResult {
        id: 33,
        category: "STRESS",
        name: format!("Rapid create/destroy ({} sandboxes)", n),
        status: if successes == n { "PASS" } else { "WARN" },
        details: vec![
            format!("{}/{} succeeded in {:?}", successes, n, elapsed),
            format!("{:.0} ops/sec", n as f64 / elapsed.as_secs_f64()),
        ],
    }
}

async fn exp34_max_concurrency_limit() -> ExperimentResult {
    let runtime = Arc::new(create_runtime_with_concurrency(5));
    let n = 20;
    let start = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..n {
        let rt = runtime.clone();
        handles.push(tokio::spawn(async move {
            rt.execute(
                ExecutionRequest::new(
                    "let s=0; for(let i=0;i<10000;i++) s+=i; return s;"
                )
                .with_limits(Limits {
                    timeout: Duration::from_secs(5),
                    ..Limits::default()
                }),
            )
            .await
        }));
    }
    let mut successes = 0;
    for h in handles {
        if let Ok(Ok(r)) = h.await {
            if r.is_success() {
                successes += 1;
            }
        }
    }
    let elapsed = start.elapsed();
    ExperimentResult {
        id: 34,
        category: "STRESS",
        name: "Max concurrency limit (5 slots, 20 tasks)".into(),
        status: if successes == n { "PASS" } else { "WARN" },
        details: vec![
            format!("{}/{} succeeded in {:?}", successes, n, elapsed),
            format!("All should succeed (semaphore queues, doesn't reject)"),
        ],
    }
}

async fn exp35_huge_input(runtime: &SandCastle) -> ExperimentResult {
    let sizes = [1_000_000, 5_000_000, 10_000_000];
    let mut details = vec![];
    for size in sizes {
        let input = serde_json::json!({"data": "x".repeat(size)});
        let start = Instant::now();
        let result = runtime
            .execute(
                ExecutionRequest::new("return globalThis.__sandcastle_input.data.length;")
                    .with_input(input),
            )
            .await;
        let elapsed = start.elapsed();
        match result {
            Ok(r) => details.push(format!(
                "{}MB input: {:?} status={:?} output={:?}",
                size / 1_000_000,
                elapsed,
                r.status,
                r.output
            )),
            Err(e) => details.push(format!("{}MB input: ERROR {:?}", size / 1_000_000, e)),
        }
    }
    ExperimentResult {
        id: 35,
        category: "STRESS",
        name: "Huge input payloads (1-10MB)".into(),
        status: "INFO",
        details,
    }
}

async fn exp36_console_spam(runtime: &SandCastle) -> ExperimentResult {
    let code = r#"
        for (let i = 0; i < 10000; i++) {
            console.log("spam line " + i);
        }
        return "done";
    "#;
    let start = Instant::now();
    let result = runtime.execute(ExecutionRequest::new(code)).await.unwrap();
    let elapsed = start.elapsed();
    ExperimentResult {
        id: 36,
        category: "STRESS",
        name: "Console spam (10K log lines)".into(),
        status: if result.is_success() { "PASS" } else { "WARN" },
        details: vec![
            format!("Status: {:?}", result.status),
            format!("Time: {:?}", elapsed),
            format!("Console messages captured: {}", result.transcript.console.len()),
            format!("Fuel: {}", result.transcript.fuel_consumed),
        ],
    }
}

async fn exp37_capability_quota_exhaustion(runtime: &SandCastle) -> ExperimentResult {
    let mut registry = CapabilityRegistry::new();
    registry.register_with_limits(
        Box::new(SimpleCapability::new("limited", |_, _| Ok(serde_json::json!("ok")))),
        CapabilityLimits {
            max_calls: 5,
            ..CapabilityLimits::default()
        },
    );
    let caps = Arc::new(registry);

    let code = r#"
        const results = [];
        for (let i = 0; i < 10; i++) {
            try {
                const r = __sandcastle_host_call("limited", "do", '{}');
                results.push({i, ok: true});
            } catch(e) {
                results.push({i, ok: false, err: String(e)});
            }
        }
        return results;
    "#;
    let result = runtime
        .execute(ExecutionRequest::new(code).with_capabilities(caps))
        .await
        .unwrap();

    let output = match &result.output {
        OutputValue::Json(v) => v.clone(),
        _ => serde_json::json!(null),
    };
    let arr = output.as_array();
    let ok_count = arr.map(|a| a.iter().filter(|v| v.get("ok") == Some(&serde_json::json!(true))).count()).unwrap_or(0);

    ExperimentResult {
        id: 37,
        category: "STRESS",
        name: "Capability quota exhaustion (5 call limit)".into(),
        status: if ok_count == 5 { "PASS" } else { "WARN" },
        details: vec![
            format!("Successful calls: {} (expected 5)", ok_count),
            format!("Status: {:?}", result.status),
        ],
    }
}

async fn exp38_malformed_javascript(runtime: &SandCastle) -> ExperimentResult {
    let mut bad_inputs = vec![
        ("empty string", ""),
        ("just whitespace", "   \n\t  "),
        ("syntax error", "function { broken"),
        ("incomplete", "return"),
        ("null bytes", "return \0\0\01;"),
        ("binary garbage", "\x7f\x01\x02"),
    ];
    let unicode_bomb = "🌍".repeat(10000);
    let long_ident = format!("var {} = 1; return {};", "x".repeat(50000), "x".repeat(50000));
    bad_inputs.push(("unicode bomb", &unicode_bomb));
    bad_inputs.push(("very long identifier", &long_ident));
    let mut details = vec![];
    for (label, code) in bad_inputs {
        let result = runtime.execute(ExecutionRequest::new(code)).await;
        match result {
            Ok(r) => details.push(format!("[handled] {}: {:?}", label, r.status)),
            Err(e) => details.push(format!("[error] {}: {:?}", label, e)),
        }
    }
    ExperimentResult {
        id: 38,
        category: "STRESS",
        name: "Malformed JavaScript handling".into(),
        status: if details.iter().all(|d| d.starts_with("[handled]") || d.starts_with("[error]")) {
            "PASS"
        } else {
            "FAIL"
        },
        details,
    }
}

async fn exp39_unicode_edge_cases(runtime: &SandCastle) -> ExperimentResult {
    let tests = vec![
        ("emoji", r#"return "👨‍👩‍👧‍👦".length;"#, None), // Just check it doesn't crash
        ("null in string", r#"return "a\u0000b".length;"#, Some(serde_json::json!(3))),
        ("RTL", r#"return "مرحبا".length;"#, Some(serde_json::json!(5))),
        ("surrogate pairs", r#"return "𝕳𝖊𝖑𝖑𝖔".length;"#, None),
        ("BOM", r#"return "\uFEFFhello".trim().length;"#, None),
        ("combining chars", r#"return "e\u0301".normalize("NFC");"#, Some(serde_json::json!("é"))),
    ];
    let mut details = vec![];
    for (label, code, expected) in tests {
        let result = runtime.execute(ExecutionRequest::new(code)).await.unwrap();
        let ok = result.is_success();
        let output = match &result.output {
            OutputValue::Json(v) => format!("{}", v),
            other => format!("{:?}", other),
        };
        let match_str = if let Some(exp) = expected {
            if matches!(&result.output, OutputValue::Json(v) if v == &exp) { "exact" } else { "differ" }
        } else {
            if ok { "ran" } else { "failed" }
        };
        details.push(format!("[{}] {}: output={}", match_str, label, output));
    }
    ExperimentResult {
        id: 39,
        category: "STRESS",
        name: "Unicode edge cases".into(),
        status: "PASS",
        details,
    }
}

async fn exp40_output_artifact_stress(runtime: &SandCastle) -> ExperimentResult {
    let code = r#"
        for (let i = 0; i < 16; i++) {
            globalThis.__sandcastle_write_artifact("file_" + i + ".txt", "data_" + i);
        }
        return "done";
    "#;
    let result = runtime.execute(ExecutionRequest::new(code)).await.unwrap();
    let artifact_count = result.output_artifacts.len();
    ExperimentResult {
        id: 40,
        category: "STRESS",
        name: "Output artifact stress (16 files)".into(),
        status: if result.is_success() && artifact_count == 16 { "PASS" } else { "WARN" },
        details: vec![
            format!("Status: {:?}", result.status),
            format!("Artifacts written: {}", artifact_count),
        ],
    }
}

// ============================================================================
// ARCHITECTURE EXPERIMENTS (41-50)
// ============================================================================

async fn exp41_module_compilation_cost() -> ExperimentResult {
    let guest_module = load_guest_module();
    let mut details = vec![];
    let mut times = vec![];

    for i in 0..10 {
        let gm = guest_module.clone();
        let start = Instant::now();
        let _ = SandCastle::new(Config::new(gm)).unwrap();
        let elapsed = start.elapsed();
        times.push(elapsed);
        if i < 5 || i == 9 {
            details.push(format!("Compilation #{}: {:?}", i + 1, elapsed));
        }
    }
    let (min, max, median, avg) = stats(&times);
    details.push(format!("avg={:?} median={:?} min={:?} max={:?}", avg, median, min, max));

    ExperimentResult {
        id: 41,
        category: "ARCH",
        name: "Module compilation cost (10 runtimes)".into(),
        status: "INFO",
        details,
    }
}

async fn exp42_fuel_precision(runtime: &SandCastle) -> ExperimentResult {
    let mut details = vec![];
    // Test that fuel is consistently consumed for the same code
    let code = "let s=0; for(let i=0;i<100;i++) s+=i; return s;";
    let mut fuel_values = vec![];
    for _ in 0..10 {
        let result = runtime.execute(ExecutionRequest::new(code)).await.unwrap();
        fuel_values.push(result.transcript.fuel_consumed);
    }
    let all_same = fuel_values.windows(2).all(|w| w[0] == w[1]);
    details.push(format!("Fuel values (10 runs): {:?}", &fuel_values[..5]));
    details.push(format!("Deterministic: {}", all_same));

    // Test fuel boundary: find exact fuel needed
    let result = runtime.execute(ExecutionRequest::new(code)).await.unwrap();
    let exact_fuel = result.transcript.fuel_consumed;
    details.push(format!("Exact fuel needed: {}", exact_fuel));

    // Try with slightly less fuel
    let limits = Limits {
        fuel: exact_fuel.saturating_sub(1000),
        ..Limits::default()
    };
    let result = runtime
        .execute(ExecutionRequest::new(code).with_limits(limits))
        .await
        .unwrap();
    details.push(format!(
        "With fuel-1000: {:?}",
        result.status
    ));

    ExperimentResult {
        id: 42,
        category: "ARCH",
        name: "Fuel metering precision and determinism".into(),
        status: if all_same { "PASS" } else { "WARN" },
        details,
    }
}

async fn exp43_epoch_timeout_precision(runtime: &SandCastle) -> ExperimentResult {
    let mut details = vec![];
    for timeout_ms in [100, 250, 500, 1000, 2000] {
        let limits = Limits {
            timeout: Duration::from_millis(timeout_ms),
            fuel: 0,
            ..Limits::default()
        };
        let start = Instant::now();
        let result = runtime
            .execute(ExecutionRequest::new("while(true){}").with_limits(limits))
            .await
            .unwrap();
        let actual_ms = start.elapsed().as_millis();
        let overshoot = actual_ms as i64 - timeout_ms as i64;
        details.push(format!(
            "target={}ms actual={}ms overshoot={}ms status={:?}",
            timeout_ms, actual_ms, overshoot, result.status
        ));
    }
    ExperimentResult {
        id: 43,
        category: "ARCH",
        name: "Epoch timeout precision".into(),
        status: "INFO",
        details,
    }
}

async fn exp44_memory_enforcement_precision(runtime: &SandCastle) -> ExperimentResult {
    let mut details = vec![];
    for mb in [2, 4, 8, 16] {
        let limits = Limits {
            memory_mb: mb,
            timeout: Duration::from_secs(5),
            ..Limits::default()
        };
        // Try to allocate a chunk relative to the limit
        let alloc_bytes = (mb as usize) * 1024 * 1024;
        let code = format!(
            r#"
            try {{
                const arr = new Uint8Array({});
                return {{ allocated: true, size: arr.length }};
            }} catch(e) {{
                return {{ allocated: false, error: e.message }};
            }}
            "#,
            alloc_bytes / 4 // Try quarter of the limit
        );
        let result = match runtime
            .execute(ExecutionRequest::new(&code).with_limits(limits.clone()))
            .await
        {
            Ok(r) => r,
            Err(e) => {
                details.push(format!("{}MB limit: creation error: {:?}", mb, e));
                continue;
            }
        };
        details.push(format!(
            "{}MB limit, alloc {}KB: status={:?} peak={}KB output={:?}",
            mb,
            alloc_bytes / 2048,
            result.status,
            result.transcript.peak_memory_bytes / 1024,
            match &result.output {
                OutputValue::Json(v) => format!("{}", v),
                _ => "?".into(),
            }
        ));
    }
    ExperimentResult {
        id: 44,
        category: "ARCH",
        name: "Memory limit enforcement precision".into(),
        status: "INFO",
        details,
    }
}

async fn exp45_capability_concurrency_limits(runtime: &SandCastle) -> ExperimentResult {
    // This tests the quota tracker concurrency, not actual concurrent calls
    // (since dispatch_sync is synchronous from the guest's perspective)
    let mut registry = CapabilityRegistry::new();
    registry.register_with_limits(
        Box::new(SimpleCapability::new("conc_test", |_, _| {
            Ok(serde_json::json!("ok"))
        })),
        CapabilityLimits {
            max_concurrent: 2,
            max_calls: 100,
            ..CapabilityLimits::default()
        },
    );
    let caps = Arc::new(registry);

    let code = r#"
        const results = [];
        for (let i = 0; i < 20; i++) {
            try {
                __sandcastle_host_call("conc_test", "do", '{}');
                results.push("ok");
            } catch(e) {
                results.push("err: " + e);
            }
        }
        return { total: results.length, ok: results.filter(r => r === "ok").length };
    "#;
    let result = runtime
        .execute(ExecutionRequest::new(code).with_capabilities(caps))
        .await
        .unwrap();

    ExperimentResult {
        id: 45,
        category: "ARCH",
        name: "Capability concurrency limit behavior".into(),
        status: if result.is_success() { "PASS" } else { "WARN" },
        details: vec![
            format!("Output: {:?}", result.output),
            format!("Note: sync dispatch means concurrency=1 effective from guest"),
        ],
    }
}

async fn exp46_namespace_isolation(runtime: &SandCastle) -> ExperimentResult {
    let mut details = vec![];

    let caps = Arc::new(CapabilityRegistry::new());
    let manager = NamespaceManager::new(10);

    let ns1 = manager
        .create("tenant-a", NamespaceLimits::default(), caps.clone())
        .unwrap();
    let ns2 = manager
        .create("tenant-b", NamespaceLimits::default(), caps.clone())
        .unwrap();

    ns1.register("handler", String::from("return 'A';"), None).unwrap();
    ns2.register("handler", String::from("return 'B';"), None).unwrap();

    // Verify isolation
    let script_a = ns1.get_script("handler").unwrap();
    let script_b = ns2.get_script("handler").unwrap();

    let result_a = runtime
        .execute(
            ExecutionRequest::new(&script_a.code)
                .with_capabilities(script_a.capabilities.clone()),
        )
        .await
        .unwrap();
    let result_b = runtime
        .execute(
            ExecutionRequest::new(&script_b.code)
                .with_capabilities(script_b.capabilities.clone()),
        )
        .await
        .unwrap();

    let a_output = match &result_a.output {
        OutputValue::Json(v) => v.clone(),
        _ => serde_json::json!(null),
    };
    let b_output = match &result_b.output {
        OutputValue::Json(v) => v.clone(),
        _ => serde_json::json!(null),
    };

    details.push(format!("tenant-a output: {}", a_output));
    details.push(format!("tenant-b output: {}", b_output));

    let ns1_scripts = ns1.list_scripts();
    let ns2_scripts = ns2.list_scripts();
    details.push(format!("tenant-a scripts: {:?}", ns1_scripts));
    details.push(format!("tenant-b scripts: {:?}", ns2_scripts));

    // Cross-contamination check
    let isolated = a_output == serde_json::json!("A") && b_output == serde_json::json!("B");
    details.push(format!("Isolated: {}", isolated));

    ExperimentResult {
        id: 46,
        category: "ARCH",
        name: "Namespace isolation verification".into(),
        status: if isolated { "PASS" } else { "FAIL" },
        details,
    }
}

async fn exp47_registry_performance_at_scale(runtime: &SandCastle) -> ExperimentResult {
    let mut details = vec![];

    for n_scripts in [10, 100, 500, 1000] {
        let registry = ScriptRegistry::new(n_scripts + 1);
        let caps = Arc::new(CapabilityRegistry::new());
        let start = Instant::now();
        for i in 0..n_scripts {
            registry
                .register(
                    &format!("script_{}", i),
                    format!("return {};", i),
                    caps.clone(),
                    Limits::default(),
                )
                .unwrap();
        }
        let register_time = start.elapsed();

        // Lookup performance
        let start = Instant::now();
        for i in 0..n_scripts {
            let _ = registry.get(&format!("script_{}", i));
        }
        let lookup_time = start.elapsed();

        // Dispatch last script
        let start = Instant::now();
        let _ = runtime
            .dispatch(
                &registry,
                &format!("script_{}", n_scripts - 1),
                serde_json::Value::Null,
            )
            .await;
        let dispatch_time = start.elapsed();

        details.push(format!(
            "{} scripts: register={:?} lookup_all={:?} dispatch_one={:?}",
            n_scripts, register_time, lookup_time, dispatch_time
        ));
    }

    ExperimentResult {
        id: 47,
        category: "ARCH",
        name: "Script registry performance at scale".into(),
        status: "INFO",
        details,
    }
}

async fn exp48_transcript_overhead(runtime: &SandCastle) -> ExperimentResult {
    let mut details = vec![];

    // Minimal transcript
    let result = runtime
        .execute(ExecutionRequest::new("return 1;"))
        .await
        .unwrap();
    let transcript_json = serde_json::to_string(&result.transcript).unwrap();
    details.push(format!(
        "Minimal transcript size: {} bytes",
        transcript_json.len()
    ));

    // Heavy transcript (lots of console + capabilities)
    let mut registry = CapabilityRegistry::new();
    registry.register(Box::new(SimpleCapability::new("t", |_, _| {
        Ok(serde_json::json!("ok"))
    })));
    let caps = Arc::new(registry);

    let code = r#"
        for (let i = 0; i < 100; i++) {
            console.log("log " + i);
            __sandcastle_host_call("t", "do", '{}');
        }
        return "done";
    "#;
    let result = runtime
        .execute(ExecutionRequest::new(code).with_capabilities(caps))
        .await
        .unwrap();
    let transcript_json = serde_json::to_string(&result.transcript).unwrap();
    details.push(format!(
        "Heavy transcript (100 logs + 100 cap calls): {} bytes",
        transcript_json.len()
    ));
    details.push(format!(
        "Console entries: {}",
        result.transcript.console.len()
    ));
    details.push(format!(
        "Capability call entries: {}",
        result.transcript.capability_calls.len()
    ));

    ExperimentResult {
        id: 48,
        category: "ARCH",
        name: "Transcript overhead measurement".into(),
        status: "INFO",
        details,
    }
}

async fn exp49_store_creation_overhead() -> ExperimentResult {
    let guest_module = load_guest_module();
    let runtime = SandCastle::new(Config::new(guest_module)).unwrap();
    let mut details = vec![];

    // Measure just the execute call overhead for minimal work
    let iters = 200;
    let mut times = Vec::with_capacity(iters);
    for _ in 0..iters {
        let start = Instant::now();
        let _ = runtime
            .execute(ExecutionRequest::new("return null;"))
            .await;
        times.push(start.elapsed());
    }
    let (min, max, median, avg) = stats(&times);
    details.push(format!(
        "Execute overhead (includes Store+Linker+Instance creation):"
    ));
    details.push(format!(
        "  avg={:?} median={:?} min={:?} max={:?}",
        avg, median, min, max
    ));

    // Compare: creating a Sandbox object (no execution)
    let mut sandbox_times = Vec::with_capacity(iters);
    for _ in 0..iters {
        let start = Instant::now();
        let _ = runtime.create_sandbox();
        sandbox_times.push(start.elapsed());
    }
    let (smin, smax, smedian, savg) = stats(&sandbox_times);
    details.push(format!("Sandbox::new (no execution):"));
    details.push(format!(
        "  avg={:?} median={:?} min={:?} max={:?}",
        savg, smedian, smin, smax
    ));

    ExperimentResult {
        id: 49,
        category: "ARCH",
        name: "Store creation overhead isolation".into(),
        status: "INFO",
        details,
    }
}

async fn exp50_execution_determinism(runtime: &SandCastle) -> ExperimentResult {
    let mut details = vec![];
    let code = r#"
        function fibonacci(n) {
            if (n <= 1) return n;
            return fibonacci(n - 1) + fibonacci(n - 2);
        }
        const results = [];
        for (let i = 0; i < 20; i++) {
            results.push(fibonacci(i));
        }
        return results;
    "#;

    let mut outputs = vec![];
    let mut fuels = vec![];
    for _ in 0..10 {
        let result = runtime.execute(ExecutionRequest::new(code)).await.unwrap();
        if let OutputValue::Json(v) = &result.output {
            outputs.push(v.clone());
        }
        fuels.push(result.transcript.fuel_consumed);
    }

    let all_outputs_same = outputs.windows(2).all(|w| w[0] == w[1]);
    let all_fuel_same = fuels.windows(2).all(|w| w[0] == w[1]);

    details.push(format!("Output deterministic: {}", all_outputs_same));
    details.push(format!("Fuel deterministic: {}", all_fuel_same));
    details.push(format!("Fuel values: {:?}", &fuels[..3]));
    if !outputs.is_empty() {
        details.push(format!("Fibonacci(19) = {}", outputs[0].as_array().unwrap().last().unwrap()));
    }

    ExperimentResult {
        id: 50,
        category: "ARCH",
        name: "Execution determinism (output + fuel)".into(),
        status: if all_outputs_same && all_fuel_same { "PASS" } else { "WARN" },
        details,
    }
}

// ============================================================================
// FEATURE TEST HELPER
// ============================================================================

async fn run_feature_tests(
    id: usize,
    name: &str,
    tests: Vec<(&str, &str, serde_json::Value)>,
    runtime: &SandCastle,
) -> ExperimentResult {
    let mut details = vec![];
    let mut all_pass = true;

    for (label, code, expected) in &tests {
        let result = runtime.execute(ExecutionRequest::new(*code)).await.unwrap();
        let output = match &result.output {
            OutputValue::Json(v) => v.clone(),
            other => {
                details.push(format!("[FAIL] {}: unexpected output type {:?}", label, other));
                all_pass = false;
                continue;
            }
        };
        if &output == expected {
            details.push(format!("[ok] {}", label));
        } else {
            details.push(format!("[FAIL] {}: expected {} got {}", label, expected, output));
            all_pass = false;
        }
    }

    ExperimentResult {
        id,
        category: "FEAT",
        name: name.to_string(),
        status: if all_pass { "PASS" } else { "FAIL" },
        details,
    }
}

// ============================================================================
// ROUND 2 EXPERIMENTS (51-70)
// ============================================================================

async fn exp51_memory_exceeded_status(runtime: &SandCastle) -> ExperimentResult {
    let mut details = vec![];
    // Test that exceeding memory produces MemoryExceeded (not opaque GuestError)
    for mb in [4, 8, 16] {
        let limits = Limits {
            memory_mb: mb,
            timeout: Duration::from_secs(5),
            ..Limits::default()
        };
        let alloc_target = mb as usize * 1024 * 1024; // try to allocate the full limit
        let code = format!(
            "try {{ const a = new Uint8Array({}); return 'ok'; }} catch(e) {{ return e.message; }}",
            alloc_target
        );
        let result = runtime
            .execute(ExecutionRequest::new(&code).with_limits(limits))
            .await
            .unwrap();
        details.push(format!(
            "{}MB limit, alloc {}MB: status={:?}",
            mb, mb, result.status
        ));
    }
    ExperimentResult {
        id: 51,
        category: "MEM",
        name: "MemoryExceeded status for over-limit allocations".into(),
        status: "INFO",
        details,
    }
}

async fn exp52_trap_on_grow_failure_classification(runtime: &SandCastle) -> ExperimentResult {
    let limits = Limits {
        memory_mb: 4,
        timeout: Duration::from_secs(5),
        ..Limits::default()
    };
    // Allocate way beyond the limit — should trigger trap_on_grow_failure
    let code = r#"
        const chunks = [];
        for (let i = 0; i < 100; i++) {
            chunks.push(new Uint8Array(1024 * 1024)); // 1MB each
        }
        return chunks.length;
    "#;
    let result = runtime
        .execute(ExecutionRequest::new(code).with_limits(limits))
        .await
        .unwrap();

    let is_memory = matches!(result.status, ExecutionStatus::MemoryExceeded);
    let is_guest_oom = matches!(&result.status, ExecutionStatus::GuestError { message } if message.contains("memory") || message.contains("out of"));

    ExperimentResult {
        id: 52,
        category: "MEM",
        name: "trap_on_grow_failure maps to MemoryExceeded".into(),
        status: if is_memory || is_guest_oom { "PASS" } else { "FAIL" },
        details: vec![
            format!("Status: {:?}", result.status),
            format!("Classified as memory error: {}", is_memory || is_guest_oom),
        ],
    }
}

async fn exp53_gc_threshold_impact(runtime: &SandCastle) -> ExperimentResult {
    let iters = 100;
    let mut details = vec![];
    // Code that creates lots of garbage
    let code = r#"
        let results = [];
        for (let i = 0; i < 1000; i++) {
            const obj = { a: i, b: String(i).repeat(10), c: [i, i+1, i+2] };
            results.push(JSON.parse(JSON.stringify(obj)));
        }
        return results.length;
    "#;
    let (total, times) = bench_iterations(runtime, code, None, None, iters).await;
    let (min, max, median, avg) = stats(&times);
    details.push(format!("{} iters, avg={:?} median={:?}", iters, avg, median));

    // Check fuel to gauge GC activity
    let result = runtime.execute(ExecutionRequest::new(code)).await.unwrap();
    details.push(format!("Fuel consumed: {}", result.transcript.fuel_consumed));
    details.push(format!("Peak memory: {}KB", result.transcript.peak_memory_bytes / 1024));

    ExperimentResult {
        id: 53,
        category: "MEM",
        name: "GC threshold impact on garbage-heavy workload".into(),
        status: "INFO",
        details,
    }
}

async fn exp54_linker_reuse_speedup() -> ExperimentResult {
    let guest_module = load_guest_module();
    let mut details = vec![];

    // Measure with shared linker (current arch after linter refactor)
    let runtime = SandCastle::new(Config::new(guest_module.clone())).unwrap();
    let iters = 500;
    let start = Instant::now();
    for _ in 0..iters {
        let _ = runtime.execute(ExecutionRequest::new("return 1;")).await;
    }
    let shared_time = start.elapsed();
    let shared_avg = shared_time / iters as u32;
    details.push(format!("Shared linker: {} iters in {:?} (avg={:?})", iters, shared_time, shared_avg));
    details.push(format!("Throughput: {:.0} ops/sec", iters as f64 / shared_time.as_secs_f64()));

    ExperimentResult {
        id: 54,
        category: "ARCH",
        name: "Linker reuse performance".into(),
        status: "INFO",
        details,
    }
}

async fn exp55_prototype_pollution_isolation(runtime: &SandCastle) -> ExperimentResult {
    // Execute code that tries to pollute Object.prototype
    let pollute = r#"
        Object.prototype.injected = "pwned";
        Array.prototype.evil = function() { return "evil"; };
        return { injected: ({}).injected, evil: [].evil() };
    "#;
    let result1 = runtime.execute(ExecutionRequest::new(pollute)).await.unwrap();

    // Execute in a fresh sandbox — pollution should NOT carry over
    let check = r#"
        return {
            hasInjected: "injected" in {},
            hasEvil: typeof [].evil === "function"
        };
    "#;
    let result2 = runtime.execute(ExecutionRequest::new(check)).await.unwrap();

    let isolated = match &result2.output {
        OutputValue::Json(v) => {
            v.get("hasInjected") == Some(&serde_json::json!(false))
                && v.get("hasEvil") == Some(&serde_json::json!(false))
        }
        _ => false,
    };

    ExperimentResult {
        id: 55,
        category: "SEC",
        name: "Prototype pollution doesn't leak between sandboxes".into(),
        status: if isolated { "PASS" } else { "FAIL" },
        details: vec![
            format!("Sandbox 1 (polluter): {:?}", result1.output),
            format!("Sandbox 2 (checker): {:?}", result2.output),
            format!("Isolated: {}", isolated),
        ],
    }
}

async fn exp56_global_state_isolation(runtime: Arc<SandCastle>) -> ExperimentResult {
    // Set a global in one sandbox
    let set_global = r#"
        globalThis.__secret = "sandbox_a_data";
        return globalThis.__secret;
    "#;
    let r1 = runtime.execute(ExecutionRequest::new(set_global)).await.unwrap();

    // Check it doesn't exist in another sandbox
    let check_global = r#"
        return {
            hasSecret: typeof globalThis.__secret !== "undefined",
            value: globalThis.__secret || null
        };
    "#;
    let r2 = runtime.execute(ExecutionRequest::new(check_global)).await.unwrap();

    let isolated = match &r2.output {
        OutputValue::Json(v) => v.get("hasSecret") == Some(&serde_json::json!(false)),
        _ => false,
    };

    ExperimentResult {
        id: 56,
        category: "SEC",
        name: "Global state isolation between sandboxes".into(),
        status: if isolated { "PASS" } else { "FAIL" },
        details: vec![
            format!("Sandbox 1 set: {:?}", r1.output),
            format!("Sandbox 2 check: {:?}", r2.output),
        ],
    }
}

async fn exp57_error_propagation_chain(runtime: &SandCastle) -> ExperimentResult {
    let mut details = vec![];
    let tests = [
        ("throw string", "throw 'raw string error';"),
        ("throw number", "throw 42;"),
        ("throw object", "throw { code: 'ERR_CUSTOM', msg: 'bad' };"),
        ("throw null", "throw null;"),
        ("nested throw", "function a() { throw new Error('deep'); } function b() { a(); } b();"),
        ("throw in catch", "try { throw 1; } catch(e) { throw new Error('re-thrown: ' + e); }"),
    ];
    let mut all_caught = true;
    for (label, code) in tests {
        let result = runtime.execute(ExecutionRequest::new(code)).await.unwrap();
        let caught = matches!(result.status, ExecutionStatus::GuestError { .. });
        if !caught { all_caught = false; }
        details.push(format!("[{}] {}: {:?}", if caught { "ok" } else { "FAIL" }, label, result.status));
    }
    ExperimentResult {
        id: 57,
        category: "FEAT",
        name: "Error propagation for non-Error throw values".into(),
        status: if all_caught { "PASS" } else { "FAIL" },
        details,
    }
}

async fn exp58_nested_capability_calls(runtime: &SandCastle) -> ExperimentResult {
    let mut registry = CapabilityRegistry::new();
    registry.register(Box::new(SimpleCapability::new("math", |method, input| {
        match method {
            "add" => {
                let a = input.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let b = input.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);
                Ok(serde_json::json!({"result": a + b}))
            }
            "multiply" => {
                let a = input.get("a").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let b = input.get("b").and_then(|v| v.as_f64()).unwrap_or(0.0);
                Ok(serde_json::json!({"result": a * b}))
            }
            _ => Err(sandcastle::error::CapabilityError::NotFound {
                capability: "math".into(), method: method.to_string(),
            }),
        }
    })));
    let caps = Arc::new(registry);

    let code = r#"
        function call(method, args) {
            const r = __sandcastle_host_call("math", method, JSON.stringify(args));
            return JSON.parse(r).result;
        }
        const sum = call("add", {a: 3, b: 4});
        const product = call("multiply", {a: sum, b: 5});
        const final_result = call("add", {a: product, b: 10});
        return final_result;
    "#;
    let result = runtime
        .execute(ExecutionRequest::new(code).with_capabilities(caps))
        .await
        .unwrap();

    let correct = matches!(&result.output, OutputValue::Json(v) if v == &serde_json::json!(45.0));
    ExperimentResult {
        id: 58,
        category: "FEAT",
        name: "Nested/chained capability calls".into(),
        status: if result.is_success() && correct { "PASS" } else { "FAIL" },
        details: vec![
            format!("Expected 45 (3+4=7, 7*5=35, 35+10=45)"),
            format!("Got: {:?}", result.output),
            format!("Capability calls: {}", result.transcript.capability_calls.len()),
        ],
    }
}

async fn exp59_promise_rejection_handling(runtime: &SandCastle) -> ExperimentResult {
    let tests = vec![
        ("rejected promise caught", r#"
            try {
                const p = Promise.reject(new Error("nope"));
                let err = null;
                p.catch(e => { err = e.message; });
                return err;
            } catch(e) {
                return "caught: " + e.message;
            }
        "#, Some("nope")),
        ("return rejected promise", r#"
            return Promise.reject(new Error("fail"));
        "#, None), // Should be a GuestError
    ];
    let mut details = vec![];
    let mut all_ok = true;
    for (label, code, expected_contains) in &tests {
        let result = runtime.execute(ExecutionRequest::new(*code)).await.unwrap();
        match expected_contains {
            Some(expected) => {
                let output_str = format!("{:?}", result.output);
                let ok = output_str.contains(expected);
                if !ok { all_ok = false; }
                details.push(format!("[{}] {}: {:?}", if ok { "ok" } else { "FAIL" }, label, result.output));
            }
            None => {
                // Should be a GuestError from rejected promise
                let ok = !result.is_success();
                if !ok { all_ok = false; }
                details.push(format!("[{}] {}: status={:?}", if ok { "ok" } else { "FAIL" }, label, result.status));
            }
        }
    }
    ExperimentResult {
        id: 59,
        category: "FEAT",
        name: "Promise rejection handling".into(),
        status: if all_ok { "PASS" } else { "WARN" },
        details,
    }
}

async fn exp60_async_chain_resolution(runtime: &SandCastle) -> ExperimentResult {
    let code = r#"
        async function step1() { return 10; }
        async function step2(x) { return x * 2; }
        async function step3(x) { return x + 5; }
        async function pipeline() {
            const a = await step1();
            const b = await step2(a);
            const c = await step3(b);
            return c;
        }
        return pipeline();
    "#;
    let result = runtime.execute(ExecutionRequest::new(code)).await.unwrap();
    let correct = matches!(&result.output, OutputValue::Json(v) if v == &serde_json::json!(25));

    ExperimentResult {
        id: 60,
        category: "FEAT",
        name: "Async/await chain resolution (10 -> *2 -> +5 = 25)".into(),
        status: if result.is_success() && correct { "PASS" } else { "FAIL" },
        details: vec![format!("Output: {:?}", result.output)],
    }
}

async fn exp61_json_circular_reference(runtime: &SandCastle) -> ExperimentResult {
    let code = r#"
        const obj = { a: 1 };
        obj.self = obj;
        try {
            JSON.stringify(obj);
            return "no error";
        } catch(e) {
            return { caught: true, type: e.constructor.name };
        }
    "#;
    let result = runtime.execute(ExecutionRequest::new(code)).await.unwrap();
    let caught = match &result.output {
        OutputValue::Json(v) => v.get("caught") == Some(&serde_json::json!(true)),
        _ => false,
    };
    ExperimentResult {
        id: 61,
        category: "FEAT",
        name: "JSON.stringify circular reference detection".into(),
        status: if caught { "PASS" } else { "FAIL" },
        details: vec![format!("Output: {:?}", result.output)],
    }
}

async fn exp62_eval_and_function_constructor(runtime: &SandCastle) -> ExperimentResult {
    let tests = vec![
        ("eval basic", r#"return eval("1 + 2");"#, serde_json::json!(3)),
        ("eval with scope", r#"
            const x = 10;
            return eval("x * 3");
        "#, serde_json::json!(30)),
        ("Function constructor", r#"
            const add = new Function("a", "b", "return a + b;");
            return add(5, 7);
        "#, serde_json::json!(12)),
    ];
    run_feature_tests(62, "eval() and Function constructor", tests, runtime).await
}

async fn exp63_symbol_behavior(runtime: &SandCastle) -> ExperimentResult {
    let tests = vec![
        ("Symbol creation", r#"
            const s = Symbol("test");
            return typeof s;
        "#, serde_json::json!("symbol")),
        ("Symbol as key", r#"
            const key = Symbol("key");
            const obj = {};
            obj[key] = 42;
            return obj[key];
        "#, serde_json::json!(42)),
        ("Symbol.for registry", r#"
            const s1 = Symbol.for("shared");
            const s2 = Symbol.for("shared");
            return s1 === s2;
        "#, serde_json::json!(true)),
    ];
    run_feature_tests(63, "Symbol behavior", tests, runtime).await
}

async fn exp64_proxy_and_reflect(runtime: &SandCastle) -> ExperimentResult {
    let tests = vec![
        ("Proxy get trap", r#"
            const handler = { get(target, prop) { return prop in target ? target[prop] : 42; }};
            const p = new Proxy({a: 1}, handler);
            return { a: p.a, missing: p.xyz };
        "#, serde_json::json!({"a": 1, "missing": 42})),
        ("Proxy set trap", r#"
            let log = [];
            const handler = { set(target, prop, value) { log.push(prop); target[prop] = value; return true; }};
            const p = new Proxy({}, handler);
            p.x = 1; p.y = 2;
            return { log, x: p.x, y: p.y };
        "#, serde_json::json!({"log": ["x", "y"], "x": 1, "y": 2})),
        ("Reflect.ownKeys", r#"
            const obj = { a: 1, b: 2 };
            return Reflect.ownKeys(obj);
        "#, serde_json::json!(["a", "b"])),
    ];
    run_feature_tests(64, "Proxy and Reflect", tests, runtime).await
}

async fn exp65_multi_artifact_round_trip(runtime: &SandCastle) -> ExperimentResult {
    // Write artifacts in sandbox 1, read them back in sandbox 2 (via input artifacts)
    let write_code = r#"
        globalThis.__sandcastle_write_artifact("config.json", JSON.stringify({version: 3, items: [1,2,3]}));
        globalThis.__sandcastle_write_artifact("data.csv", "name,value\nalice,10\nbob,20");
        return "written";
    "#;
    let r1 = runtime.execute(ExecutionRequest::new(write_code)).await.unwrap();

    let mut details = vec![format!("Write: {:?} ({} artifacts)", r1.status, r1.output_artifacts.len())];

    // Feed the output artifacts as input to a second sandbox
    let input_artifacts: Vec<Artifact> = r1.output_artifacts.iter().map(|a| {
        Artifact::new(&a.name, a.data.clone())
    }).collect();

    let read_code = r#"
        const config = JSON.parse(globalThis.__sandcastle_read_artifact("config.json"));
        const csv = globalThis.__sandcastle_read_artifact("data.csv");
        const lines = csv.split("\n").length;
        return { version: config.version, items: config.items, csvLines: lines };
    "#;
    let r2 = runtime
        .execute(ExecutionRequest::new(read_code).with_artifacts(input_artifacts))
        .await
        .unwrap();

    let correct = match &r2.output {
        OutputValue::Json(v) => {
            v.get("version") == Some(&serde_json::json!(3))
                && v.get("csvLines") == Some(&serde_json::json!(3))
        }
        _ => false,
    };
    details.push(format!("Read: {:?}", r2.output));

    ExperimentResult {
        id: 65,
        category: "FEAT",
        name: "Multi-artifact round trip (write -> read)".into(),
        status: if r1.is_success() && r2.is_success() && correct { "PASS" } else { "FAIL" },
        details,
    }
}

async fn exp66_capability_payload_sizes(runtime: &SandCastle) -> ExperimentResult {
    let mut details = vec![];
    let mut registry = CapabilityRegistry::new();
    registry.register(Box::new(SimpleCapability::new("echo", |_, input| Ok(input))));
    let caps = Arc::new(registry);

    for size_label in ["100B", "1KB", "10KB", "50KB"] {
        let n = match size_label {
            "100B" => 100,
            "1KB" => 1_000,
            "10KB" => 10_000,
            "50KB" => 50_000,
            _ => 0,
        };
        let code = format!(
            r#"
            const payload = JSON.stringify({{ data: "x".repeat({n}) }});
            const result = __sandcastle_host_call("echo", "echo", payload);
            return JSON.parse(result).data.length;
            "#
        );
        let iters = 50;
        let mut total = Duration::ZERO;
        for _ in 0..iters {
            let req = ExecutionRequest::new(&code).with_capabilities(caps.clone());
            let start = Instant::now();
            let _ = runtime.execute(req).await;
            total += start.elapsed();
        }
        let avg = total / iters as u32;
        details.push(format!("{} payload: avg={:?}", size_label, avg));
    }

    ExperimentResult {
        id: 66,
        category: "PERF",
        name: "Capability call with varying payload sizes".into(),
        status: "INFO",
        details,
    }
}

async fn exp67_fuel_vs_timeout_race(runtime: &SandCastle) -> ExperimentResult {
    let mut details = vec![];
    // Low fuel + long timeout — fuel should win
    let limits_fuel_wins = Limits {
        fuel: 200_000_000,
        timeout: Duration::from_secs(30),
        ..Limits::default()
    };
    let start = Instant::now();
    let r = runtime
        .execute(ExecutionRequest::new("while(true){}").with_limits(limits_fuel_wins))
        .await
        .unwrap();
    let elapsed = start.elapsed();
    let fuel_won = matches!(r.status, ExecutionStatus::FuelExhausted);
    details.push(format!(
        "Low fuel + long timeout: {:?} in {:?} (fuel won: {})",
        r.status, elapsed, fuel_won
    ));

    // High fuel + short timeout — timeout should win
    let limits_timeout_wins = Limits {
        fuel: 0, // unlimited
        timeout: Duration::from_millis(200),
        ..Limits::default()
    };
    let start = Instant::now();
    let r = runtime
        .execute(ExecutionRequest::new("while(true){}").with_limits(limits_timeout_wins))
        .await
        .unwrap();
    let elapsed = start.elapsed();
    let timeout_won = matches!(r.status, ExecutionStatus::Timeout);
    details.push(format!(
        "Unlimited fuel + 200ms timeout: {:?} in {:?} (timeout won: {})",
        r.status, elapsed, timeout_won
    ));

    ExperimentResult {
        id: 67,
        category: "ARCH",
        name: "Fuel vs timeout race condition".into(),
        status: if fuel_won && timeout_won { "PASS" } else { "FAIL" },
        details,
    }
}

async fn exp68_concurrent_dispatch(runtime: Arc<SandCastle>) -> ExperimentResult {
    let registry = ScriptRegistry::new(100);
    let caps = Arc::new(CapabilityRegistry::new());
    for i in 0..10 {
        registry.register(
            &format!("worker_{}", i),
            format!("return {{ worker: {}, result: (globalThis.__sandcastle_input || {{}}).x || 0 }};", i),
            caps.clone(),
            Limits::default(),
        ).unwrap();
    }

    let registry = Arc::new(registry);
    let start = Instant::now();
    let mut handles = Vec::new();
    for i in 0..50 {
        let rt = runtime.clone();
        let reg = registry.clone();
        handles.push(tokio::spawn(async move {
            let worker = format!("worker_{}", i % 10);
            let input = serde_json::json!({"x": i});
            rt.dispatch(&reg, &worker, input).await
        }));
    }
    let mut successes = 0;
    for h in handles {
        if let Ok(Ok(r)) = h.await {
            if r.is_success() { successes += 1; }
        }
    }
    let elapsed = start.elapsed();

    ExperimentResult {
        id: 68,
        category: "PERF",
        name: "50 concurrent dispatches across 10 scripts".into(),
        status: if successes == 50 { "PASS" } else { "WARN" },
        details: vec![
            format!("{}/50 succeeded in {:?}", successes, elapsed),
            format!("{:.0} dispatches/sec", 50.0 / elapsed.as_secs_f64()),
        ],
    }
}

async fn exp69_warmup_cold_vs_hot(runtime: &SandCastle) -> ExperimentResult {
    let code = "return 42;";
    let mut details = vec![];

    // Cold: first execution after runtime creation
    let cold_rt = SandCastle::new(Config::new(load_guest_module())).unwrap();
    let start = Instant::now();
    let _ = cold_rt.execute(ExecutionRequest::new(code)).await;
    let cold_time = start.elapsed();
    details.push(format!("Cold (first execution): {:?}", cold_time));

    // Hot: 100th execution on same runtime
    for _ in 0..99 {
        let _ = runtime.execute(ExecutionRequest::new(code)).await;
    }
    let start = Instant::now();
    let _ = runtime.execute(ExecutionRequest::new(code)).await;
    let hot_time = start.elapsed();
    details.push(format!("Hot (100th execution): {:?}", hot_time));
    details.push(format!("Cold/hot ratio: {:.2}x", cold_time.as_secs_f64() / hot_time.as_secs_f64()));

    ExperimentResult {
        id: 69,
        category: "PERF",
        name: "Cold start vs hot execution".into(),
        status: "INFO",
        details,
    }
}

async fn exp70_tail_latency_p99(runtime: Arc<SandCastle>) -> ExperimentResult {
    let iters = 1000;
    let code = "let s=0; for(let i=0;i<100;i++) s+=i; return s;";
    let mut times = Vec::with_capacity(iters);

    for _ in 0..iters {
        let start = Instant::now();
        let _ = runtime.execute(ExecutionRequest::new(code)).await;
        times.push(start.elapsed());
    }

    times.sort();
    let p50 = times[iters / 2];
    let p90 = times[(iters * 90) / 100];
    let p95 = times[(iters * 95) / 100];
    let p99 = times[(iters * 99) / 100];
    let p999 = times[(iters * 999) / 1000];
    let max = *times.last().unwrap();
    let avg = times.iter().sum::<Duration>() / iters as u32;

    ExperimentResult {
        id: 70,
        category: "PERF",
        name: format!("Tail latency distribution ({} samples)", iters),
        status: "INFO",
        details: vec![
            format!("avg={:?}", avg),
            format!("p50={:?}  p90={:?}  p95={:?}", p50, p90, p95),
            format!("p99={:?}  p99.9={:?}  max={:?}", p99, p999, max),
            format!("p99/p50 ratio: {:.2}x", p99.as_secs_f64() / p50.as_secs_f64()),
        ],
    }
}

// ============================================================================
// ROUND 3: USER-FACING SCENARIOS (71-80)
// ============================================================================

async fn exp71_kv_end_to_end(runtime: &SandCastle) -> ExperimentResult {
    use sandcastle::capabilities::KvCapability;

    let kv = KvCapability::default();
    let store = kv.store().clone();
    let mut registry = CapabilityRegistry::new();
    registry.register(Box::new(kv));
    let caps = Arc::new(registry);

    let code = r#"
        function kv(method, args) {
            return JSON.parse(__sandcastle_host_call("kv", method, JSON.stringify(args)));
        }
        kv("set", {key: "user:1", value: {name: "Alice", score: 95}});
        kv("set", {key: "user:2", value: {name: "Bob", score: 87}});
        kv("set", {key: "user:3", value: {name: "Carol", score: 92}});

        const keys = kv("list", {prefix: "user:"});
        const alice = kv("get", {key: "user:1"});
        const hasCarol = kv("has", {key: "user:3"});
        kv("delete", {key: "user:2"});
        const hasBob = kv("has", {key: "user:2"});
        const keysAfter = kv("list", {prefix: "user:"});

        return { keyCount: keys.length, alice: alice.name, hasCarol, hasBob, keysAfter: keysAfter.length };
    "#;
    let result = runtime
        .execute(ExecutionRequest::new(code).with_capabilities(caps))
        .await
        .unwrap();

    let correct = match &result.output {
        OutputValue::Json(v) => {
            v["keyCount"] == 3 && v["alice"] == "Alice" && v["hasCarol"] == true
                && v["hasBob"] == false && v["keysAfter"] == 2
        }
        _ => false,
    };
    // Verify host-side store
    let host_has_alice = store.contains_key("user:1");
    let host_no_bob = !store.contains_key("user:2");

    ExperimentResult {
        id: 71,
        category: "E2E",
        name: "KV capability end-to-end (set/get/list/delete/has)".into(),
        status: if result.is_success() && correct && host_has_alice && host_no_bob { "PASS" } else { "FAIL" },
        details: vec![
            format!("Output: {:?}", result.output),
            format!("Host store verified: alice={}, no_bob={}", host_has_alice, host_no_bob),
        ],
    }
}

async fn exp72_shared_kv_across_sandboxes(runtime: &SandCastle) -> ExperimentResult {
    use sandcastle::capabilities::KvCapability;

    let kv = KvCapability::default();
    let mut registry = CapabilityRegistry::new();
    registry.register(Box::new(kv));
    let caps = Arc::new(registry);

    // Sandbox 1: write data
    let write_code = r#"
        __sandcastle_host_call("kv", "set", JSON.stringify({key: "counter", value: 100}));
        return "written";
    "#;
    let r1 = runtime.execute(ExecutionRequest::new(write_code).with_capabilities(caps.clone())).await.unwrap();

    // Sandbox 2: read and modify (same caps = same store)
    let read_code = r#"
        const val = JSON.parse(__sandcastle_host_call("kv", "get", JSON.stringify({key: "counter"})));
        __sandcastle_host_call("kv", "set", JSON.stringify({key: "counter", value: val + 1}));
        return val;
    "#;
    let r2 = runtime.execute(ExecutionRequest::new(read_code).with_capabilities(caps.clone())).await.unwrap();

    // Sandbox 3: verify the increment
    let verify_code = r#"
        return JSON.parse(__sandcastle_host_call("kv", "get", JSON.stringify({key: "counter"})));
    "#;
    let r3 = runtime.execute(ExecutionRequest::new(verify_code).with_capabilities(caps)).await.unwrap();

    let all_ok = r1.is_success() && r2.is_success() && r3.is_success();
    let val2 = match &r2.output { OutputValue::Json(v) => v.as_f64(), _ => None };
    let val3 = match &r3.output { OutputValue::Json(v) => v.as_f64(), _ => None };

    ExperimentResult {
        id: 72,
        category: "E2E",
        name: "Shared KV store across sandboxes (write → read+modify → verify)".into(),
        status: if all_ok && val2 == Some(100.0) && val3 == Some(101.0) { "PASS" } else { "FAIL" },
        details: vec![
            format!("Sandbox 2 read: {:?}", r2.output),
            format!("Sandbox 3 verify: {:?}", r3.output),
        ],
    }
}

async fn exp73_script_registry_dispatch(runtime: &SandCastle) -> ExperimentResult {
    let registry = ScriptRegistry::new(100);
    let caps = Arc::new(CapabilityRegistry::new());

    registry.register("greet", "const i = globalThis.__sandcastle_input; return 'Hello ' + i.name + '!';", caps.clone(), Limits::default()).unwrap();
    registry.register("add", "const i = globalThis.__sandcastle_input; return i.a + i.b;", caps, Limits::default()).unwrap();

    let r1 = runtime.dispatch(&registry, "greet", serde_json::json!({"name": "World"})).await.unwrap();
    let r2 = runtime.dispatch(&registry, "add", serde_json::json!({"a": 10, "b": 32})).await.unwrap();

    // Dispatch same script multiple times — no state leak
    let r3 = runtime.dispatch(&registry, "greet", serde_json::json!({"name": "Agent"})).await.unwrap();

    // Dispatch nonexistent
    let r4 = runtime.dispatch(&registry, "missing", serde_json::Value::Null).await;

    let greet_ok = matches!(&r1.output, OutputValue::Json(v) if v == "Hello World!");
    let add_ok = matches!(&r2.output, OutputValue::Json(v) if v == 42);
    let greet2_ok = matches!(&r3.output, OutputValue::Json(v) if v == "Hello Agent!");
    let missing_err = r4.is_err();

    ExperimentResult {
        id: 73,
        category: "E2E",
        name: "Script registry: register, dispatch, state isolation, missing".into(),
        status: if greet_ok && add_ok && greet2_ok && missing_err { "PASS" } else { "FAIL" },
        details: vec![
            format!("greet: {:?}", r1.output),
            format!("add: {:?}", r2.output),
            format!("greet again: {:?}", r3.output),
            format!("missing: is_err={}", missing_err),
        ],
    }
}

async fn exp74_namespace_isolation_execution(runtime: &SandCastle) -> ExperimentResult {
    let caps = Arc::new(CapabilityRegistry::new());
    let manager = NamespaceManager::new(10);

    let ns_a = manager.create("acme", NamespaceLimits::default(), caps.clone()).unwrap();
    let ns_b = manager.create("globex", NamespaceLimits::default(), caps).unwrap();

    ns_a.register("compute", String::from("return 'acme:' + globalThis.__sandcastle_input.x;"), None).unwrap();
    ns_b.register("compute", String::from("return 'globex:' + globalThis.__sandcastle_input.x;"), None).unwrap();

    let sa = ns_a.get_script("compute").unwrap();
    let sb = ns_b.get_script("compute").unwrap();

    let ra = runtime.execute(
        ExecutionRequest::new(&sa.code).with_input(serde_json::json!({"x": 42})).with_capabilities(sa.capabilities.clone())
    ).await.unwrap();
    let rb = runtime.execute(
        ExecutionRequest::new(&sb.code).with_input(serde_json::json!({"x": 42})).with_capabilities(sb.capabilities.clone())
    ).await.unwrap();

    // Cross-check: namespace A can't see B's scripts
    let cross = ns_a.get_script("compute").is_some() && ns_b.get_script("compute").is_some();
    let a_no_b = ns_a.list_scripts().len() == 1 && ns_b.list_scripts().len() == 1;

    let a_ok = matches!(&ra.output, OutputValue::Json(v) if v == "acme:42");
    let b_ok = matches!(&rb.output, OutputValue::Json(v) if v == "globex:42");

    ExperimentResult {
        id: 74,
        category: "E2E",
        name: "Namespace isolation: same script name, different tenants".into(),
        status: if a_ok && b_ok && cross && a_no_b { "PASS" } else { "FAIL" },
        details: vec![
            format!("Tenant A: {:?}", ra.output),
            format!("Tenant B: {:?}", rb.output),
        ],
    }
}

async fn exp75_artifact_pipeline(runtime: &SandCastle) -> ExperimentResult {
    // Step 1: Process CSV artifact, produce JSON summary
    let csv = "product,price,qty\nWidget,9.99,100\nGadget,24.99,50\nDoohickey,4.99,200\n";
    let artifact = Artifact::new("sales.csv", csv.as_bytes().to_vec());

    let code = r#"
        const csv = globalThis.__sandcastle_read_artifact("sales.csv");
        const lines = csv.trim().split("\n");
        const headers = lines[0].split(",");
        const rows = lines.slice(1).map(line => {
            const vals = line.split(",");
            return { product: vals[0], price: parseFloat(vals[1]), qty: parseInt(vals[2]) };
        });
        const total = rows.reduce((s, r) => s + r.price * r.qty, 0);
        const summary = { rowCount: rows.length, totalRevenue: Math.round(total * 100) / 100, rows };
        globalThis.__sandcastle_write_artifact("summary.json", JSON.stringify(summary));
        return summary;
    "#;
    let result = runtime.execute(ExecutionRequest::new(code).with_artifacts(vec![artifact])).await.unwrap();

    let correct = match &result.output {
        OutputValue::Json(v) => v["rowCount"] == 3 && v.get("totalRevenue").is_some(),
        _ => false,
    };
    let has_artifact = result.output_artifacts.len() == 1 && result.output_artifacts[0].name == "summary.json";

    // Parse the artifact and verify it's valid JSON
    let artifact_valid = result.output_artifacts.first()
        .and_then(|a| serde_json::from_slice::<serde_json::Value>(&a.data).ok())
        .map(|v| v["rowCount"] == 3)
        .unwrap_or(false);

    ExperimentResult {
        id: 75,
        category: "E2E",
        name: "Artifact pipeline: CSV input → process → JSON output".into(),
        status: if result.is_success() && correct && has_artifact && artifact_valid { "PASS" } else { "FAIL" },
        details: vec![
            format!("Output: {:?}", result.output),
            format!("Artifacts: {}", result.output_artifacts.len()),
            format!("Artifact valid JSON: {}", artifact_valid),
        ],
    }
}

async fn exp76_error_recovery_agent_pattern(runtime: &SandCastle) -> ExperimentResult {
    // Simulates an LLM agent that generates code, fails, adapts
    let bad_code = "return JSON.parse('{invalid');"; // Will throw
    let r1 = runtime.execute(ExecutionRequest::new(bad_code)).await.unwrap();

    let good_code = r#"
        try {
            return JSON.parse('{"valid": true}');
        } catch(e) {
            return { error: e.message };
        }
    "#;
    let r2 = runtime.execute(ExecutionRequest::new(good_code)).await.unwrap();

    // The "agent" can observe the error from attempt 1 and succeed on attempt 2
    let attempt1_failed = !r1.is_success();
    let attempt2_ok = r2.is_success() && matches!(&r2.output, OutputValue::Json(v) if v["valid"] == true);

    ExperimentResult {
        id: 76,
        category: "E2E",
        name: "Agent error recovery: fail → observe → retry with fix".into(),
        status: if attempt1_failed && attempt2_ok { "PASS" } else { "FAIL" },
        details: vec![
            format!("Attempt 1 (bad): {:?}", r1.status),
            format!("Attempt 2 (fixed): {:?}", r2.output),
        ],
    }
}

async fn exp77_prd_globals_availability(runtime: &SandCastle) -> ExperimentResult {
    let code = r#"
        const results = {};

        // JSON
        results.JSON = typeof JSON.parse === 'function' && typeof JSON.stringify === 'function';

        // Math
        results.Math = typeof Math.PI === 'number' && typeof Math.random === 'function';

        // Date
        results.Date = typeof Date.now === 'function' && typeof new Date().toISOString === 'function';

        // TextEncoder / TextDecoder
        results.TextEncoder = typeof TextEncoder !== 'undefined';
        results.TextDecoder = typeof TextDecoder !== 'undefined';

        // URL
        results.URL = typeof URL !== 'undefined';

        // atob / btoa
        results.atob = typeof atob === 'function';
        results.btoa = typeof btoa === 'function';

        // crypto
        results.crypto = typeof crypto !== 'undefined';
        results.cryptoRandomUUID = typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function';
        results.cryptoGetRandomValues = typeof crypto !== 'undefined' && typeof crypto.getRandomValues === 'function';

        // Map, Set, Promise, Proxy, Symbol, Reflect
        results.Map = typeof Map !== 'undefined';
        results.Set = typeof Set !== 'undefined';
        results.Promise = typeof Promise !== 'undefined';
        results.Proxy = typeof Proxy !== 'undefined';
        results.Symbol = typeof Symbol !== 'undefined';
        results.Reflect = typeof Reflect !== 'undefined';

        // TypedArrays
        results.Uint8Array = typeof Uint8Array !== 'undefined';
        results.Float64Array = typeof Float64Array !== 'undefined';
        results.ArrayBuffer = typeof ArrayBuffer !== 'undefined';
        results.DataView = typeof DataView !== 'undefined';

        return results;
    "#;
    let result = runtime.execute(ExecutionRequest::new(code)).await.unwrap();
    let mut details = vec![];
    let mut missing = vec![];

    if let OutputValue::Json(v) = &result.output {
        if let Some(obj) = v.as_object() {
            for (key, val) in obj {
                let available = val.as_bool().unwrap_or(false);
                let marker = if available { "ok" } else { "MISSING" };
                details.push(format!("[{}] {}", marker, key));
                if !available { missing.push(key.clone()); }
            }
        }
    }

    ExperimentResult {
        id: 77,
        category: "E2E",
        name: "PRD-claimed globals availability audit".into(),
        status: if missing.is_empty() { "PASS" } else { "WARN" },
        details: {
            if !missing.is_empty() {
                details.push(format!("Missing: {:?}", missing));
            }
            details
        },
    }
}

async fn exp78_large_artifact_boundary(runtime: &SandCastle) -> ExperimentResult {
    let mut details = vec![];
    // Test writing artifacts at various sizes
    for (label, size) in [("1KB", 1024), ("100KB", 102400), ("1MB", 1048576), ("4MB", 4194304)] {
        let code = format!(
            r#"globalThis.__sandcastle_write_artifact("data.bin", "x".repeat({})); return "ok";"#,
            size
        );
        let result = runtime.execute(ExecutionRequest::new(&code)).await.unwrap();
        let artifact_size = result.output_artifacts.first().map(|a| a.data.len()).unwrap_or(0);
        details.push(format!(
            "{}: status={:?} artifact_bytes={}",
            label,
            result.status,
            artifact_size
        ));
    }

    ExperimentResult {
        id: 78,
        category: "STRESS",
        name: "Large artifact write boundary (1KB to 4MB)".into(),
        status: "INFO",
        details,
    }
}

async fn exp79_multi_capability_pipeline(runtime: &SandCastle) -> ExperimentResult {
    use sandcastle::capabilities::KvCapability;

    // Wire up KV + a mock "transform" capability
    let kv = KvCapability::default();
    let mut registry = CapabilityRegistry::new();
    registry.register(Box::new(kv));
    registry.register(Box::new(SimpleCapability::new("transform", |method, input| {
        match method {
            "uppercase" => {
                let text = input.get("text").and_then(|v| v.as_str()).unwrap_or("");
                Ok(serde_json::json!({"result": text.to_uppercase()}))
            }
            "summarize" => {
                let items = input.get("items").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
                Ok(serde_json::json!({"count": items, "summary": format!("{} items processed", items)}))
            }
            _ => Ok(serde_json::Value::Null),
        }
    })));
    let caps = Arc::new(registry);

    let code = r#"
        function kv(method, args) { return JSON.parse(__sandcastle_host_call("kv", method, JSON.stringify(args))); }
        function transform(method, args) { return JSON.parse(__sandcastle_host_call("transform", method, JSON.stringify(args))); }

        // Pipeline: store data → transform → store result → summarize
        const names = ["alice", "bob", "carol"];
        for (const name of names) {
            const upper = transform("uppercase", {text: name});
            kv("set", {key: "processed:" + name, value: upper.result});
        }

        const keys = kv("list", {prefix: "processed:"});
        const summary = transform("summarize", {items: keys});

        return { stored: keys.length, summary: summary.summary };
    "#;
    let result = runtime.execute(ExecutionRequest::new(code).with_capabilities(caps)).await.unwrap();

    let correct = match &result.output {
        OutputValue::Json(v) => v["stored"] == 3 && v["summary"] == "3 items processed",
        _ => false,
    };

    ExperimentResult {
        id: 79,
        category: "E2E",
        name: "Multi-capability pipeline: KV + transform chained".into(),
        status: if result.is_success() && correct { "PASS" } else { "FAIL" },
        details: vec![
            format!("Output: {:?}", result.output),
            format!("Capability calls: {}", result.transcript.capability_calls.len()),
        ],
    }
}

async fn exp80_concurrent_kv_contention(runtime: Arc<SandCastle>) -> ExperimentResult {
    use sandcastle::capabilities::KvCapability;

    let kv = KvCapability::default();
    let store = kv.store().clone();
    let mut registry = CapabilityRegistry::new();
    registry.register(Box::new(kv));
    let caps = Arc::new(registry);

    // 20 concurrent sandboxes all incrementing the same key
    // (no atomic increment, so we expect some lost updates — this tests contention behavior)
    store.insert("counter".to_string(), serde_json::json!(0));

    let mut handles = Vec::new();
    for _ in 0..20 {
        let rt = runtime.clone();
        let c = caps.clone();
        handles.push(tokio::spawn(async move {
            let code = r#"
                const val = JSON.parse(__sandcastle_host_call("kv", "get", JSON.stringify({key: "counter"})));
                __sandcastle_host_call("kv", "set", JSON.stringify({key: "counter", value: val + 1}));
                return val;
            "#;
            rt.execute(ExecutionRequest::new(code).with_capabilities(c)).await
        }));
    }

    let mut successes = 0;
    for h in handles {
        if let Ok(Ok(r)) = h.await {
            if r.is_success() { successes += 1; }
        }
    }

    let final_val = store.get("counter").map(|v| v.clone()).unwrap_or(serde_json::json!(-1));

    ExperimentResult {
        id: 80,
        category: "E2E",
        name: "Concurrent KV contention (20 sandboxes, non-atomic increment)".into(),
        status: if successes == 20 { "PASS" } else { "WARN" },
        details: vec![
            format!("{}/20 succeeded", successes),
            format!("Final counter: {} (expected <=20 due to race conditions)", final_val),
            format!("Note: non-atomic read-modify-write means lost updates are expected"),
        ],
    }
}

// ============================================================================
// MAIN
// ============================================================================

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let total_start = Instant::now();

    println!("\n{}", "=".repeat(70));
    println!("  SandCastle Experiment Suite — 50 Experiments");
    println!("{}\n", "=".repeat(70));

    let runtime = Arc::new(create_runtime());

    // Warmup
    println!("Warming up...");
    let _ = runtime.execute(ExecutionRequest::new("return null;")).await;
    println!();

    let mut results: Vec<ExperimentResult> = Vec::new();

    // ---- PERFORMANCE ----
    println!("--- PERFORMANCE EXPERIMENTS (1-15) ---\n");

    let r = exp01_baseline_throughput(&runtime).await;
    r.print();
    results.push(r);

    let r = exp02_simple_expression_throughput(&runtime).await;
    r.print();
    results.push(r);

    let r = exp03_memory_limit_impact(&runtime).await;
    r.print();
    results.push(r);

    let r = exp04_fuel_limit_impact(&runtime).await;
    r.print();
    results.push(r);

    let r = exp05_concurrency_scaling(runtime.clone()).await;
    r.print();
    results.push(r);

    let r = exp06_input_payload_size(&runtime).await;
    r.print();
    results.push(r);

    let r = exp07_output_payload_size(&runtime).await;
    r.print();
    results.push(r);

    let r = exp08_code_size_impact(&runtime).await;
    r.print();
    results.push(r);

    let r = exp09_json_processing_complexity(&runtime).await;
    r.print();
    results.push(r);

    let r = exp10_console_output_overhead(&runtime).await;
    r.print();
    results.push(r);

    let r = exp11_artifact_io_throughput(&runtime).await;
    r.print();
    results.push(r);

    let r = exp12_capability_call_overhead(&runtime).await;
    r.print();
    results.push(r);

    let r = exp13_fuel_consumption_correlation(&runtime).await;
    r.print();
    results.push(r);

    let r = exp14_sequential_vs_concurrent(runtime.clone()).await;
    r.print();
    results.push(r);

    let r = exp15_sustained_throughput(&runtime).await;
    r.print();
    results.push(r);

    // ---- FEATURES ----
    println!("\n--- FEATURE EXPERIMENTS (16-27) ---\n");

    let r = exp16_closures_and_scoping(&runtime).await;
    r.print();
    results.push(r);

    let r = exp17_generators_and_iterators(&runtime).await;
    r.print();
    results.push(r);

    let r = exp18_regexp_capabilities(&runtime).await;
    r.print();
    results.push(r);

    let r = exp19_json_edge_cases(&runtime).await;
    r.print();
    results.push(r);

    let r = exp20_math_library(&runtime).await;
    r.print();
    results.push(r);

    let r = exp21_string_array_methods(&runtime).await;
    r.print();
    results.push(r);

    let r = exp22_map_set_weakref(&runtime).await;
    r.print();
    results.push(r);

    let r = exp23_promise_behavior(&runtime).await;
    r.print();
    results.push(r);

    let r = exp24_typed_arrays(&runtime).await;
    r.print();
    results.push(r);

    let r = exp25_error_types(&runtime).await;
    r.print();
    results.push(r);

    let r = exp26_destructuring_and_spread(&runtime).await;
    r.print();
    results.push(r);

    let r = exp27_date_handling(&runtime).await;
    r.print();
    results.push(r);

    // ---- STRESS ----
    println!("\n--- STRESS/RELIABILITY EXPERIMENTS (28-40) ---\n");

    let r = exp28_infinite_loop_fuel(&runtime).await;
    r.print();
    results.push(r);

    let r = exp29_infinite_recursion(&runtime).await;
    r.print();
    results.push(r);

    let r = exp30_memory_bomb(&runtime).await;
    r.print();
    results.push(r);

    let r = exp31_deeply_nested_objects(&runtime).await;
    r.print();
    results.push(r);

    let r = exp32_redos_attack(&runtime).await;
    r.print();
    results.push(r);

    let r = exp33_rapid_create_destroy(&runtime).await;
    r.print();
    results.push(r);

    let r = exp34_max_concurrency_limit().await;
    r.print();
    results.push(r);

    let r = exp35_huge_input(&runtime).await;
    r.print();
    results.push(r);

    let r = exp36_console_spam(&runtime).await;
    r.print();
    results.push(r);

    let r = exp37_capability_quota_exhaustion(&runtime).await;
    r.print();
    results.push(r);

    let r = exp38_malformed_javascript(&runtime).await;
    r.print();
    results.push(r);

    let r = exp39_unicode_edge_cases(&runtime).await;
    r.print();
    results.push(r);

    let r = exp40_output_artifact_stress(&runtime).await;
    r.print();
    results.push(r);

    // ---- ARCHITECTURE ----
    println!("\n--- ARCHITECTURE EXPERIMENTS (41-50) ---\n");

    let r = exp41_module_compilation_cost().await;
    r.print();
    results.push(r);

    let r = exp42_fuel_precision(&runtime).await;
    r.print();
    results.push(r);

    let r = exp43_epoch_timeout_precision(&runtime).await;
    r.print();
    results.push(r);

    let r = exp44_memory_enforcement_precision(&runtime).await;
    r.print();
    results.push(r);

    let r = exp45_capability_concurrency_limits(&runtime).await;
    r.print();
    results.push(r);

    let r = exp46_namespace_isolation(&runtime).await;
    r.print();
    results.push(r);

    let r = exp47_registry_performance_at_scale(&runtime).await;
    r.print();
    results.push(r);

    let r = exp48_transcript_overhead(&runtime).await;
    r.print();
    results.push(r);

    let r = exp49_store_creation_overhead().await;
    r.print();
    results.push(r);

    let r = exp50_execution_determinism(&runtime).await;
    r.print();
    results.push(r);

    // ---- ROUND 2 ----
    println!("\n--- ROUND 2: EXPERIMENTS (51-70) ---\n");

    let r = exp51_memory_exceeded_status(&runtime).await;
    r.print(); results.push(r);

    let r = exp52_trap_on_grow_failure_classification(&runtime).await;
    r.print(); results.push(r);

    let r = exp53_gc_threshold_impact(&runtime).await;
    r.print(); results.push(r);

    let r = exp54_linker_reuse_speedup().await;
    r.print(); results.push(r);

    let r = exp55_prototype_pollution_isolation(&runtime).await;
    r.print(); results.push(r);

    let r = exp56_global_state_isolation(runtime.clone()).await;
    r.print(); results.push(r);

    let r = exp57_error_propagation_chain(&runtime).await;
    r.print(); results.push(r);

    let r = exp58_nested_capability_calls(&runtime).await;
    r.print(); results.push(r);

    let r = exp59_promise_rejection_handling(&runtime).await;
    r.print(); results.push(r);

    let r = exp60_async_chain_resolution(&runtime).await;
    r.print(); results.push(r);

    let r = exp61_json_circular_reference(&runtime).await;
    r.print(); results.push(r);

    let r = exp62_eval_and_function_constructor(&runtime).await;
    r.print(); results.push(r);

    let r = exp63_symbol_behavior(&runtime).await;
    r.print(); results.push(r);

    let r = exp64_proxy_and_reflect(&runtime).await;
    r.print(); results.push(r);

    let r = exp65_multi_artifact_round_trip(&runtime).await;
    r.print(); results.push(r);

    let r = exp66_capability_payload_sizes(&runtime).await;
    r.print(); results.push(r);

    let r = exp67_fuel_vs_timeout_race(&runtime).await;
    r.print(); results.push(r);

    let r = exp68_concurrent_dispatch(runtime.clone()).await;
    r.print(); results.push(r);

    let r = exp69_warmup_cold_vs_hot(&runtime).await;
    r.print(); results.push(r);

    let r = exp70_tail_latency_p99(runtime.clone()).await;
    r.print(); results.push(r);

    // ---- ROUND 3: USER-FACING ----
    println!("\n--- ROUND 3: USER-FACING SCENARIOS (71-80) ---\n");

    let r = exp71_kv_end_to_end(&runtime).await;
    r.print(); results.push(r);

    let r = exp72_shared_kv_across_sandboxes(&runtime).await;
    r.print(); results.push(r);

    let r = exp73_script_registry_dispatch(&runtime).await;
    r.print(); results.push(r);

    let r = exp74_namespace_isolation_execution(&runtime).await;
    r.print(); results.push(r);

    let r = exp75_artifact_pipeline(&runtime).await;
    r.print(); results.push(r);

    let r = exp76_error_recovery_agent_pattern(&runtime).await;
    r.print(); results.push(r);

    let r = exp77_prd_globals_availability(&runtime).await;
    r.print(); results.push(r);

    let r = exp78_large_artifact_boundary(&runtime).await;
    r.print(); results.push(r);

    let r = exp79_multi_capability_pipeline(&runtime).await;
    r.print(); results.push(r);

    let r = exp80_concurrent_kv_contention(runtime.clone()).await;
    r.print(); results.push(r);

    // ---- SUMMARY ----
    println!("\n{}", "=".repeat(70));
    println!("  SUMMARY");
    println!("{}\n", "=".repeat(70));

    let pass = results.iter().filter(|r| r.status == "PASS").count();
    let fail = results.iter().filter(|r| r.status == "FAIL").count();
    let warn = results.iter().filter(|r| r.status == "WARN").count();
    let info = results.iter().filter(|r| r.status == "INFO").count();

    println!(
        "  \x1b[32mPASS: {}\x1b[0m  \x1b[31mFAIL: {}\x1b[0m  \x1b[33mWARN: {}\x1b[0m  \x1b[36mINFO: {}\x1b[0m  Total: {}",
        pass, fail, warn, info, results.len()
    );
    println!("  Total time: {:?}", total_start.elapsed());

    if fail > 0 {
        println!("\n  Failed experiments:");
        for r in results.iter().filter(|r| r.status == "FAIL") {
            println!("    - [{}] {}", r.id, r.name);
        }
    }
    if warn > 0 {
        println!("\n  Warnings:");
        for r in results.iter().filter(|r| r.status == "WARN") {
            println!("    - [{}] {}", r.id, r.name);
        }
    }

    println!();
}
