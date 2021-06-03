use {
    std::{path::PathBuf, str::FromStr},
    structopt::StructOpt,
};

use crate::{
    amount::{parse_amount, Amount},
    customer::{AccountName, ChannelName},
    transport::client::ZkChannelAddress,
};

#[derive(Debug, StructOpt)]
#[structopt(name = crate::defaults::shared::APPLICATION)]
pub enum ZkChannel {
    Customer {
        #[structopt(long)]
        config: Option<PathBuf>,
        #[structopt(subcommand)]
        customer: Customer,
    },
    Merchant {
        #[structopt(long)]
        config: Option<PathBuf>,
        #[structopt(subcommand)]
        merchant: Merchant,
    },
}

#[derive(Debug, StructOpt)]
pub enum Customer {
    Account(Account),
    List,
    Configure,
    Rename {
        old_label: ChannelName,
        new_label: ChannelName,
    },
    Establish {
        merchant: ZkChannelAddress,
        #[structopt(parse(try_from_str = parse_amount))]
        deposit: Amount,
        #[structopt(long)]
        from: AccountName,
        #[structopt(long)]
        label: Option<ChannelName>,
        #[structopt(long)]
        note: Option<Note>,
    },
    Pay {
        label: ChannelName,
        #[structopt(parse(try_from_str = parse_amount))]
        pay: Amount,
        #[structopt(long)]
        note: Option<Note>,
    },
    Refund {
        label: ChannelName,
        #[structopt(parse(try_from_str = parse_amount))]
        refund: Amount,
        #[structopt(long)]
        note: Option<Note>,
    },
    Close {
        label: ChannelName,
    },
}

#[derive(Debug, StructOpt)]
pub enum Merchant {
    Configure,
    Run,
}

#[derive(Debug, StructOpt)]
pub enum Account {
    Import { address: Option<String> },
    Remove { address: Option<String> },
}

#[derive(Debug)]
pub enum Note {
    Stdin,
    String(String),
}

impl FromStr for Note {
    type Err = std::convert::Infallible;

    fn from_str(str: &str) -> Result<Self, Self::Err> {
        if str == "-" {
            Ok(Note::Stdin)
        } else {
            Ok(Note::String(str.to_string()))
        }
    }
}
