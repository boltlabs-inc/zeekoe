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
    List(List),
    Show(Show),
    Configure(Configure),
    Run(Run),
    Close(Close),
}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct List {}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Show {
    #[structopt(empty_values(false))]
    pub prefix: String,
}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Configure {}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Run {}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Close {
    #[structopt(long)]
    pub all: bool,

    /// TODO: replace this with a ChannelId or a ChannelIdPrefix of some flavor.
    #[structopt(long)]
    pub channel: bool,
}
