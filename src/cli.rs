use {
    rusty_money::{crypto, Money, MoneyError},
    std::str::FromStr,
    structopt::StructOpt,
    thiserror::Error,
    webpki::{DNSName, DNSNameRef, InvalidDNSNameError},
};

#[derive(Debug, StructOpt)]
pub enum Zeekoe {
    Customer(Customer),
    Merchant(Merchant),
}

#[derive(Debug, StructOpt)]
pub enum Customer {
    Init {
        #[structopt(long, short)]
        interactive: bool,
    },
    Account(Account),
    Channel(Channel),
}

#[derive(Debug)]
pub struct MerchantAddress {
    domain: DNSName,
    port: Option<u16>,
}

impl FromStr for MerchantAddress {
    type Err = InvalidAddress;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(if let Some((domain, port)) = s.split_once(':') {
            let domain = DNSNameRef::try_from_ascii_str(&domain)
                .map_err(InvalidAddress::InvalidDNSName)
                .map(|d| d.to_owned())?;
            let port = Some(
                port.parse::<u16>()
                    .map_err(|_| InvalidAddress::InvalidPort(port.to_string()))?,
            );
            MerchantAddress { domain, port }
        } else {
            let domain = DNSNameRef::try_from_ascii_str(s)
                .map_err(InvalidAddress::InvalidDNSName)
                .map(|d| d.to_owned())?;
            MerchantAddress { domain, port: None }
        })
    }
}

#[derive(Debug, Error)]
pub enum InvalidAddress {
    #[error("{0}")]
    InvalidDNSName(InvalidDNSNameError),
    #[error("Invalid port number: {0}")]
    InvalidPort(String),
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

#[derive(Debug, StructOpt)]
pub enum Channel {
    List,
    New {
        label: String,
        merchant: MerchantAddress,
    },
    Fund {
        label: String,
        #[structopt(parse(try_from_str = parse_amount))]
        deposit: Amount,
        #[structopt(long)]
        from: AccountName,
    },
    Pay {
        label: String,
        #[structopt(parse(try_from_str = parse_amount))]
        pay: Amount,
        #[structopt(long)]
        note: Option<Note>,
    },
    Refund {
        label: String,
        #[structopt(parse(try_from_str = parse_amount))]
        refund: Amount,
        #[structopt(long)]
        note: Option<Note>,
    },
    Close {
        label: String,
    },
}

#[derive(Debug)]
pub enum AmountParseError {
    UnknownCurrency(String),
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

#[derive(Debug, StructOpt)]
pub enum Merchant {
    Init {
        #[structopt(long)]
        interactive: bool,
    },
    Run {},
}
