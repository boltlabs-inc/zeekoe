use {
    read_restrict::ReadExt,
    std::{
        io::{self, Read},
        path::PathBuf,
        str::FromStr,
    },
    structopt::StructOpt,
};

use crate::{
    amount::{parse_amount, Amount},
    customer::{AccountName, ChannelName},
    transport::client::ZkChannelAddress,
};

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Cli {
    #[structopt(long)]
    pub config: Option<PathBuf>,
    #[structopt(subcommand)]
    pub customer: Customer,
}

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

impl Pay {
    pub fn into_negative_refund(self) -> Refund {
        let Self { label, pay, note } = self;
        Refund {
            label,
            refund: -1 * pay,
            note,
        }
    }
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

impl Refund {
    pub fn into_negative_pay(self) -> Pay {
        let Self {
            label,
            refund,
            note,
        } = self;
        Pay {
            label,
            pay: -1 * refund,
            note,
        }
    }
}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Close {
    pub label: ChannelName,
    #[structopt(long)]
    pub force: bool,
}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Import {
    pub address: Option<String>,
}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Remove {
    pub address: Option<String>,
}

#[derive(Debug, StructOpt)]
pub enum Account {
    Import(Import),
    Remove(Remove),
}

/// An argument specified on the command line which may be a string literal, or the special string
/// `-`, which indicates that the value should be read from standard input.
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

impl Note {
    pub fn read(self, max_length: u64) -> Result<String, io::Error> {
        match self {
            Note::Stdin => {
                let mut output = String::new();
                io::stdin()
                    .lock()
                    .restrict(max_length)
                    .read_to_string(&mut output)?;
                Ok(output)
            }
            Note::String(s) => {
                if s.len() as u64 <= max_length {
                    Ok(s)
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Read restriction exceeded",
                    ))
                }
            }
        }
    }
}
