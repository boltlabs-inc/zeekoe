use {std::convert::identity, structopt::StructOpt};

use zeekoe::{
    customer, merchant,
    Cli::{self, Customer, Merchant},
};

#[tokio::main]
pub async fn main() -> Result<(), anyhow::Error> {
    match Cli::from_args() {
        Merchant { merchant, config } => {
            use merchant::{
                cli::{self, Command, Merchant::*},
                defaults::config_path,
                Config,
            };

            let config_path = config.ok_or_else(config_path).or_else(identity)?;
            let config = Config::load(&config_path);

            match merchant {
                Configure(cli::Configure { .. }) => {
                    drop(config);
                    tokio::task::spawn_blocking(|| Ok(edit::edit_file(config_path)?)).await?
                }
                Run(run) => run.run(config.await?).await,
            }
        }
        Customer { customer, config } => {
            use customer::{
                cli::{self, Account::*, Command, Customer::*},
                defaults::config_path,
                Config,
            };

            let config_path = config.ok_or_else(config_path).or_else(identity)?;
            let config = Config::load(&config_path);

            match customer {
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
    }
}
