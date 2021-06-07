use {std::path::PathBuf, structopt::StructOpt};

pub use crate::merchant;

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Cli {
    #[structopt(long)]
    pub config: Option<PathBuf>,
    #[structopt(subcommand)]
    pub merchant: Merchant,
}

#[derive(Debug, StructOpt)]
pub enum Merchant {
    Configure(Configure),
    Run(Run),
}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Configure {}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Run {}
