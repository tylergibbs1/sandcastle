#!/bin/bash
# CLI integration tests for sandcastle
# Run with: bash tests/cli_integration.sh

set -e

SANDCASTLE="./target/release/sandcastle"
PASS=0
FAIL=0
TOTAL=0

pass() { PASS=$((PASS + 1)); TOTAL=$((TOTAL + 1)); echo "  ✓ $1"; }
fail() { FAIL=$((FAIL + 1)); TOTAL=$((TOTAL + 1)); echo "  ✗ $1: $2"; }

check() {
    local name="$1"
    shift
    if eval "$@" > /dev/null 2>&1; then
        pass "$name"
    else
        fail "$name" "command failed"
    fi
}

check_output() {
    local name="$1"
    local expected="$2"
    shift 2
    local output
    output=$(eval "$@" 2>/dev/null) || { fail "$name" "command failed"; return; }
    if echo "$output" | grep -q "$expected"; then
        pass "$name"
    else
        fail "$name" "expected '$expected', got: $(echo "$output" | head -1)"
    fi
}

check_fail() {
    local name="$1"
    shift
    if eval "$@" > /dev/null 2>&1; then
        fail "$name" "expected failure but succeeded"
    else
        pass "$name"
    fi
}

echo "SandCastle CLI Integration Tests"
echo "================================"
echo ""

# --- Basic execution ---
echo "Basic execution:"

check_output "run simple expression" "2" \
    "echo 'return 1 + 1;' | $SANDCASTLE run /dev/stdin"

check_output "run with JSON input" "Alice" \
    "echo 'return globalThis.__sandcastle_input.name;' | $SANDCASTLE run /dev/stdin --input '{\"name\":\"Alice\"}'"

check_output "input shorthand works" "Bob" \
    "echo 'return input.name;' | $SANDCASTLE run /dev/stdin --input '{\"name\":\"Bob\"}'"

check_output "return object" "hello" \
    "echo 'return {msg: \"hello\"};' | $SANDCASTLE run /dev/stdin"

check_output "return array" '1,' \
    "echo 'return [1, 2, 3];' | $SANDCASTLE run /dev/stdin"

echo ""

# --- Environment variables ---
echo "Environment variables:"

check_output "env injection via --env" "sk-test" \
    "echo 'return process.env.API_KEY;' | $SANDCASTLE run /dev/stdin -e API_KEY=sk-test"

check_output "multiple env vars" "true" \
    "echo 'return process.env.A === \"1\" && process.env.B === \"2\";' | $SANDCASTLE run /dev/stdin -e A=1 -e B=2"

check_output "env vars dont leak to input" "null" \
    "echo 'return input;' | $SANDCASTLE run /dev/stdin -e SECRET=hidden"

echo ""

# --- Limits ---
echo "Limits:"

check_fail "fuel exhaustion stops infinite loop" \
    "echo 'while(true){}' | $SANDCASTLE run /dev/stdin --fuel 1000000"

check_fail "timeout stops execution" \
    "echo 'while(true){}' | $SANDCASTLE run /dev/stdin --timeout 1 --fuel 0"

echo ""

# --- Error handling ---
echo "Error handling:"

check_fail "syntax error fails" \
    "echo 'function {' | $SANDCASTLE run /dev/stdin"

check_fail "throw fails" \
    "echo 'throw new Error(\"boom\");' | $SANDCASTLE run /dev/stdin"

check_fail "nonexistent file fails" \
    "$SANDCASTLE run /nonexistent/path.js"

echo ""

# --- Polyfills and globals ---
echo "Polyfills and globals:"

check_output "JSON works" "true" \
    "echo 'return JSON.parse(JSON.stringify({a:1})).a === 1;' | $SANDCASTLE run /dev/stdin"

check_output "Math works" "5" \
    "echo 'return Math.max(1,5,3);' | $SANDCASTLE run /dev/stdin"

check_output "Date works" "number" \
    "echo 'return typeof Date.now();' | $SANDCASTLE run /dev/stdin"

check_output "crypto.randomUUID works" "true" \
    "echo 'return crypto.randomUUID().length === 36;' | $SANDCASTLE run /dev/stdin"

check_output "setTimeout works" "true" \
    "echo 'let r=false; setTimeout(()=>{r=true;}); return r;' | $SANDCASTLE run /dev/stdin"

check_output "structuredClone works" "true" \
    "echo 'const a={x:1}; const b=structuredClone(a); b.x=2; return a.x===1;' | $SANDCASTLE run /dev/stdin"

check_output "URL works" "example.com" \
    "echo 'return new URL(\"https://example.com/path\").hostname;' | $SANDCASTLE run /dev/stdin"

echo ""

# --- Module shims ---
echo "Module shims:"

check_output "require lodash works" "2" \
    "echo 'const _=require(\"lodash\"); return Object.keys(_.groupBy([{t:\"a\"},{t:\"b\"},{t:\"a\"}],\"t\")).length;' | $SANDCASTLE run /dev/stdin"

check_output "require path works" "/foo/bar" \
    "echo 'return require(\"path\").join(\"/foo\",\"bar\");' | $SANDCASTLE run /dev/stdin"

check_output "require uuid works" "true" \
    "echo 'return typeof require(\"uuid\").v4() === \"string\";' | $SANDCASTLE run /dev/stdin"

check_output "require unknown gives clear error" "SandCastle sandbox" \
    "echo 'try{require(\"express\")}catch(e){return e.message;}' | $SANDCASTLE run /dev/stdin"

echo ""

# --- ES2024+ features ---
echo "ES2024+ features:"

check_output "Object.groupBy works" "true" \
    "echo 'return Object.groupBy([{t:\"a\"},{t:\"b\"}], i=>i.t).a.length === 1;' | $SANDCASTLE run /dev/stdin --fuel 5000000000"

check_output "Set.intersection works" "true" \
    "echo 'return [...new Set([1,2,3]).intersection(new Set([2,3,4]))].length === 2;' | $SANDCASTLE run /dev/stdin --fuel 5000000000"

echo ""

# --- Artifacts ---
echo "Artifacts:"

TMPFILE=$(mktemp)
echo "hello world" > "$TMPFILE"
check_output "read input artifact" "hello world" \
    "echo 'return globalThis.__sandcastle_read_artifact(\"data.txt\");' | $SANDCASTLE run /dev/stdin --artifact data.txt=$TMPFILE"
rm -f "$TMPFILE"

echo ""

# --- Transcript ---
echo "Transcript:"

check_output "transcript contains execution_id" "execution_id" \
    "echo 'return 1;' | $SANDCASTLE run /dev/stdin --transcript"

check_output "transcript contains fuel_consumed" "fuel_consumed" \
    "echo 'return 1;' | $SANDCASTLE run /dev/stdin --transcript"

echo ""

# --- Info command ---
echo "Info command:"

check_output "info shows version" "SandCastle" \
    "$SANDCASTLE info"

check_output "info shows platform" "Platform" \
    "$SANDCASTLE info"

echo ""

# --- Init command ---
echo "Init command:"

TMPDIR_INIT=$(mktemp -d)
check "init creates project" "$SANDCASTLE init $TMPDIR_INIT/test-project"
check_output "init creates scripts dir" "hello.js" "ls $TMPDIR_INIT/test-project/scripts/"
check_output "init creates config" "sandcastle.config.json" "ls $TMPDIR_INIT/test-project/"
rm -rf "$TMPDIR_INIT"

echo ""

# --- Summary ---
echo "================================"
echo "Results: $PASS passed, $FAIL failed, $TOTAL total"

if [ $FAIL -gt 0 ]; then
    exit 1
fi
