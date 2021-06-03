use {std::io::Cursor, structopt::StructOpt, tokio::io::AsyncRead};

use zeekoe::{
    cli::{Account, Customer, Merchant, Note, ZkChannel},
    customer::{self, ChannelName},
    merchant,
};

fn note_contents(note: Option<Note>) -> Box<dyn AsyncRead> {
    match note.unwrap_or_else(|| Note::String(String::new())) {
        Note::Stdin => Box::new(tokio::io::stdin()),
        Note::String(s) => Box::new(Cursor::new(s)),
    }
}

pub fn main() -> Result<(), anyhow::Error> {
    use self::Account::*;

    match ZkChannel::from_args() {
        ZkChannel::Merchant { merchant, config } => {
            use Merchant::*;
            let config = if let Some(config) = config {
                config
            } else {
                merchant::defaults::config_path()?
            };

            match merchant {
                Configure => Ok(edit::edit_file(config)?),
                Run => {
                    let config = merchant::config::load(config)?;
                    todo!();
                    Ok(())
                }
            }
        }
        ZkChannel::Customer { customer, config } => {
            use Customer::*;
            let config = if let Some(config) = config {
                config
            } else {
                customer::defaults::config_path()?
            };

            match customer {
                Account(a) => {
                    let config = customer::config::load(config)?;
                    match a {
                        Import { address } => todo!(),
                        Remove { address } => todo!(),
                    }
                }
                List => todo!(),
                Configure => todo!(),
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
                    let config = customer::config::load(config)?;
                    let note = note_contents(note);
                    let label = label.unwrap_or_else(|| ChannelName::new(merchant.to_string()));
                    todo!()
                }
                Pay { label, pay, note } => {
                    let config = customer::config::load(config)?;
                    let note = note_contents(note);
                    // let merchant: ZkChannelAddress = db.get_merchant(label)?;
                    customer::pay(todo!(), &pay, todo!())?;
                    todo!()
                }
                Refund {
                    label,
                    refund,
                    note,
                } => {
                    let config = customer::config::load(config)?;
                    let note = note_contents(note);
                    // let merchant: ZkChannelAddress = db.get_merchant(label)?;
                    customer::pay(todo!(), &(-1 * refund), todo!())?;
                    todo!()
                }
                Close { label } => todo!(),
            };
        }
    }
}
