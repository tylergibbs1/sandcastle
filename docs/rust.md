# Rust Library Mode

Use Rust when you want to embed the runtime directly instead of spawning the CLI.

## Minimal example

```rust
use sandcastle::runtime::{Config, SandCastle};
use sandcastle::sandbox::ExecutionRequest;

let guest_module = std::fs::read("guest-js.wasm")?;
let runtime = SandCastle::new(Config::new(guest_module))?;

let result = runtime.execute(
    ExecutionRequest::new("return 1 + 1;")
).await?;

println!("{:?}", result.output); // Json(2)
```

## Good fit

- You want direct embedding in a Rust service.
- You want host capabilities implemented in Rust.
- You want full control over runtime configuration and lifecycle.
