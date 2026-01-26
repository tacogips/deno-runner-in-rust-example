# deno-runner-in-rust-example

A complete example demonstrating how to embed and run JavaScript code within a Rust application using [Deno Core](https://crates.io/crates/deno_core).

## Features

- Run JavaScript/TypeScript code from Rust
- Bidirectional value passing between Rust and JavaScript
- Custom Rust operations (ops) callable from JavaScript
- Thread-safe architecture for async Rust applications
- Channel-based communication pattern

## Quick Start

```bash
# Run the example
cargo run

# With debug logging
RUST_LOG=debug cargo run
```

## Architecture

Since Deno's `JsRuntime` doesn't implement `Send`, it cannot be used directly with `tokio::spawn()`. This example demonstrates the recommended pattern:

```
+-------------------+     mpsc channel      +---------------------+
|   Tokio Runtime   | ------------------->  |   OS Thread         |
|   (async tasks)   |                       |   (LocalSet)        |
|                   |     oneshot channel   |                     |
|   Send JsFunction | <-------------------  |   JsRunner          |
|   Receive result  |                       |   (V8 engine)       |
+-------------------+                       +---------------------+
```

1. Spawn a dedicated OS thread using `std::thread::spawn()`
2. Create a `tokio::task::LocalSet` within that thread
3. Use `mpsc` channels to send function calls from async Rust code
4. Use `oneshot` channels to receive results back

## Project Structure

```
deno-runner-in-rust-example/
  Cargo.toml          # Package configuration
  README.md           # This file
  CLAUDE.md           # Instructions for Claude Code AI assistant
  src/
    lib.rs            # Core library - JsRunner, JsValue, JsFunction, etc.
    main.rs           # Example application with 7 usage examples
    runtime.js        # JavaScript runtime setup (console, crypto, TextEncoder)
  resources/
    sample.js         # Simple JavaScript sample functions
```

## Core Components

### JsRunner

Wrapper around Deno's `JsRuntime` that simplifies module loading and execution.

```rust
use deno_sample::{JsRunner, resolve_deno_url};

let url = resolve_deno_url("file:///main.js")?;
let code = r#"
    export function greet(name) {
        return `Hello, ${name}!`;
    }
"#;
let runner = JsRunner::new(url, Some(code.to_string())).await?;
```

### JsFunction / JsValue

Type-safe function calls and value conversion between Rust and JavaScript.

```rust
use deno_sample::{JsFunction, JsValue, JsValueType, JsFunctionResult, func_runner};

// Create a function call with expected return type
let (result_handler, receiver) = JsFunctionResult::new(JsValueType::String);
let call = JsFunction::new(
    "greet",
    vec![JsValue::String("World".to_string())],
    Some(result_handler),
);

// Send to Deno thread
sender.send(Box::new(func_runner(vec![call])))?;

// Receive result
let result = receiver.await??;
println!("{}", result.as_string().unwrap()); // "Hello, World!"
```

### Supported Value Types

| Rust Type | JavaScript Type | JsValue Variant |
|-----------|-----------------|-----------------|
| `String` | `string` | `JsValue::String` |
| `i32` | `number` | `JsValue::Int32` |
| `f64` | `number` | `JsValue::Float64` |
| `Vec<i32>` | `number[]` | `JsValue::ArrayOfInt32` |
| `Vec<f64>` | `number[]` | `JsValue::ArrayOfFloat64` |
| `Vec<String>` | `string[]` | `JsValue::ArrayOfString` |
| `()` | `undefined` | `JsValue::Undefined` |

### Custom Operations (Ops)

The runtime provides these Rust functions callable from JavaScript:

- `console.log/warn/error/debug` - Forwards to Rust's tracing system
- `crypto.getRandomValues()` - Cryptographic random number generation
- `TextEncoder` - String encoding utilities

## Usage Examples

The `main.rs` demonstrates:

1. **Basic arithmetic**: `add(5, 3)` returns `8`
2. **String operations**: `greet("World")` returns `"Hello, World!"`
3. **Array processing**: `squareArray([1,2,3,4,5])` returns `[1,4,9,16,25]`
4. **String splitting**: `splitWords("Hello Rust")` returns `["Hello", "Rust"]`
5. **Void functions**: `logOnly("message")` returns `undefined`
6. **Async functions**: `asyncGreet("World")` returns Promise result
7. **Batch calls**: Multiple functions in a single request

## Dependencies

```toml
[dependencies]
deno_core = "=0.327.0"  # Deno JavaScript runtime
rand = "0.9"            # For crypto.getRandomValues()
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync"] }
tracing = "0.1"         # Structured logging
thiserror = "2"         # Error handling
```

Note: `deno_core` version is pinned to 0.327.0 because 0.328.0+ requires syntax changes.

## Adding Custom JavaScript Functions

1. Create a JavaScript file with exported functions:

```javascript
// my_functions.js
export function myFunction(input) {
    return input.toUpperCase();
}
```

2. Load it in Rust:

```rust
const MY_JS: &str = include_str!("my_functions.js");

let runner = JsRunner::new(
    resolve_deno_url("file:///my_functions.js")?,
    Some(MY_JS.to_string())
).await?;
```

## Adding Custom Rust Operations

To expose new Rust functions to JavaScript:

```rust
use deno_core::{op2, extension};

#[op2(fast)]
fn op_my_custom_op(#[string] input: String) -> String {
    input.to_uppercase()
}

extension!(
    my_extension,
    ops = [op_my_custom_op],
    // ...
);
```

Then call from JavaScript:

```javascript
const result = Deno.core.ops.op_my_custom_op("hello");
```

## License

MIT
