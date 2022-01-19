mod common;
use zeekoe::customer::Config;

#[tokio::test(flavor = "multi_thread")]
async fn setup_works() {
    common::setup();

    let _config = Config::load("TestCustomer.toml")
        .await
        .expect("failed to load customer config");

    assert!(true);
}
