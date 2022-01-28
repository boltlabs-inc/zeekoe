pub(crate) mod common;

use std::panic;

#[tokio::main]
pub async fn main() {
    let server_future = common::setup().await;

    // Run tests
    let tests = &[failing_test, another_failing_test];

    // Run every test, printing out details if it fails
    let results = tests
        .iter()
        .map(|test| {
            let result = panic::catch_unwind(|| test());
            if result.is_err() {
                eprintln!("Test failed <details>\n")
            }
            result
        })
        .collect::<Vec<_>>();

    // Fail if any test failed. This is separate from evaluation to enforce that _every_ test must
    // run -- otherwise, any will short-circuit the execution
    assert!(results.iter().any(|result| result.is_err()));

    common::teardown(server_future).await;
}

fn failing_test() {
    panic!("this test failed :)")
}

fn another_failing_test() {
    assert_eq!(100, 3)
}
