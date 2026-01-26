//! # Deno Sample - Running JavaScript in Rust with Deno Core
//!
//! This crate demonstrates how to embed the Deno JavaScript runtime within a Rust application.
//! It provides a complete example of:
//!
//! - Creating a `JsRuntime` with custom Rust operations (ops)
//! - Loading and executing JavaScript modules
//! - Passing values between Rust and JavaScript (bidirectional)
//! - Running Deno in a dedicated OS thread (required because `JsRuntime` is `!Send`)
//! - Using channels for communication between async Rust code and the Deno thread
//!
//! ## Architecture Overview
//!
//! ```text
//! +-------------------+     mpsc channel      +---------------------+
//! |   Tokio Runtime   | ------------------->  |   OS Thread         |
//! |   (async tasks)   |                       |   (LocalSet)        |
//! |                   |     oneshot channel   |                     |
//! |   Send JsFunction | <-------------------  |   JsRunner          |
//! |   Receive result  |                       |   (V8 engine)       |
//! +-------------------+                       +---------------------+
//! ```
//!
//! ## Why a Separate OS Thread?
//!
//! The Deno `JsRuntime` (and the underlying V8 engine) is designed for single-threaded operation
//! and does not implement `Send`. This means it cannot be moved between threads, which is
//! required by `tokio::spawn()`. The solution is to:
//!
//! 1. Spawn a dedicated OS thread using `std::thread::spawn()`
//! 2. Create a `tokio::task::LocalSet` within that thread for async operations
//! 3. Use `mpsc` channels to send function call requests from the main async runtime
//! 4. Use `oneshot` channels to receive results back
//!
//! ## Custom Operations (Ops)
//!
//! This example registers several custom Rust functions that can be called from JavaScript:
//!
//! - `op_get_random_values`: Provides cryptographic random bytes (for `crypto.getRandomValues()`)
//! - `op_log_info`, `op_log_warn`, `op_log_debug`, `op_log_error`: Logging functions
//!
//! These are exposed to JavaScript through the runtime extension mechanism.

use thiserror::Error;

use deno_core::error::AnyError;
use deno_core::url::Url as DenoUrl;
use deno_core::v8::{self, DataError, Global, HandleScope, Local, Object, Value};
use deno_core::{JsRuntime, ModuleResolutionError, ModuleSpecifier, extension, op2, serde_v8};
use rand::Rng;
use std::rc::Rc;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tracing::{debug, error, info, warn};

// ============================================================================
// Custom Operations (Ops)
// ============================================================================
//
// These Rust functions are exposed to JavaScript through the Deno extension system.
// The `#[op2]` attribute macro generates the necessary boilerplate for V8 integration.

/// Provides cryptographic random bytes for JavaScript's `crypto.getRandomValues()`.
///
/// This op takes an input buffer and returns a new buffer of the same length
/// filled with random bytes.
#[op2]
#[arraybuffer]
fn op_get_random_values(#[buffer(copy)] input: Vec<u8>) -> Result<Vec<u8>, AnyError> {
    let mut rng = rand::rng();
    let result = (0..input.len()).map(|_| rng.random::<u8>()).collect::<Vec<u8>>();
    Ok(result)
}

/// Logs an info-level message from JavaScript to Rust's tracing system.
#[op2(fast)]
fn op_log_info(#[string] message: String) -> Result<(), AnyError> {
    info!("embedded js info: {:?}", message);
    Ok(())
}

/// Logs a warning-level message from JavaScript to Rust's tracing system.
#[op2(fast)]
fn op_log_warn(#[string] message: String) -> Result<(), AnyError> {
    warn!("embedded js warn: {:?}", message);
    Ok(())
}

/// Logs a debug-level message from JavaScript to Rust's tracing system.
#[op2(fast)]
fn op_log_debug(#[string] message: String) -> Result<(), AnyError> {
    debug!("embedded js debug: {:?}", message);
    Ok(())
}

/// Logs an error-level message from JavaScript to Rust's tracing system.
/// Note: Output as warn level to distinguish from actual Rust errors.
#[op2(fast)]
fn op_log_error(#[string] message: String) -> Result<(), AnyError> {
    warn!("embedded js error: {:?}", message);
    Ok(())
}

// ============================================================================
// Deno Extension Registration
// ============================================================================
//
// The `extension!` macro registers our custom operations and specifies the
// JavaScript entry point that sets up the global environment.

extension!(
    runjs,
    ops = [
        op_get_random_values,
        op_log_info,
        op_log_warn,
        op_log_debug,
        op_log_error,
    ],
    // Entry point JavaScript file that initializes the global environment
    esm_entry_point = "ext:runjs/runtime.js",
    // Location of the JavaScript files (relative to src/)
    esm = [dir "src", "runtime.js"],
);

// ============================================================================
// URL Resolution
// ============================================================================

/// Resolves a URL string to a Deno `ModuleSpecifier`.
///
/// This is used to create the module identifier for loading JavaScript code.
///
/// # Example
/// ```
/// let module = resolve_deno_url("file:///main.js")?;
/// ```
pub fn resolve_deno_url(url: &str) -> Result<ModuleSpecifier, JsCallError> {
    let identifier = deno_core::resolve_url(url)?;
    Ok(identifier)
}

// ============================================================================
// Types for Channel-based Function Execution
// ============================================================================

/// Type alias for a function runner closure.
///
/// This closure is sent through a channel to the Deno thread and executed
/// with access to the `JsRuntime` and the module's global namespace.
pub type FunctionRunner = dyn FnOnce(&mut JsRuntime, Global<Object>) -> Result<(), JsCallError> + Send;

/// Type alias for the sender side of the function runner channel.
pub type FunctionRunnerSender = mpsc::UnboundedSender<Box<FunctionRunner>>;

/// Type alias for the receiver side of the function runner channel.
pub type FunctionRunnerReceiver = mpsc::UnboundedReceiver<Box<FunctionRunner>>;

// ============================================================================
// JsRunner - The Core Runtime Wrapper
// ============================================================================

/// A wrapper around Deno's `JsRuntime` that provides a simplified API for
/// loading and executing JavaScript modules.
///
/// # Thread Safety
///
/// `JsRunner` is `!Send` because `JsRuntime` is `!Send`. This means it must be
/// created and used within the same OS thread. For async applications using
/// tokio, this typically means running it in a dedicated OS thread with a
/// `LocalSet`.
///
/// # Example
///
/// ```no_run
/// use deno_sample::{JsRunner, resolve_deno_url};
///
/// async fn run_js() {
///     let url = resolve_deno_url("file:///main.js").unwrap();
///     let code = r#"
///         export function add(a, b) {
///             return a + b;
///         }
///     "#.to_string();
///
///     let runner = JsRunner::new(url, Some(code)).await.unwrap();
///     // Now you can call functions from the module
/// }
/// ```
pub struct JsRunner {
    /// The underlying Deno JavaScript runtime.
    js_runtime: deno_core::JsRuntime,
    /// The module ID for the loaded main module.
    mod_id: usize,
}

impl JsRunner {
    /// Creates a new `JsRunner` by loading a JavaScript module.
    ///
    /// # Arguments
    ///
    /// * `main_module` - The URL of the main module (e.g., `file:///main.js`)
    /// * `code` - Optional JavaScript code. If `Some`, the code is loaded directly.
    ///   If `None`, the module is loaded from the filesystem.
    ///
    /// # Returns
    ///
    /// A new `JsRunner` instance with the module loaded and evaluated.
    pub async fn new(main_module: DenoUrl, code: Option<String>) -> Result<Self, JsCallError> {
        // Create the JsRuntime with our custom extension
        let mut js_runtime = deno_core::JsRuntime::new(deno_core::RuntimeOptions {
            // Use the built-in filesystem module loader
            module_loader: Some(Rc::new(deno_core::FsModuleLoader)),
            // Register our custom extension with ops and runtime.js
            extensions: vec![runjs::init_ops_and_esm()],
            ..Default::default()
        });

        // Load the main module (either from code string or filesystem)
        let mod_id = match code {
            Some(code) => js_runtime.load_main_es_module_from_code(&main_module, code).await?,
            None => js_runtime.load_main_es_module(&main_module).await?,
        };

        // Evaluate the module (executes top-level code)
        js_runtime.mod_evaluate(mod_id).await?;

        // Run the event loop to completion (handles any pending async operations)
        js_runtime.run_event_loop(Default::default()).await?;

        Ok(Self { js_runtime, mod_id })
    }

    /// Waits for function call requests from a channel and executes them.
    ///
    /// This method is designed to be run in a dedicated OS thread. It continuously
    /// receives `FunctionRunner` closures from the channel and executes them
    /// with access to the JavaScript runtime.
    ///
    /// The method returns when the channel is closed (all senders are dropped).
    ///
    /// # Arguments
    ///
    /// * `function_runner_receiver` - The receiving end of the function runner channel
    ///
    /// # Example
    ///
    /// ```no_run
    /// use tokio::sync::mpsc;
    /// use tokio::task::LocalSet;
    /// use deno_sample::{JsRunner, FunctionRunner, resolve_deno_url};
    ///
    /// // Create the channel
    /// let (sender, receiver) = mpsc::unbounded_channel::<Box<FunctionRunner>>();
    ///
    /// // Spawn a dedicated OS thread for the JsRunner
    /// std::thread::spawn(move || {
    ///     let rt = tokio::runtime::Builder::new_current_thread()
    ///         .enable_all()
    ///         .build()
    ///         .unwrap();
    ///
    ///     let local = LocalSet::new();
    ///     local.spawn_local(async move {
    ///         let url = resolve_deno_url("file:///main.js").unwrap();
    ///         let mut runner = JsRunner::new(url, Some("".to_string())).await.unwrap();
    ///         runner.wait_js_functions(receiver).await.unwrap();
    ///     });
    ///
    ///     rt.block_on(local);
    /// });
    /// ```
    #[tracing::instrument(skip_all)]
    pub async fn wait_js_functions(
        &mut self,
        mut function_runner_receiver: mpsc::UnboundedReceiver<Box<FunctionRunner>>,
    ) -> Result<(), JsCallError> {
        while let Some(each_js_function_runner) = function_runner_receiver.recv().await {
            // Get the module's global namespace object
            let global = self.js_runtime.get_module_namespace(self.mod_id)?;
            // Execute the function runner with access to the runtime and namespace
            if let Err(e) = each_js_function_runner(&mut self.js_runtime, global) {
                error!("running js function failed: {} ", e);
            }
        }

        Ok(())
    }

    /// Executes a single function runner directly.
    ///
    /// This is useful for one-off function calls when you don't need the
    /// channel-based approach.
    ///
    /// # Arguments
    ///
    /// * `runner` - A closure that will be executed with access to the runtime
    #[tracing::instrument(skip_all)]
    pub async fn run_js_function<Runner>(&mut self, runner: Runner) -> Result<(), JsCallError>
    where
        Runner: FnOnce(&mut JsRuntime, Global<Object>) -> Result<(), JsCallError>,
    {
        let global = self.js_runtime.get_module_namespace(self.mod_id)?;
        runner(&mut self.js_runtime, global)?;
        Ok(())
    }
}

// ============================================================================
// One-shot Module Execution
// ============================================================================

/// Executes JavaScript code in a new runtime and runs a function.
///
/// This is a convenience function for one-off JavaScript execution.
/// A new runtime is created, the code is loaded, and the runner is executed.
///
/// # Arguments
///
/// * `main_module` - The URL of the main module
/// * `code` - Optional JavaScript code to load
/// * `runner` - A closure to execute with access to the runtime
#[tracing::instrument(skip_all)]
pub async fn execute_in_module<Runner>(main_module: DenoUrl, code: Option<String>, runner: Runner) -> Result<(), JsCallError>
where
    Runner: FnOnce(&mut JsRuntime, Global<Object>) -> Result<(), JsCallError>,
{
    let mut js_runner = JsRunner::new(main_module, code).await?;
    js_runner.run_js_function(runner).await?;
    Ok(())
}

// ============================================================================
// Function Call Helpers
// ============================================================================

/// Creates a function runner closure that executes multiple JavaScript function calls.
///
/// This function creates a closure that can be sent through a channel to the
/// Deno thread. When executed, it will call each JavaScript function in sequence
/// and send back the results through oneshot channels.
///
/// # Arguments
///
/// * `func_calls` - A vector of `JsFunction` describing the functions to call
///
/// # Returns
///
/// A boxable closure that implements `FnOnce` and can be sent to the Deno thread
#[tracing::instrument(skip_all)]
pub fn func_runner(func_calls: Vec<JsFunction>) -> impl FnOnce(&mut JsRuntime, Global<Object>) -> Result<(), JsCallError> {
    |js_runtime: &mut JsRuntime, global: Global<Object>| {
        for each_function in func_calls.into_iter() {
            let JsFunction {
                function_name,
                args,
                result,
            } = each_function;

            // Open the global namespace in the current scope
            let namespace = global.open(js_runtime.v8_isolate());
            let mut scope = js_runtime.handle_scope();

            // Get the function from the module namespace
            let key = serde_v8::to_v8(&mut scope, &function_name)?;
            let maybe_function: v8::Local<'_, v8::Value> = namespace
                .get(&mut scope, key)
                .ok_or_else(|| JsCallError::JsRuntimeNullPointer(format!("{:?}", function_name)))?;

            // Verify it's actually a function
            if !maybe_function.is_function() {
                if maybe_function.is_undefined() {
                    return Err(JsCallError::UndefinedValue(function_name));
                }
                return Err(JsCallError::NotAJsFunction(
                    function_name,
                    maybe_function.type_repr().to_string(),
                ));
            }

            let f: v8::Local<'_, v8::Function> = maybe_function.try_into()?;

            // Convert Rust arguments to V8 values
            let mut v8_args: Vec<Local<'_, Value>> = Vec::new();
            for each_arg in args.into_iter() {
                let arg = each_arg.as_v8_value(&mut scope)?;
                v8_args.push(arg);
            }

            // Call the JavaScript function
            let recv = v8::undefined(&mut scope).into();
            let call_result = f.call(&mut scope, recv, v8_args.as_slice());

            let (call_result, func_call_returns_empty) = match call_result {
                Some(return_value) => (return_value, false),
                None => (v8::undefined(&mut scope).into(), true),
            };

            // Handle Promise results (async functions)
            let func_call_result = if call_result.is_promise() {
                let promise: v8::Local<v8::Promise> = call_result.try_into()?;
                let async_result: v8::Local<v8::Value> = promise.result(&mut scope);
                async_result
            } else {
                call_result
            };

            // Send the result back through the oneshot channel if requested
            if let Some(JsFunctionResult {
                return_type,
                result_sender,
            }) = result
            {
                if func_call_returns_empty || JsFunctionResult::value_is_exception(&func_call_result) {
                    if let Err(e) =
                        result_sender.send(Err(JsCallError::FuncThrowsException(format!("func name:{}", function_name))))
                    {
                        error!("error occurred when sending result of js func runner: {:?}", e);
                    }
                } else {
                    let return_result = return_type.js_value_from_v8_value(&mut scope, func_call_result)?;
                    if let Err(e) = result_sender.send(Ok(return_result)) {
                        error!("error occurred when sending result of js func runner: {:?}", e);
                    }
                }
            }
        }

        Ok(())
    }
}

// ============================================================================
// JavaScript Function Result Handling
// ============================================================================

/// Represents the expected result type and channel for a JavaScript function call.
///
/// When calling a JavaScript function, you can optionally specify what type of
/// return value you expect and provide a oneshot channel to receive the result.
#[derive(Debug)]
pub struct JsFunctionResult {
    /// The expected return type of the JavaScript function
    return_type: JsValueType,
    /// The channel to send the result back on
    result_sender: oneshot::Sender<Result<JsValue, JsCallError>>,
}

impl JsFunctionResult {
    /// Creates a new `JsFunctionResult` and returns the receiver end of the channel.
    ///
    /// # Arguments
    ///
    /// * `return_type` - The expected return type of the JavaScript function
    ///
    /// # Returns
    ///
    /// A tuple of the `JsFunctionResult` and the receiver to get the result from
    pub fn new(return_type: JsValueType) -> (Self, oneshot::Receiver<Result<JsValue, JsCallError>>) {
        let (tx, rx) = oneshot::channel::<Result<JsValue, JsCallError>>();
        (
            Self {
                return_type,
                result_sender: tx,
            },
            rx,
        )
    }

    /// Creates a new `JsFunctionResult` with an existing sender.
    ///
    /// This is useful when you want to manage the channel yourself.
    pub fn new_with_response_sender(
        return_type: JsValueType,
        result_sender: oneshot::Sender<Result<JsValue, JsCallError>>,
    ) -> Self {
        Self {
            return_type,
            result_sender,
        }
    }

    /// Checks if a V8 value represents an exception.
    fn value_is_exception(value: &Local<'_, v8::Value>) -> bool {
        value.type_repr() == "unknown"
    }
}

// ============================================================================
// JavaScript Function Call Description
// ============================================================================

/// Describes a JavaScript function to be called from Rust.
///
/// This struct encapsulates all the information needed to call a JavaScript
/// function: its name, arguments, and optionally where to send the result.
#[derive(Debug)]
pub struct JsFunction {
    /// The name of the function to call (must be exported from the module)
    function_name: String,
    /// The arguments to pass to the function
    args: Vec<JsValue>,
    /// Optional result handler (if you want to receive the return value)
    result: Option<JsFunctionResult>,
}

impl JsFunction {
    /// Creates a new JavaScript function call description.
    ///
    /// # Arguments
    ///
    /// * `function_name` - The name of the exported function to call
    /// * `args` - The arguments to pass to the function
    /// * `result` - Optional result handler for receiving the return value
    ///
    /// # Example
    ///
    /// ```
    /// use deno_sample::{JsFunction, JsValue, JsValueType, JsFunctionResult};
    ///
    /// // Call a function without expecting a result
    /// let call = JsFunction::new("logMessage", vec![JsValue::String("Hello".into())], None);
    ///
    /// // Call a function and receive the result
    /// let (result_handler, receiver) = JsFunctionResult::new(JsValueType::Int32);
    /// let call = JsFunction::new("add", vec![JsValue::Int32(1), JsValue::Int32(2)], Some(result_handler));
    /// ```
    pub fn new(function_name: &str, args: Vec<JsValue>, result: Option<JsFunctionResult>) -> Self {
        Self {
            function_name: function_name.to_string(),
            args,
            result,
        }
    }
}

// ============================================================================
// JavaScript Value Types
// ============================================================================

/// Describes the expected type of a JavaScript value.
///
/// This is used to specify what type of value you expect a JavaScript function
/// to return, so it can be properly converted to the corresponding Rust type.
#[derive(Debug)]
pub enum JsValueType {
    /// A JavaScript string
    String,
    /// A JavaScript 32-bit integer
    Int32,
    /// A JavaScript 32-bit float (stored as f64 internally)
    Float32,
    /// A JavaScript array of 32-bit integers
    ArrayOfInt32,
    /// A JavaScript array of strings
    ArrayOfString,
    /// A JavaScript array of 64-bit floats
    ArrayOfFloat64,
    /// JavaScript `undefined` (functions without `return` statements)
    Undefined,
}

impl JsValueType {
    /// Converts a V8 value to a `JsValue` based on the expected type.
    fn js_value_from_v8_value<'s>(
        &self,
        scope: &mut HandleScope<'s>,
        value: Local<'s, v8::Value>,
    ) -> Result<JsValue, JsCallError> {
        match self {
            Self::String => {
                let result: String = serde_v8::from_v8(scope, value)?;
                Ok(JsValue::String(result))
            }

            Self::Int32 => {
                let result: i32 = serde_v8::from_v8(scope, value)?;
                Ok(JsValue::Int32(result))
            }

            Self::Float32 => {
                let result: f64 = serde_v8::from_v8(scope, value)?;
                Ok(JsValue::Float64(result))
            }

            Self::ArrayOfInt32 => {
                let result: Vec<i32> = serde_v8::from_v8(scope, value)?;
                Ok(JsValue::ArrayOfInt32(result))
            }

            Self::ArrayOfString => {
                let result: Vec<String> = serde_v8::from_v8(scope, value)?;
                Ok(JsValue::ArrayOfString(result))
            }

            Self::ArrayOfFloat64 => {
                let result: Vec<f64> = serde_v8::from_v8(scope, value)?;
                Ok(JsValue::ArrayOfFloat64(result))
            }

            Self::Undefined => {
                if value.is_undefined() {
                    Ok(JsValue::Undefined)
                } else {
                    Err(JsCallError::UnExpectedJsValue(format!(
                        "expected 'undefined' but actual {}",
                        value.type_repr()
                    )))
                }
            }
        }
    }
}

/// Represents a value that can be passed between Rust and JavaScript.
///
/// This enum provides a type-safe way to convert values in both directions:
/// - Rust to JavaScript: Use `as_v8_value()` to create a V8 value
/// - JavaScript to Rust: Use the accessor methods like `as_string()`, `as_int32()`, etc.
#[derive(Clone, Debug, PartialEq)]
pub enum JsValue {
    /// A string value
    String(String),
    /// A 32-bit integer value
    Int32(i32),
    /// A 64-bit floating point value
    Float64(f64),
    /// An array of 32-bit integers
    ArrayOfInt32(Vec<i32>),
    /// An array of 64-bit floats
    ArrayOfFloat64(Vec<f64>),
    /// An array of strings
    ArrayOfString(Vec<String>),
    /// The JavaScript `undefined` value
    Undefined,
}

impl JsValue {
    /// Converts this `JsValue` to a V8 value for passing to JavaScript.
    fn as_v8_value<'s>(&self, scope: &mut HandleScope<'s>) -> Result<Local<'s, v8::Value>, JsCallError> {
        let result = match self {
            Self::String(value) => v8::String::new(scope, value)
                .ok_or_else(|| JsCallError::JsRuntimeNullPointer(format!("string value:{}", value)))?
                .into(),

            Self::Int32(value) => v8::Integer::new(scope, *value).into(),
            Self::Float64(value) => v8::Number::new(scope, *value).into(),

            Self::ArrayOfInt32(array_value) => {
                let v8_array_value = array_value
                    .iter()
                    .map(|each_value| v8::Integer::new(scope, *each_value).into())
                    .collect::<Vec<Local<'_, v8::Value>>>();

                v8::Array::new_with_elements(scope, &v8_array_value).into()
            }

            Self::ArrayOfFloat64(array_value) => {
                let v8_array_value = array_value
                    .iter()
                    .map(|each_value| v8::Number::new(scope, *each_value).into())
                    .collect::<Vec<Local<'_, v8::Value>>>();

                v8::Array::new_with_elements(scope, &v8_array_value).into()
            }

            Self::ArrayOfString(array_value) => {
                let mut v8_array_value = vec![];
                for each_value in array_value {
                    v8_array_value.push(
                        v8::String::new(scope, each_value)
                            .ok_or_else(|| JsCallError::JsRuntimeNullPointer(format!("string value:{}", each_value)))?
                            .into(),
                    );
                }

                v8::Array::new_with_elements(scope, &v8_array_value).into()
            }

            Self::Undefined => v8::undefined(scope).into(),
        };
        Ok(result)
    }

    /// Attempts to extract a string value.
    pub fn as_string(&self) -> Option<String> {
        if let Self::String(value) = self {
            Some(value.clone())
        } else {
            None
        }
    }

    /// Attempts to extract a 32-bit integer value.
    pub fn as_int32(&self) -> Option<i32> {
        if let Self::Int32(value) = self { Some(*value) } else { None }
    }

    /// Attempts to extract a 64-bit float value.
    pub fn as_float64(&self) -> Option<f64> {
        if let Self::Float64(value) = self { Some(*value) } else { None }
    }

    /// Attempts to extract an array of strings.
    pub fn as_array_of_string(&self) -> Option<Vec<String>> {
        if let Self::ArrayOfString(value) = self {
            Some(value.clone())
        } else {
            None
        }
    }

    /// Attempts to extract an array of 32-bit integers.
    pub fn as_array_of_int32(&self) -> Option<Vec<i32>> {
        if let Self::ArrayOfInt32(value) = self {
            Some(value.clone())
        } else {
            None
        }
    }

    /// Attempts to extract an array of 64-bit floats.
    pub fn as_array_of_float64(&self) -> Option<Vec<f64>> {
        if let Self::ArrayOfFloat64(value) = self {
            Some(value.clone())
        } else {
            None
        }
    }
}

// ============================================================================
// Error Types
// ============================================================================

/// Errors that can occur when calling JavaScript functions from Rust.
#[derive(Error, Debug)]
pub enum JsCallError {
    /// The returned JavaScript value doesn't match the expected type.
    #[error("unexpected js value: {0}")]
    UnExpectedJsValue(String),

    /// The requested function or value is undefined in JavaScript.
    #[error("{0} is undefined")]
    UndefinedValue(String),

    /// The value exists but is not a callable function.
    #[error("{0} is not a function. {1}")]
    NotAJsFunction(String, String),

    /// A null pointer was encountered (V8 returned None unexpectedly).
    #[error("reference to null pointer. It doesn't mean the reference is 'undefined': {0}")]
    JsRuntimeNullPointer(String),

    /// A general error from the Deno runtime.
    #[error("error occurred in js runtime: {0}")]
    JsRuntimeError(#[from] AnyError),

    /// An error during serde serialization/deserialization with V8.
    #[error("serde v8 error: {0}")]
    JsRuntimeSerdeError(#[from] serde_v8::Error),

    /// A V8 data error.
    #[error("error occurred in js runtime: {0}")]
    JsRuntimeDataError(#[from] DataError),

    /// Failed to resolve a JavaScript module.
    #[error("couldn't resolve the module: {0}")]
    ModuleResolutionError(#[from] ModuleResolutionError),

    /// An I/O error when loading modules.
    #[error("io error when executing embedded js: {0}")]
    IoError(#[from] std::io::Error),

    /// No function runner receiver was set up.
    #[error("no function runner receiver set")]
    NoFunctionRunnerReceiver,

    /// The JavaScript function threw an exception.
    #[error("function throws exception: {0}")]
    FuncThrowsException(String),
}

// ============================================================================
// Channel Cleanup Helper
// ============================================================================

/// A guard that cleans up the function runner channel when dropped.
///
/// This is useful for ensuring the Deno thread shuts down gracefully when
/// the main application exits. Dropping the sender will cause the receiver
/// to return `None`, which signals `wait_js_functions` to exit.
pub struct FunctionRunnerChannelCleaner {
    sender: Option<FunctionRunnerSender>,
}

impl FunctionRunnerChannelCleaner {
    /// Creates a new cleaner that will drop the sender when this guard is dropped.
    pub fn new(sender: FunctionRunnerSender) -> Self {
        Self { sender: Some(sender) }
    }

    /// Takes ownership of the sender, preventing automatic cleanup.
    pub fn take(&mut self) -> Option<FunctionRunnerSender> {
        self.sender.take()
    }
}

impl Drop for FunctionRunnerChannelCleaner {
    fn drop(&mut self) {
        // Dropping the sender closes the channel
        self.sender.take();
    }
}
