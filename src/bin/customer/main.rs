use {async_trait::async_trait, std::convert::identity, structopt::StructOpt};

use zeekoe::customer::{
    cli::{self, Account::*, Customer::*},
    defaults::config_path,
    Cli, Config,
};

mod close;
mod establish;
mod manage;
mod pay;

/// A single customer-side command, parameterized by the currently loaded configuration.
///
/// All subcommands of [`Customer`] should implement this, except [`Configure`], which does not need
/// to start with a valid loaded configuration.
#[async_trait]
pub trait Command {
    async fn run(self, config: Config) -> Result<(), anyhow::Error>;
}

pub async fn main_with_cli(cli: Cli) -> Result<(), anyhow::Error> {
    let config_path = cli.config.ok_or_else(config_path).or_else(identity)?;
    let config = Config::load(&config_path);

    match cli.customer {
        Configure(cli::Configure { .. }) => {
            drop(config);
            tokio::task::spawn_blocking(|| Ok(edit::edit_file(config_path)?)).await?
        }
        Account(Import(import)) => import.run(config.await?).await,
        Account(Remove(remove)) => remove.run(config.await?).await,
        List(list) => list.run(config.await?).await,
        Rename(rename) => rename.run(config.await?).await,
        Establish(establish) => establish.run(config.await?).await,
        Pay(pay) => pay.run(config.await?).await,
        Refund(refund) => refund.run(config.await?).await,
        Close(close) => close.run(config.await?).await,
    }
}

#[allow(unused)]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    main_with_cli(Cli::from_args()).await
}
