use {async_trait::async_trait, structopt::StructOpt};

use crate::{
    amount::{parse_amount, Amount},
    customer::{self, AccountName, ChannelName},
    transport::client::ZkChannelAddress,
};

pub use crate::cli::Note;

#[derive(Debug, StructOpt)]
pub enum Customer {
    Account(Account),
    List(List),
    Configure(Configure),
    Rename(Rename),
    Establish(Establish),
    Pay(Pay),
    Refund(Refund),
    Close(Close),
}

/// A single customer-side command, parameterized by the currently loaded configuration.
///
/// All subcommands of [`Customer`] should implement this, except [`Configure`], which does not need
/// to start with a valid loaded configuration.
#[async_trait]
pub trait Command {
    async fn run(self, config: customer::Config) -> Result<(), anyhow::Error>;
}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct List {}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Configure {}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Establish {
    pub merchant: ZkChannelAddress,
    #[structopt(parse(try_from_str = parse_amount))]
    pub deposit: Amount,
    #[structopt(long)]
    pub from: AccountName,
    #[structopt(long)]
    pub label: Option<ChannelName>,
    #[structopt(long)]
    pub note: Option<Note>,
}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Rename {
    pub old_label: ChannelName,
    pub new_label: ChannelName,
}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Pay {
    pub label: ChannelName,
    #[structopt(parse(try_from_str = parse_amount))]
    pub pay: Amount,
    #[structopt(long)]
    pub note: Option<Note>,
}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Refund {
    pub label: ChannelName,
    #[structopt(parse(try_from_str = parse_amount))]
    pub refund: Amount,
    #[structopt(long)]
    pub note: Option<Note>,
}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Close {
    pub label: ChannelName,
}

#[derive(Debug, StructOpt)]
pub struct Import {
    pub address: Option<String>,
}

#[derive(Debug, StructOpt)]
pub struct Remove {
    pub address: Option<String>,
}

#[derive(Debug, StructOpt)]
pub enum Account {
    Import(Import),
    Remove(Remove),
}
