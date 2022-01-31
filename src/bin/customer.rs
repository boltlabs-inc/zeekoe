use {
    anyhow::Context,
    futures::FutureExt,
    rand::{rngs::StdRng, SeedableRng},
    std::convert::identity,
    structopt::StructOpt,
};

use zeekoe::customer::{
    cli::{self, Customer::*},
    defaults::config_path,
    zkchannels::Command,
    Cli, Config,
};

pub async fn main_with_cli(cli: Cli) -> Result<(), anyhow::Error> {
    let config_path = cli.config.ok_or_else(config_path).or_else(identity)?;
    let config = Config::load(&config_path).map(|result| {
        result.with_context(|| {
            format!(
                "Could not load customer configuration from {:?}",
                config_path
            )
        })
    });

    // TODO: let this be made deterministic during testing
    let rng = StdRng::from_entropy();

    match cli.customer {
        Configure(cli::Configure { .. }) => {
            drop(config);
            tokio::task::spawn_blocking(|| Ok(edit::edit_file(config_path)?)).await?
        }
        List(list) => list.run(rng, config.await?).await,
        Show(show) => show.run(rng, config.await?).await,
        Rename(rename) => rename.run(rng, config.await?).await,
        Establish(establish) => establish.run(rng, config.await?).await,
        Pay(pay) => pay.run(rng, config.await?).await,
        Refund(refund) => refund.run(rng, config.await?).await,
        Close(close) => close.run(rng, config.await?).await,
        Watch(watch) => watch.run(rng, config.await?).await,
    }
}

#[allow(unused)]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    main_with_cli(Cli::from_args()).await
}
