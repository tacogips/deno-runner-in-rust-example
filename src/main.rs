//! # Deno Sample - Example Application
//!
//! This binary demonstrates how to run JavaScript code within a Rust application
//! using the Deno runtime. It shows the recommended patterns for:
//!
//! 1. Running Deno in a dedicated OS thread (required because JsRuntime is !Send)
//! 2. Communicating between async Rust code and the Deno thread via channels
//! 3. Calling JavaScript functions and receiving results
//!
//! ## Running the Example
//!
//! ```bash
//! cargo run -p deno_sample
//! ```
//!
//! ## Architecture
//!
//! The example uses this architecture:
//!
//! ```text
//! Main Thread (tokio async runtime)
//!     |
//!     +-- spawns OS thread for Deno
//!     |       |
//!     |       +-- LocalSet with JsRunner
//!     |       |
//!     |       +-- waits for function calls via channel
//!     |
//!     +-- sends JsFunction calls through channel
//!     |
//!     +-- receives results through oneshot channels
//! ```

use deno_sample::{
    FunctionRunner, FunctionRunnerChannelCleaner, JsFunction, JsFunctionResult, JsRunner, JsValue, JsValueType, func_runner,
    resolve_deno_url,
};
use tokio::runtime::Builder;
use tokio::sync::mpsc;
use tokio::task::LocalSet;
use tracing::{error, info};

/// Sample JavaScript code that demonstrates various function types.
///
/// This code is loaded directly into the Deno runtime and provides
/// several exported functions that can be called from Rust.
const SAMPLE_JS_CODE: &str = r#"
// ============================================================================
// Sample JavaScript Module
// ============================================================================
// This module exports several functions that demonstrate different types
// of Rust-JavaScript interoperability.

/**
 * Adds two numbers together.
 *
 * @param {number} a - First number
 * @param {number} b - Second number
 * @returns {number} The sum of a and b
 */
export function add(a, b) {
    console.log(`add called with a=${a}, b=${b}`);
    return a + b;
}

/**
 * Multiplies two numbers.
 *
 * @param {number} a - First number
 * @param {number} b - Second number
 * @returns {number} The product of a and b
 */
export function multiply(a, b) {
    console.log(`multiply called with a=${a}, b=${b}`);
    return a * b;
}

/**
 * Greets a person by name.
 *
 * @param {string} name - The name to greet
 * @returns {string} A greeting message
 */
export function greet(name) {
    console.log(`greet called with name=${name}`);
    return `Hello, ${name}!`;
}

/**
 * Processes an array of numbers and returns their squares.
 *
 * @param {number[]} numbers - Array of numbers to square
 * @returns {number[]} Array of squared numbers
 */
export function squareArray(numbers) {
    console.log(`squareArray called with ${numbers.length} numbers`);
    return numbers.map(n => n * n);
}

/**
 * Splits a string into an array of words.
 *
 * @param {string} text - The text to split
 * @returns {string[]} Array of words
 */
export function splitWords(text) {
    console.log(`splitWords called with text: "${text}"`);
    return text.split(' ').filter(word => word.length > 0);
}

/**
 * A function that doesn't return anything (returns undefined).
 *
 * @param {string} message - Message to log
 */
export function logOnly(message) {
    console.log(`logOnly: ${message}`);
    // No return statement - returns undefined
}

/**
 * A simple async function that demonstrates Promise handling.
 * Note: In this minimal runtime, we don't have setTimeout, so we just
 * return a resolved Promise immediately.
 *
 * @param {string} value - Value to return
 * @returns {Promise<string>} A promise that resolves to the value
 */
export async function asyncGreet(name) {
    console.log(`asyncGreet called with name=${name}`);
    return `Async Hello, ${name}!`;
}

// Log that the module has been loaded
console.log("Sample JavaScript module loaded successfully!");
"#;

/// The main entry point for the example application.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for logging
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .init();

    info!("=== Deno Sample Application ===");
    info!("This example demonstrates running JavaScript in Rust using Deno.");
    info!("");

    // ========================================================================
    // Step 1: Create the communication channel
    // ========================================================================
    //
    // We use an unbounded mpsc channel to send function calls from the main
    // async runtime to the Deno thread.

    info!("Step 1: Creating communication channel...");
    let (function_sender, function_receiver) = mpsc::unbounded_channel::<Box<FunctionRunner>>();

    // Create a cleaner that will close the channel when dropped
    let _cleaner = FunctionRunnerChannelCleaner::new(function_sender.clone());

    // ========================================================================
    // Step 2: Spawn the Deno runtime in a dedicated OS thread
    // ========================================================================
    //
    // JsRuntime doesn't implement Send, so we can't use tokio::spawn().
    // Instead, we create a dedicated OS thread with its own LocalSet.

    info!("Step 2: Spawning Deno runtime in dedicated OS thread...");

    let deno_thread_handle = std::thread::spawn(move || {
        // Create a single-threaded tokio runtime for this thread
        let rt = Builder::new_current_thread().enable_all().build().unwrap();

        // LocalSet allows running !Send futures on the current thread
        let local = LocalSet::new();

        local.spawn_local(async move {
            // Resolve the module URL (can be any valid URL scheme)
            let main_module = resolve_deno_url("file:///main.js").unwrap();

            // Create the JsRunner with our sample code
            info!("Deno thread: Creating JsRunner...");
            let mut runner = match JsRunner::new(main_module, Some(SAMPLE_JS_CODE.to_string())).await {
                Ok(runner) => runner,
                Err(e) => {
                    error!("Failed to create JsRunner: {}", e);
                    return;
                }
            };

            info!("Deno thread: Waiting for function calls...");

            // Wait for function calls from the channel
            if let Err(e) = runner.wait_js_functions(function_receiver).await {
                error!("JsRunner error: {}", e);
            }

            info!("Deno thread: Shutting down...");
        });

        // Run the LocalSet to completion
        rt.block_on(local);
        info!("Deno thread: Exited.");
    });

    // Give the Deno thread time to initialize
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // ========================================================================
    // Step 3: Call JavaScript functions from the main thread
    // ========================================================================

    info!("");
    info!("Step 3: Calling JavaScript functions from Rust...");
    info!("");

    // ------------------------------------------------------------------------
    // Example 1: Call add(5, 3) and get the result
    // ------------------------------------------------------------------------
    info!("--- Example 1: add(5, 3) ---");

    let (result_handler, result_receiver) = JsFunctionResult::new(JsValueType::Int32);
    let add_call = JsFunction::new("add", vec![JsValue::Int32(5), JsValue::Int32(3)], Some(result_handler));

    // Send the function call to the Deno thread
    function_sender.send(Box::new(func_runner(vec![add_call])))?;

    // Wait for the result
    match result_receiver.await {
        Ok(Ok(value)) => {
            info!("Result: {:?}", value);
            if let Some(sum) = value.as_int32() {
                info!("5 + 3 = {}", sum);
            }
        }
        Ok(Err(e)) => error!("Function error: {}", e),
        Err(e) => error!("Channel error: {}", e),
    }

    info!("");

    // ------------------------------------------------------------------------
    // Example 2: Call greet("World") and get the string result
    // ------------------------------------------------------------------------
    info!("--- Example 2: greet(\"World\") ---");

    let (result_handler, result_receiver) = JsFunctionResult::new(JsValueType::String);
    let greet_call = JsFunction::new("greet", vec![JsValue::String("World".to_string())], Some(result_handler));

    function_sender.send(Box::new(func_runner(vec![greet_call])))?;

    match result_receiver.await {
        Ok(Ok(value)) => {
            info!("Result: {:?}", value);
            if let Some(greeting) = value.as_string() {
                info!("Greeting: {}", greeting);
            }
        }
        Ok(Err(e)) => error!("Function error: {}", e),
        Err(e) => error!("Channel error: {}", e),
    }

    info!("");

    // ------------------------------------------------------------------------
    // Example 3: Call squareArray([1, 2, 3, 4, 5]) and get the array result
    // ------------------------------------------------------------------------
    info!("--- Example 3: squareArray([1, 2, 3, 4, 5]) ---");

    let (result_handler, result_receiver) = JsFunctionResult::new(JsValueType::ArrayOfInt32);
    let square_call = JsFunction::new(
        "squareArray",
        vec![JsValue::ArrayOfInt32(vec![1, 2, 3, 4, 5])],
        Some(result_handler),
    );

    function_sender.send(Box::new(func_runner(vec![square_call])))?;

    match result_receiver.await {
        Ok(Ok(value)) => {
            info!("Result: {:?}", value);
            if let Some(squares) = value.as_array_of_int32() {
                info!("Squares: {:?}", squares);
            }
        }
        Ok(Err(e)) => error!("Function error: {}", e),
        Err(e) => error!("Channel error: {}", e),
    }

    info!("");

    // ------------------------------------------------------------------------
    // Example 4: Call splitWords("Hello Rust and JavaScript")
    // ------------------------------------------------------------------------
    info!("--- Example 4: splitWords(\"Hello Rust and JavaScript\") ---");

    let (result_handler, result_receiver) = JsFunctionResult::new(JsValueType::ArrayOfString);
    let split_call = JsFunction::new(
        "splitWords",
        vec![JsValue::String("Hello Rust and JavaScript".to_string())],
        Some(result_handler),
    );

    function_sender.send(Box::new(func_runner(vec![split_call])))?;

    match result_receiver.await {
        Ok(Ok(value)) => {
            info!("Result: {:?}", value);
            if let Some(words) = value.as_array_of_string() {
                info!("Words: {:?}", words);
            }
        }
        Ok(Err(e)) => error!("Function error: {}", e),
        Err(e) => error!("Channel error: {}", e),
    }

    info!("");

    // ------------------------------------------------------------------------
    // Example 5: Call logOnly("This message is logged but returns undefined")
    // ------------------------------------------------------------------------
    info!("--- Example 5: logOnly(...) - No return value ---");

    let (result_handler, result_receiver) = JsFunctionResult::new(JsValueType::Undefined);
    let log_call = JsFunction::new(
        "logOnly",
        vec![JsValue::String("This message is logged but returns undefined".to_string())],
        Some(result_handler),
    );

    function_sender.send(Box::new(func_runner(vec![log_call])))?;

    match result_receiver.await {
        Ok(Ok(value)) => {
            info!("Result: {:?} (as expected for void function)", value);
        }
        Ok(Err(e)) => error!("Function error: {}", e),
        Err(e) => error!("Channel error: {}", e),
    }

    info!("");

    // ------------------------------------------------------------------------
    // Example 6: Call async function asyncGreet("Async World")
    // ------------------------------------------------------------------------
    info!("--- Example 6: asyncGreet(\"Async World\") - Async function ---");

    let (result_handler, result_receiver) = JsFunctionResult::new(JsValueType::String);
    let async_call = JsFunction::new(
        "asyncGreet",
        vec![JsValue::String("Async World".to_string())],
        Some(result_handler),
    );

    function_sender.send(Box::new(func_runner(vec![async_call])))?;

    match result_receiver.await {
        Ok(Ok(value)) => {
            info!("Result: {:?}", value);
            if let Some(greeting) = value.as_string() {
                info!("Async Greeting: {}", greeting);
            }
        }
        Ok(Err(e)) => error!("Function error: {}", e),
        Err(e) => error!("Channel error: {}", e),
    }

    info!("");

    // ------------------------------------------------------------------------
    // Example 7: Call multiple functions in a single batch
    // ------------------------------------------------------------------------
    info!("--- Example 7: Multiple function calls in a batch ---");

    let (result1_handler, result1_receiver) = JsFunctionResult::new(JsValueType::Int32);
    let (result2_handler, result2_receiver) = JsFunctionResult::new(JsValueType::Int32);

    let call1 = JsFunction::new("add", vec![JsValue::Int32(10), JsValue::Int32(20)], Some(result1_handler));
    let call2 = JsFunction::new("multiply", vec![JsValue::Int32(6), JsValue::Int32(7)], Some(result2_handler));

    // Send both calls in a single batch
    function_sender.send(Box::new(func_runner(vec![call1, call2])))?;

    // Receive both results
    let (r1, r2) = tokio::join!(result1_receiver, result2_receiver);

    match r1 {
        Ok(Ok(value)) => {
            if let Some(sum) = value.as_int32() {
                info!("10 + 20 = {}", sum);
            }
        }
        _ => error!("Failed to get result 1"),
    }

    match r2 {
        Ok(Ok(value)) => {
            if let Some(product) = value.as_int32() {
                info!("6 * 7 = {}", product);
            }
        }
        _ => error!("Failed to get result 2"),
    }

    info!("");

    // ========================================================================
    // Step 4: Cleanup
    // ========================================================================

    info!("Step 4: Cleaning up...");

    // Drop the sender to signal the Deno thread to shut down
    drop(function_sender);
    drop(_cleaner);

    // Wait for the Deno thread to finish
    info!("Waiting for Deno thread to exit...");
    if let Err(e) = deno_thread_handle.join() {
        error!("Deno thread panicked: {:?}", e);
    }

    info!("");
    info!("=== Example completed successfully! ===");

    Ok(())
}
