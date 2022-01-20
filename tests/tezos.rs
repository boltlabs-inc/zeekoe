mod common;

#[tokio::test(flavor = "multi_thread")]
async fn setup_works() {
    common::setup();

    // Make sure that the config files were encoded validly
    let _customer_config = zeekoe::customer::Config::load(common::CUSTOMER_CONFIG)
        .await
        .expect("failed to load customer config");

    let _merch_config = zeekoe::merchant::Config::load(common::MERCHANT_CONFIG)
        .await
        .expect("failed to load merchant config");
}
