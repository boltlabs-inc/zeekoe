use {
    http::uri::{InvalidUri, Uri},
    rusty_money::{crypto, Money, MoneyError},
    std::{
        fmt::{self, Display},
        io::{self, Cursor},
        str::FromStr,
    },
    structopt::StructOpt,
    thiserror::Error,
    tokio::io::AsyncRead,
    webpki::{DNSName, DNSNameRef, InvalidDNSNameError},
};

pub fn main() -> Result<(), anyhow::Error> {
    use self::Account::*;
    use Customer::*;
    use Merchant::*;

    fn note_contents(note: Option<Note>) -> Box<dyn AsyncRead> {
        match note.unwrap_or_else(|| Note::String(String::new())) {
            Note::Stdin => Box::new(tokio::io::stdin()),
            Note::String(s) => Box::new(Cursor::new(s)),
        }
    }

    match ZkChannel::from_args() {
        ZkChannel::Merchant(m) => match m {
            Run {} => todo!(),
        },
        ZkChannel::Customer(c) => {
            let config: config::customer::Config = config::customer::load()?;
            let db = db::customer::open(config.database)?;
            match c {
                Account(a) => match a {
                    Import { address } => todo!(),
                    Remove { address } => todo!(),
                },
                List => todo!(),
                Rename {
                    old_label,
                    new_label,
                } => todo!(),
                Establish {
                    merchant,
                    deposit,
                    from,
                    label,
                    note,
                } => {
                    let note = note_contents(note);
                    let label = label.unwrap_or_else(|| ChannelName(merchant.to_string()));
                    todo!()
                }
                Pay { label, pay, note } => {
                    let note = note_contents(note);
                    let merchant: ZkChannelAddress = db.get_merchant(label)?;
                    crate::customer::pay(merchant, pay, note)?;
                }
                Refund {
                    label,
                    refund,
                    note,
                } => {
                    let note = note_contents(note);
                    todo!()
                }
                Close { label } => todo!(),
            };
        }
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "zkchannel")]
pub enum ZkChannel {
    Customer(Customer),
    Merchant(Merchant),
}

#[derive(Debug, StructOpt)]
pub enum Customer {
    Account(Account),
    List,
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
    Run {},
}

pub type Amount = Money<'static, crypto::Currency>;

/// Parse an amount specified like "100.00 XTZ"
fn parse_amount(str: &str) -> Result<Amount, MoneyError> {
    if let Some((amount, currency)) = str.split_once(' ') {
        let currency = crypto::find(currency).ok_or(MoneyError::InvalidCurrency)?;
        Money::from_str(amount, currency)
    } else {
        Err(MoneyError::InvalidAmount)
    }
}

#[derive(Debug)]
pub struct AccountName(String);

impl FromStr for AccountName {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(AccountName(s.to_string()))
    }
}

#[derive(Debug)]
pub struct ChannelName(String);

impl FromStr for ChannelName {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(ChannelName(s.to_string()))
    }
}

/// The address of a zkChannels merchant: a URI of the form `zkchannel://some.domain.com:2611` with
/// an optional port number.
#[derive(Debug, Clone)]
pub struct ZkChannelAddress {
    host: DNSName,
    port: Option<u16>,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum InvalidZkChannelAddress {
    #[error("Incorrect URI scheme: expecting `zkchannel://`")]
    IncorrectScheme,
    #[error("Unexpected non-root path in `zkchannel://` address")]
    UnsupportedPath,
    #[error("Unexpected query string in `zkchannel://` address")]
    UnsupportedQuery,
    #[error("Missing hostname in `zkchannel://` address")]
    MissingHost,
    #[error("Invalid DNS hostname in `zkchannel://` address: {0}")]
    InvalidDnsName(InvalidDNSNameError),
    #[error("Invalid `zkchannel://` address: {0}")]
    InvalidUri(InvalidUri),
}

impl FromStr for ZkChannelAddress {
    type Err = InvalidZkChannelAddress;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uri: Uri = s.parse().map_err(InvalidZkChannelAddress::InvalidUri)?;
        if uri.scheme_str() != Some("zkchannel") {
            Err(InvalidZkChannelAddress::IncorrectScheme)
        } else if uri.path() != "" && uri.path() != "/" {
            Err(InvalidZkChannelAddress::UnsupportedPath)
        } else if uri.query().is_some() {
            Err(InvalidZkChannelAddress::UnsupportedQuery)
        } else if let Some(host) = uri.host() {
            Ok(ZkChannelAddress {
                host: DNSNameRef::try_from_ascii_str(host)
                    .map_err(InvalidZkChannelAddress::InvalidDnsName)?
                    .to_owned(),
                port: uri.port_u16(),
            })
        } else {
            Err(InvalidZkChannelAddress::MissingHost)
        }
    }
}

impl Display for ZkChannelAddress {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let host: &str = self.host.as_ref().into();
        write!(f, "zkchannel://{}", host)?;
        if let Some(port) = self.port {
            write!(f, ":{}", port)?;
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum AmountParseError {
    #[error("Unknown currency: {0}")]
    UnknownCurrency(String),
    #[error("Invalid format for currency amount")]
    InvalidFormat,
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

#[derive(Debug, StructOpt)]
pub enum Account {
    Import { address: Option<String> },
    Remove { address: Option<String> },
}
