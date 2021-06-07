use structopt::StructOpt;

#[path = "customer/main.rs"]
mod main;

#[path = "merchant/main.rs"]
mod merchant;

#[derive(Debug, StructOpt)]
pub enum Cli {
    Customer {
        #[structopt(subcommand)]
        customer: zeekoe::customer::Cli,
    },
    Merchant {
        #[structopt(subcommand)]
        merchant: zeekoe::merchant::Cli,
    },
}

#[tokio::main]
pub async fn main() -> Result<(), anyhow::Error> {
    use Cli::{Customer, Merchant};
    match Cli::from_args() {
        Merchant { merchant } => merchant::main_with_cli(merchant).await,
        Customer { customer } => main::main_with_cli(customer).await,
    }
}
