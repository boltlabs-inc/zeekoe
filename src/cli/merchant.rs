use {async_trait::async_trait, structopt::StructOpt};

pub use crate::{cli::Note, merchant};

#[derive(Debug, StructOpt)]
pub enum Merchant {
    Configure(Configure),
    Run(Run),
}

/// A single merchant-side command, parameterized by the currently loaded configuration.
///
/// All subcommands of [`Merchant`] should implement this, except [`Configure`], which does not need
/// to start with a valid loaded configuration.
#[async_trait]
pub trait Command {
    async fn run(self, config: merchant::Config) -> Result<(), anyhow::Error>;
}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Configure {}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Run {}
