use anyhow::Context;
use futures::FutureExt;
use std::convert::identity;
use structopt::StructOpt;

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

    match cli.customer {
        Configure(cli::Configure { .. }) => {
            drop(config);
            tokio::task::spawn_blocking(|| Ok(edit::edit_file(config_path)?)).await?
        }
        List(list) => {
            println!("{}", list.run(config.await?).await?);
            Ok(())
        }
        Show(show) => {
            println!("{}", show.run(config.await?).await?);
            Ok(())
        }
        Rename(rename) => rename.run(config.await?).await,
        Establish(establish) => establish.run(config.await?).await,
        Pay(pay) => pay.run(config.await?).await,
        Refund(refund) => refund.run(config.await?).await,
        Close(close) => close.run(config.await?).await,
        Watch(watch) => watch.run(config.await?).await,
    }
}

#[allow(unused)]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    main_with_cli(Cli::from_args()).await
}
