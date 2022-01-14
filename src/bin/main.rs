use structopt::StructOpt;
use tracing_subscriber::EnvFilter;

#[path = "customer/main.rs"]
pub(crate) mod customer;

#[path = "merchant/main.rs"]
mod merchant;

#[derive(Debug, StructOpt)]
pub enum Cli {
    Customer(zeekoe::customer::Cli),
    Merchant(zeekoe::merchant::Cli),
}

#[tokio::main]
pub async fn main() -> Result<(), anyhow::Error> {
    let filter = EnvFilter::try_new("info,sqlx::query=warn")?;
    tracing_subscriber::fmt().with_env_filter(filter).init();

    use Cli::{Customer, Merchant};
    match Cli::from_args() {
        Merchant(cli) => merchant::main_with_cli(cli).await,
        Customer(cli) => customer::main_with_cli(cli).await,
    }
}
