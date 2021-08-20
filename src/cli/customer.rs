use {
    read_restrict::ReadExt,
    std::{
        io::{self, Read},
        path::PathBuf,
        str::FromStr,
    },
    structopt::StructOpt,
};

use crate::{amount::Amount, customer::ChannelName, transport::client::ZkChannelAddress};

/// The customer zkChannels command-line interface.
#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Cli {
    /// Path to a configuration file.
    #[structopt(long)]
    pub config: Option<PathBuf>,

    /// Run customer commands.
    #[structopt(subcommand)]
    pub customer: Customer,
}

#[derive(Debug, StructOpt)]
pub enum Customer {
    List(List),
    // Show(Show),
    Configure(Configure),
    Rename(Rename),
    Establish(Establish),
    Pay(Pay),
    Refund(Refund),
    Close(Close),
    Run(Run),
}

/// List all the zkChannels you've established with merchants.
#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct List {}

/// Show details for a single zkChannel.
#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Show {
    /// The channel ID, or a unique prefix of it.
    pub prefix: String,
}

/// Edit the configuration in a text editor.
///
/// This will use the `VISUAL` or `EDITOR` environment variables if they are set.
#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Configure {}

/// Establish a new zkChannel.
#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Establish {
    /// The `zkchannel://` address for the zkChannel.
    pub merchant: ZkChannelAddress,

    /// The amount to be deposited (e.g. 123.45 XTZ).
    #[structopt(long)]
    pub deposit: Amount,

    /// The amount to be deposited by the merchant (e.g. 123.45 XTZ).
    #[structopt(long)]
    pub merchant_deposit: Option<Amount>,

    /// A text description to identify a zkChannel.
    #[structopt(long)]
    pub label: Option<ChannelName>,

    /// A note for the merchant as to why the zkChannel should be established. If you pass `-`, the
    /// value will be read from stdin.
    #[structopt(long)]
    pub note: Option<Note>,

    /// Enable off-chain transactions.
    #[structopt(long)]
    pub off_chain: bool,
}

/// Rename an existing zkChannel.
#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Rename {
    /// The previous label of the channel.
    pub old_label: ChannelName,

    /// An updated label for the channel.
    pub new_label: ChannelName,
}

/// Initiate a payment on a zkChannel.
#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Pay {
    /// A text description to identify a zkChannel.
    pub label: ChannelName,

    /// The amount you wish to pay the merchant (e.g. 123.45 XTZ).
    pub pay: Amount,

    /// A note for the payment. This is sent to the merchant. If you pass `-`, the value will be
    /// read from stdin.
    #[structopt(long)]
    pub note: Option<Note>,
}

impl Pay {
    pub fn into_negative_refund(self) -> Refund {
        let Self { label, pay, note } = self;
        Refund {
            label,
            refund: Amount {
                money: -1 * pay.money,
            },
            note,
        }
    }
}

/// Request a refund from a merchant.
#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Refund {
    /// A text description to identify a zkChannel.
    pub label: ChannelName,

    /// The amount you wish the merchant to refund (e.g. 123.45 XTZ).
    pub refund: Amount,

    /// A note for the refund. This is sent to the merchant. If you pass `-`, the value will be
    /// read from stdin.
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
            pay: Amount {
                money: -1 * refund.money,
            },
            note,
        }
    }
}

/// Close an existing zkChannel.
#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Close {
    /// A text description to identify a zkChannel.
    pub label: ChannelName,
    /// Perform a unilateral close without waiting for the merchant to respond.
    #[structopt(long)]
    pub force: bool,
    /// Enable off-chain transactions.
    #[structopt(long)]
    pub off_chain: bool,
}

#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Run {}

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
