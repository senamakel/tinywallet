//! Validate one address per supported chain, then show a failure.
//!
//! Examples are compiled and linted in CI, so they cannot drift from the API.
//! Run it with:
//!
//! ```sh
//! cargo run --example basic
//! ```

use tinywallet::{Chain, address};

fn main() {
    let fixtures = [
        (Chain::Btc, "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"),
        (Chain::Evm, "0x52908400098527886E0F7030069857D2E4169EE7"),
        (Chain::Solana, "11111111111111111111111111111111"),
        (Chain::Tron, "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"),
    ];

    for (chain, candidate) in fixtures {
        match address::validate(chain, candidate) {
            Ok(valid) => println!("{chain:>6}: {valid}"),
            Err(error) => println!("{chain:>6}: rejected — {error}"),
        }
    }

    // Failure modes are part of the public contract, so show one. A Bitcoin
    // testnet address is well-formed but on the wrong network, which is a
    // distinct error from a malformed one.
    match address::btc::validate("tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx") {
        Ok(valid) => println!("unexpectedly accepted {valid}"),
        Err(error) => println!("expected failure: {error}"),
    }

    // A legacy Bitcoin address is a fine recipient but cannot be a sender.
    match address::btc::validate_sender("1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2") {
        Ok(valid) => println!("unexpectedly accepted {valid} as a sender"),
        Err(error) => println!("expected failure: {error}"),
    }
}
