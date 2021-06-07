use {async_trait::async_trait, dialectic::Session, std::convert::identity, structopt::StructOpt};

use zeekoe::merchant::{
    self,
    cli::{self, Run},
    defaults::config_path,
    Chan, Cli, Config,
};

#[path = "merchant/pay.rs"]
mod pay;
use pay::Pay;

/// A single merchant-side command, parameterized by the currently loaded configuration.
///
/// All subcommands of [`Merchant`] should implement this, except [`Configure`], which does not need
/// to start with a valid loaded configuration.
#[async_trait]
pub trait Command {
    async fn run(self, config: merchant::Config) -> Result<(), anyhow::Error>;
}

#[async_trait]
impl Command for Run {
    async fn run(self, config: Config) -> Result<(), anyhow::Error> {
        todo!()
        // Pay.run(&config, chan).await?
    }
}

#[async_trait]
pub trait Method
where
    Self::Protocol: Session,
    <Self::Protocol as Session>::Dual: Session,
{
    type Protocol;

    fn from_config(config: &Config) -> Self;

    async fn run(&self, chan: Chan<Self::Protocol>) -> Result<(), anyhow::Error>;
}

pub async fn main_with_cli(cli: zeekoe::merchant::Cli) -> Result<(), anyhow::Error> {
    let config_path = cli.config.ok_or_else(config_path).or_else(identity)?;
    let config = Config::load(&config_path);

    use cli::Merchant::*;
    match cli.merchant {
        Configure(cli::Configure { .. }) => {
            drop(config);
            tokio::task::spawn_blocking(|| Ok(edit::edit_file(config_path)?)).await?
        }
        Run(run) => run.run(config.await?).await,
    }
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    main_with_cli(Cli::from_args()).await
}
