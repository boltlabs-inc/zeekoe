pub(crate) mod common;

use std::panic;

#[tokio::main]
pub async fn main() {
    let server_future = common::setup().await;

    // Run tests
    let tests = &[failing_test, another_failing_test];
    setup_works().await;

    // Run every test, printing out details if it fails
    let results = tests
        .into_iter()
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

async fn setup_works() {
    // Make sure that the config files were encoded validly
    let _customer_config = zeekoe::customer::Config::load(common::CUSTOMER_CONFIG)
        .await
        .expect("failed to load customer config");

    let _merch_config = zeekoe::merchant::Config::load(common::MERCHANT_CONFIG)
        .await
        .expect("failed to load merchant config");

    // Make sure that errors are written to the error log
    match common::get_error() {
        Ok(string) => {
            println!("{}", string);
            assert!(string.contains("This task is also good"))
        }
        Err(_) => assert!(false),
    }
}

fn failing_test() {
    panic!("this test failed :)")
}

fn another_failing_test() {
    assert_eq!(100, 3)
}
