use structopt::StructOpt;

#[path = "customer/main.rs"]
mod customer;

#[path = "merchant/main.rs"]
mod merchant;

#[derive(Debug, StructOpt)]
pub enum Cli {
    Customer(zeekoe::customer::Cli),
    Merchant(zeekoe::merchant::Cli),
}

#[tokio::main]
pub async fn main() -> Result<(), anyhow::Error> {
    use Cli::{Customer, Merchant};
    match Cli::from_args() {
        Merchant(cli) => merchant::main_with_cli(cli).await,
        Customer(cli) => customer::main_with_cli(cli).await,
    }
}
