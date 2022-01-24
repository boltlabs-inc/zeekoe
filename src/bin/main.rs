use structopt::StructOpt;
use tracing::error;
use tracing_subscriber::EnvFilter;

pub(crate) mod customer;

mod merchant;

#[derive(Debug, StructOpt)]
pub enum Cli {
    Customer(zeekoe::customer::Cli),
    Merchant(zeekoe::merchant::Cli),
}

#[tokio::main]
pub async fn main() {
    let filter = EnvFilter::try_new("info,sqlx::query=warn").unwrap();
    tracing_subscriber::fmt().with_env_filter(filter).init();

    use Cli::{Customer, Merchant};
    let result = match Cli::from_args() {
        Merchant(cli) => merchant::main_with_cli(cli).await,
        Customer(cli) => customer::main_with_cli(cli).await,
    };
    if let Err(e) = result {
        error!("{}, caused by: {}", e, e.root_cause());
    }
}
