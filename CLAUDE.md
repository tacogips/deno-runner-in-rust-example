# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is **deno-runner-in-rust-example**, a standalone Rust crate demonstrating how to embed and execute JavaScript code using [Deno Core](https://crates.io/crates/deno_core) (the V8-based JavaScript runtime).

## Key Concepts

### Why Deno Core?

Deno Core provides a minimal, embeddable JavaScript runtime based on V8. Unlike full Deno or Node.js, it allows fine-grained control over what JavaScript APIs are available, making it suitable for:

- Sandboxed JavaScript execution
- Custom scripting engines
- Plugins/extensions written in JavaScript
- Evaluating user-provided code safely

### Thread Architecture

The most important architectural decision in this crate is the **dedicated OS thread pattern**:

```
Tokio async runtime (main thread)
    |
    +-- Cannot use JsRuntime directly (JsRuntime is !Send)
    |
    +-- Solution: Spawn dedicated OS thread
            |
            +-- tokio::task::LocalSet for async operations
            |
            +-- JsRunner lives here permanently
            |
            +-- Receives function calls via mpsc channel
            |
            +-- Returns results via oneshot channels
```

This pattern is necessary because:
1. `JsRuntime` doesn't implement `Send` (V8 isolates are single-threaded)
2. `tokio::spawn()` requires `Send` futures
3. We need to call JS functions from async Rust code

## File Structure

```
src/
  lib.rs      # Core library with extensive documentation
              # - JsRunner: Runtime wrapper
              # - JsFunction: Function call descriptor
              # - JsValue/JsValueType: Value conversion
              # - func_runner: Batch execution helper
              # - Custom ops: console, crypto, TextEncoder

  main.rs     # Example application showing 7 usage patterns
              # - Channel setup
              # - Thread spawning
              # - Various function calls
              # - Result handling

  runtime.js  # JavaScript runtime initialization
              # - Sets up globalThis.console
              # - Sets up globalThis.crypto
              # - Sets up globalThis.TextEncoder

resources/
  sample.js   # Simple example JavaScript functions
```

## Development Commands

```bash
# Build
cargo build

# Run example
cargo run

# Run with debug logging
RUST_LOG=debug cargo run

# Check compilation
cargo check

# Run clippy
cargo clippy

# Format code
cargo fmt
```

## Code Style

### Rust

- Use `tracing` macros (`info!`, `error!`, `warn!`, `debug!`) for logging, not `println!`
- Error handling with `thiserror` derive macros
- Async code uses `tokio`
- Follow standard Rust naming conventions

### JavaScript

- ES modules with `export` statements
- Functions should be documented with JSDoc comments
- Use `console.log/warn/error/debug` for logging (forwards to Rust tracing)

## Key Implementation Details

### Module Loading

When `JsRunner::new()` is called with `Some(code)`:
- The URL (`file:///main.js`) is just an identifier, not a real file path
- `load_main_es_module_from_code()` loads the code string directly
- No filesystem access occurs

When called with `None`:
- `load_main_es_module()` actually reads from the filesystem
- The URL must be a valid file path

### Custom Operations (Ops)

Ops are Rust functions exposed to JavaScript via the `#[op2]` macro:

```rust
#[op2(fast)]
fn op_my_op(#[string] input: String) -> Result<String, AnyError> {
    Ok(input.to_uppercase())
}
```

Callable from JS as:
```javascript
Deno.core.ops.op_my_op("hello")
```

### Value Conversion

`JsValue` enum handles Rust <-> JavaScript value conversion:
- `as_v8_value()`: Rust -> V8 (for function arguments)
- `js_value_from_v8_value()`: V8 -> Rust (for return values)

Supported types: String, Int32, Float64, arrays of these, Undefined

### Error Handling

`JsCallError` enum covers all possible errors:
- `UndefinedValue`: Function not found
- `NotAJsFunction`: Value exists but not callable
- `FuncThrowsException`: JavaScript threw an error
- `JsRuntimeError`: V8/Deno runtime error
- Various serialization and I/O errors

## Dependencies

| Crate | Purpose |
|-------|---------|
| `deno_core = "=0.327.0"` | V8 JavaScript runtime (pinned version) |
| `rand` | Random number generation for crypto ops |
| `tokio` | Async runtime with channels |
| `tracing` | Structured logging |
| `thiserror` | Error type derivation |

Note: `deno_core` is pinned to 0.327.0 because 0.328.0+ has breaking changes in error handling syntax.

## Common Tasks

### Adding a New JavaScript Function

1. Add the function to your JS code (inline or in a file)
2. Export it: `export function myFunc(args) { ... }`
3. Call from Rust using `JsFunction::new("myFunc", args, result_handler)`

### Adding a New Rust Op

1. Define the op with `#[op2]` macro in `lib.rs`
2. Add it to the `extension!` macro's `ops` array
3. Call from JS: `Deno.core.ops.op_name(args)`

### Adding a New Value Type

1. Add variant to `JsValue` enum
2. Add variant to `JsValueType` enum
3. Implement `as_v8_value()` conversion
4. Implement `js_value_from_v8_value()` conversion
5. Add accessor method (e.g., `as_my_type()`)

## Testing

Currently, manual testing via `cargo run`. To add unit tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_js_function() {
        // Test implementation
    }
}
```

## Publishing

This crate is ready for publishing to crates.io:
- No proprietary code or references
- MIT license
- Complete documentation
- Working examples
