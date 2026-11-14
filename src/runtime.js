// runtime.js - JavaScript Runtime Environment Setup for Deno in Rust
//
// This file sets up the global JavaScript environment that will be available
// to all loaded JavaScript modules. It bridges the gap between Deno's minimal
// runtime and a typical browser/Node.js environment by providing:
//
// 1. console object - Logging functions that call back into Rust's tracing system
// 2. TextEncoder - String encoding utilities
// 3. crypto.getRandomValues - Cryptographic random number generation
//
// The `Deno.core.ops` object provides access to the Rust operations (ops)
// registered via the `extension!` macro in lib.rs.

const { core } = Deno;

// ============================================================================
// Helper: Convert arguments to a loggable message string
// ============================================================================

/**
 * Converts function arguments to a single message string for logging.
 * Handles undefined/null values and JSON-stringifies objects.
 *
 * @param {...any} args - The arguments to convert
 * @returns {string} A space-separated string of all arguments
 */
function argsToMessage(...args) {
  return args
    .map((arg) => {
      if (!arg) {
        return "[undefined]";
      } else {
        return JSON.stringify(arg);
      }
    })
    .join(" ");
}

// ============================================================================
// Global: console object
// ============================================================================
//
// Provides console.log, console.warn, console.error, and console.debug
// functions that forward messages to Rust's tracing system.

globalThis.console = {
  log: (...args) => {
    core.ops.op_log_info(`${argsToMessage(...args)}\n`);
  },
  warn: (...args) => {
    core.ops.op_log_warn(`${argsToMessage(...args)}\n`);
  },
  error: (...args) => {
    core.ops.op_log_error(`${argsToMessage(...args)}\n`);
  },
  debug: (...args) => {
    core.ops.op_log_debug(`${argsToMessage(...args)}\n`);
  },
};

// ============================================================================
// Global: TextEncoder class
// ============================================================================
//
// A simplified implementation of the standard TextEncoder API.
// This is needed by some JavaScript libraries that expect browser APIs.

globalThis.TextEncoder = class TextEncoder {
  /**
   * Encodes a string to a Uint8Array of UTF-16 code units.
   * Note: This is a simplified implementation - not full UTF-8 encoding.
   *
   * @param {string} string - The string to encode
   * @returns {Uint8Array} The encoded bytes
   */
  encode(string) {
    const codeUnits = new Uint16Array(string.length);
    for (let i = 0; i < codeUnits.length; i++) {
      codeUnits[i] = string.charCodeAt(i);
    }
    const charCodes = new Uint8Array(codeUnits.buffer);
    return charCodes;
  }

  /**
   * Encodes a string into the provided Uint8Array.
   *
   * @param {string} string - The string to encode
   * @param {Uint8Array} u8arr - The destination array
   * @returns {{read: number, written: number}} The number of characters read and bytes written
   */
  encodeInto(string, u8arr) {
    const encoded = this.encode(string);
    u8arr.set(encoded);
    return { read: string.length, written: encoded.length };
  }
};

// ============================================================================
// Global: crypto object
// ============================================================================
//
// Provides crypto.getRandomValues() for cryptographic random number generation.
// This calls into the Rust op_get_random_values function.

globalThis.crypto = {
  /**
   * Fills the provided typed array with cryptographically secure random values.
   *
   * @param {TypedArray} args - A typed array to fill with random values
   * @returns {TypedArray} The same array, now filled with random values
   */
  getRandomValues: (args) => {
    return core.ops.op_get_random_values(args);
  },
};
