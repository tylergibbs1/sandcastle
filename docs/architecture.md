# SandCastle Architecture

## High-Level Overview

```mermaid
graph TB
    subgraph "Host Application"
        APP[Your App / Agent Framework]

        subgraph "SandCastle Runtime"
            ENGINE[Wasmtime Engine<br/>AOT Compilation]
            MODULE[Pre-compiled QuickJS<br/>WASM Module ~823KB]

            subgraph "Sandbox Pool"
                S1[Sandbox 1<br/>Wasmtime Store]
                S2[Sandbox 2<br/>Wasmtime Store]
                S3[Sandbox N<br/>Wasmtime Store]
            end

            REG[Script Registry<br/>Named Scripts]
            NS[Namespace Manager<br/>Multi-tenant Dispatch]
        end

        subgraph "Host Capabilities"
            KV[KV Store<br/>DashMap]
            HTTP[HTTP Client<br/>reqwest]
            CUSTOM[Custom Capabilities<br/>Capability trait]
        end

        subgraph "Delivery Modes"
            LIB[Library Mode<br/>Rust embed]
            CLI[CLI Tool<br/>sandcastle run]
            SERVE[HTTP Server<br/>axum REST API]
        end
    end

    subgraph "TypeScript SDK"
        CLIENT[SandCastle Client]
        CODEMODE[Code Mode<br/>createCodeTool]
        TRANSPORT_SUB[Subprocess<br/>Transport]
        TRANSPORT_HTTP[HTTP<br/>Transport]
    end

    subgraph "AI Agent"
        LLM[LLM<br/>Claude / GPT / etc.]
        TOOLS[Tool Definitions]
    end

    LLM -->|"writes code"| CODEMODE
    CODEMODE -->|"executes"| CLIENT
    CLIENT -->|subprocess| TRANSPORT_SUB --> CLI
    CLIENT -->|http| TRANSPORT_HTTP --> SERVE

    APP --> LIB
    LIB --> ENGINE
    CLI --> ENGINE
    SERVE --> ENGINE

    ENGINE --> MODULE
    MODULE --> S1 & S2 & S3

    S1 & S2 & S3 -->|"host calls"| KV & HTTP & CUSTOM

    REG -->|"lookup"| S1
    NS -->|"dispatch"| REG
```

## Sandbox Execution Flow

```mermaid
sequenceDiagram
    participant Agent as AI Agent
    participant SDK as TypeScript SDK
    participant CLI as SandCastle CLI
    participant RT as Wasmtime Runtime
    participant QJS as QuickJS (WASM)
    participant CAP as Host Capabilities

    Agent->>SDK: sc.execute({ code, input })
    SDK->>CLI: spawn process (code via stdin)
    CLI->>RT: Create Store + set fuel/memory/epoch
    RT->>QJS: Instantiate from pre-compiled module
    QJS->>QJS: Execute guest JavaScript

    alt Guest calls host capability
        QJS->>RT: __sandcastle_host_call(cap, method, payload)
        RT->>CAP: dispatch_sync(cap, method, payload)
        CAP-->>RT: Result (JSON)
        RT-->>QJS: Result bytes
    end

    alt Guest logs to console
        QJS->>RT: __sandcastle_console(level, message)
        RT->>RT: Record in TranscriptRecorder
    end

    QJS->>RT: __sandcastle_set_output(result)
    RT-->>CLI: ExecutionResult + Transcript
    CLI-->>SDK: JSON transcript (stdout)
    SDK-->>Agent: ExecutionResult { ok, output, transcript }
```

## Code Mode Flow (Two-Pass)

```mermaid
sequenceDiagram
    participant LLM as Claude / LLM
    participant CM as Code Mode
    participant SB as SandCastle Sandbox
    participant HOST as Host Tools

    LLM->>CM: codemode tool_use { code: "async () => {<br/>  const user = await codemode.getUser({id: 42});<br/>  return user.name;<br/>}" }

    Note over CM: Pass 1: Collect tool calls
    CM->>SB: Execute with collector proxy
    SB-->>CM: { __codemode_calls: [{ tool: "getUser", args: {id: 42} }] }

    Note over CM: Pass 2: Execute tools host-side
    CM->>HOST: getUser({ id: 42 })
    HOST-->>CM: { id: 42, name: "Alice", email: "alice@example.com" }

    Note over CM: Pass 3: Replay with real results
    CM->>SB: Execute with pre-populated results
    SB-->>CM: "Alice"

    CM-->>LLM: tool_result: "Alice"
```

## Multi-Tenant Dispatch

```mermaid
graph LR
    subgraph "Namespace Manager"
        NM[NamespaceManager]
    end

    subgraph "Namespace: tenant-abc"
        NS1[DispatchNamespace]
        REG1[ScriptRegistry]
        SEM1[Concurrency<br/>Semaphore: 100]

        W1[worker-1<br/>code + limits]
        W2[worker-2<br/>code + limits]
    end

    subgraph "Namespace: tenant-xyz"
        NS2[DispatchNamespace]
        REG2[ScriptRegistry]
        SEM2[Concurrency<br/>Semaphore: 50]

        W3[api-handler<br/>code + limits]
    end

    NM --> NS1 & NS2
    NS1 --> REG1
    NS1 --> SEM1
    REG1 --> W1 & W2
    NS2 --> REG2
    NS2 --> SEM2
    REG2 --> W3
```

## Security Boundary

```mermaid
graph TB
    subgraph "Trusted: Host"
        HOST_APP[Host Application]
        CAP_BRIDGE[Capability Bridge<br/>Quota Enforcement]
        CRED[Credential Store<br/>Never exposed to guest]
    end

    subgraph "WASM Sandbox Boundary"
        subgraph "Untrusted: Guest"
            GUEST[AI-Generated JavaScript]
            QJS_RT[QuickJS Runtime]
        end
    end

    subgraph "Resource Limits"
        FUEL[Fuel Metering<br/>Instruction count cap]
        MEM[Memory Limit<br/>Wasmtime enforced]
        EPOCH[Epoch Timeout<br/>Wall-clock deadline]
        QUOTA[Capability Quotas<br/>Per-call limits]
    end

    HOST_APP -->|"mediated access only"| CAP_BRIDGE
    CAP_BRIDGE -->|"validated RPC"| GUEST
    CRED -->|"injected by host"| CAP_BRIDGE

    FUEL -.->|"enforces"| GUEST
    MEM -.->|"enforces"| QJS_RT
    EPOCH -.->|"enforces"| GUEST
    QUOTA -.->|"enforces"| CAP_BRIDGE

    style GUEST fill:#fee,stroke:#c00
    style QJS_RT fill:#fee,stroke:#c00
    style HOST_APP fill:#efe,stroke:#0a0
    style CAP_BRIDGE fill:#efe,stroke:#0a0
    style CRED fill:#efe,stroke:#0a0
```
