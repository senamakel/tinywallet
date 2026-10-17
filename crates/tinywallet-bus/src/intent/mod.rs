//! Pure comparisons between prepared wallet quotes and execution intents.
//!
//! These functions describe whether a supplied intent agrees with a quote.
//! They do not retain or consume host approval, authorize execution, or call a
//! wallet implementation.

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
