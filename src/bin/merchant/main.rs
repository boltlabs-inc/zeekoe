use {async_trait::async_trait, dialectic::Session, std::convert::identity, structopt::StructOpt};

use zeekoe::merchant::{
    cli::{self, Run},
    defaults::config_path,
    Chan, Cli, Config,
};

mod parameters;
mod pay;

use parameters::Parameters;
use pay::Pay;

/// A single merchant-side command, parameterized by the currently loaded configuration.
///
/// All subcommands of [`Merchant`] should implement this, except [`Configure`], which does not need
/// to start with a valid loaded configuration.
#[async_trait]
pub trait Command {
    async fn run(self, config: Config) -> Result<(), anyhow::Error>;
}

#[async_trait]
impl Command for Run {
    async fn run(self, config: Config) -> Result<(), anyhow::Error> {
        todo!()
        // Pay::from_config(config).run(chan).await?
    }
}

#[async_trait]
pub trait Method
where
    Self::Protocol: Session,
    <Self::Protocol as Session>::Dual: Session,
{
    type Protocol;

    async fn run(&self, config: &Config, chan: Chan<Self::Protocol>) -> Result<(), anyhow::Error>;
}

pub async fn main_with_cli(cli: Cli) -> Result<(), anyhow::Error> {
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

#[allow(unused)]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    main_with_cli(Cli::from_args()).await
}
