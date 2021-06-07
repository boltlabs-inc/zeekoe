use {
    async_trait::async_trait,
    dialectic::{offer, Session},
    futures::stream::{FuturesUnordered, StreamExt},
    std::{convert::identity, net::IpAddr, sync::Arc},
    structopt::StructOpt,
};

use zeekoe::{
    merchant::{
        cli::{self, Run},
        config::Approver,
        defaults::config_path,
        Chan, Cli, Config, Server,
    },
    protocol::ZkChannels,
};

mod parameters;
mod pay;

use parameters::Parameters;
use pay::Pay;

/// A single merchant-side command, parameterized by the currently loaded configuration.
///
/// All subcommands of [`Merchant`] should implement this, except [`Configure`], which does not need
/// to start with a valid loaded configuration.
#[async_trait]
pub trait Command {
    async fn run(self, config: Config) -> Result<(), anyhow::Error>;
}

#[async_trait]
impl Command for Run {
    async fn run(self, config: Config) -> Result<(), anyhow::Error> {
        // TODO: open database connection here, share in an `Arc` between server threads
        // TODO: graceful shutdown

        let config = Arc::new(config);

        let mut server_futures: FuturesUnordered<_> = config
            .services
            .iter()
            .map(|service| {
                let config = config.clone();
                let approve = Arc::new(service.approve.clone());
                async move {
                    let mut server: Server<ZkChannels> =
                        Server::new(&service.certificate, &service.private_key)?;
                    server
                        .timeout(service.connection_timeout)
                        .max_pending_retries(Some(service.max_pending_connection_retries))
                        .max_length(service.max_message_length);
                    server
                        .serve_while(
                            (service.address, service.port),
                            || async { Some(()) },
                            move |chan, ()| {
                                let config = config.clone();
                                let approve = approve.clone();
                                async move {
                                    offer!(in chan {
                                        0 => Parameters.run(&config, chan).await?,
                                        1 => Pay { approve: approve.clone() }.run(&config, chan).await?,
                                    })?;
                                    Ok::<_, anyhow::Error>(())
                                }
                            },
                        )
                        .await?;
                    Ok::<_, anyhow::Error>(())
                }
            })
            .collect();

        while let Some(result) = server_futures.next().await {
            if let Err(e) = result {
                eprintln!("Error: {}", e);
            }
        }

        Ok(())
    }
}

#[async_trait]
pub trait Method
where
    Self::Protocol: Session,
    <Self::Protocol as Session>::Dual: Session,
{
    type Protocol;

    async fn run(&self, config: &Config, chan: Chan<Self::Protocol>) -> Result<(), anyhow::Error>;
}

pub async fn main_with_cli(cli: Cli) -> Result<(), anyhow::Error> {
    let config_path = cli.config.ok_or_else(config_path).or_else(identity)?;
    let config = Config::load(&config_path);

    use cli::Merchant::*;
    match cli.merchant {
        Configure(cli::Configure { .. }) => {
            drop(config);
            tokio::task::spawn_blocking(|| Ok(edit::edit_file(config_path)?)).await?
        }
        Run(run) => run.run(config.await?).await,
    }
}

#[allow(unused)]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    main_with_cli(Cli::from_args()).await
}
