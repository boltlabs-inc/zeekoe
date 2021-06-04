use {
    read_restrict::ReadExt,
    std::{
        io::{self, Read},
        path::PathBuf,
        str::FromStr,
    },
    structopt::StructOpt,
};

pub mod customer;
pub mod merchant;

#[derive(Debug, StructOpt)]
#[structopt(name = crate::defaults::shared::APPLICATION)]
pub enum Cli {
    Customer {
        #[structopt(long)]
        config: Option<PathBuf>,
        #[structopt(subcommand)]
        customer: customer::Customer,
    },
    Merchant {
        #[structopt(long)]
        config: Option<PathBuf>,
        #[structopt(subcommand)]
        merchant: merchant::Merchant,
    },
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
        let mut output = String::new();
        io::stdin()
            .lock()
            .restrict(max_length)
            .read_to_string(&mut output)?;
        Ok(output)
    }
}
