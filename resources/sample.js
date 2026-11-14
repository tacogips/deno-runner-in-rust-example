// sample.js - Simple JavaScript sample for deno_sample crate
//
// This file demonstrates basic JavaScript functions that can be called from Rust.
// Each function performs a simple text transformation.

/**
 * Appends a greeting prefix to the input text.
 *
 * @param {string} text - The input text
 * @returns {string} Text with "Hello, " prefix
 */
export function addGreeting(text) {
    return "Hello, " + text + "!";
}

/**
 * Wraps text with brackets.
 *
 * @param {string} text - The input text
 * @returns {string} Text wrapped with [ ]
 */
export function wrapWithBrackets(text) {
    return "[" + text + "]";
}

/**
 * Converts text to uppercase.
 *
 * @param {string} text - The input text
 * @returns {string} Uppercase text
 */
export function toUpperCase(text) {
    return text.toUpperCase();
}

/**
 * Converts text to lowercase.
 *
 * @param {string} text - The input text
 * @returns {string} Lowercase text
 */
export function toLowerCase(text) {
    return text.toLowerCase();
}

/**
 * Reverses the input text.
 *
 * @param {string} text - The input text
 * @returns {string} Reversed text
 */
export function reverseText(text) {
    return text.split("").reverse().join("");
}

/**
 * Counts the number of characters in the text.
 *
 * @param {string} text - The input text
 * @returns {number} Character count
 */
export function countChars(text) {
    return text.length;
}

/**
 * Repeats the text n times.
 *
 * @param {string} text - The input text
 * @param {number} n - Number of repetitions
 * @returns {string} Repeated text
 */
export function repeatText(text, n) {
    return text.repeat(n);
}

/**
 * Joins an array of strings with a separator.
 *
 * @param {string[]} items - Array of strings
 * @param {string} separator - Separator string
 * @returns {string} Joined string
 */
export function joinWithSeparator(items, separator) {
    return items.join(separator);
}

/**
 * Creates a simple JSON string from key-value pairs.
 *
 * @param {string} key - The key
 * @param {string} value - The value
 * @returns {string} JSON string
 */
export function createJson(key, value) {
    const obj = {};
    obj[key] = value;
    return JSON.stringify(obj);
}

/**
 * Adds a timestamp prefix to the text.
 *
 * @param {string} text - The input text
 * @returns {string} Text with timestamp prefix
 */
export function addTimestamp(text) {
    const now = new Date().toISOString();
    return "[" + now + "] " + text;
}

// Log that the module has been loaded
console.log("sample.js loaded successfully!");
