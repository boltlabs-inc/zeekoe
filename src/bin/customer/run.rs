use dialectic::offer;
use {
    async_trait::async_trait,
    rand::rngs::StdRng,
    std::net::{IpAddr, Ipv4Addr},
    std::sync::Arc,
    tokio::sync::broadcast,
};

use zeekoe::{
    customer::server::{Chan, Server},
    customer::{cli::Run, Config},
    protocol::daemon::Daemon,
};

use super::{database, Command};
use anyhow::Context;

#[async_trait]
impl Command for Run {
    async fn run(self, _rng: StdRng, config: Config) -> Result<(), anyhow::Error> {
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;

        let config = Arc::new(config);

        // Sender and receiver to indicate graceful shutdown should occur
        let (terminate, _) = broadcast::channel(1);
        let mut wait_terminate = terminate.subscribe();

        // Initialize a new `Server` with parameters taken from the configuration
        let server: Server<Daemon> = Server::new();

        // Serve on this address
        let localhost_v4 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let address = (localhost_v4, config.daemon.port);

        // There is no meaningful initialization necessary per request
        let initialize = || async { Some(()) };

        // For each request, dispatch to the appropriate method, defined elsewhere
        let interact = move |_session_key, (), _chan: Chan<Daemon>| {
            // Clone `Arc`s for the various resources we need in this request
            let _database = database.clone();

            async move {
                offer!(in _chan {
                    // Refresh
                    0 => {
                        println!("refreshed");
                        Ok::<_, anyhow::Error>(())
                    }
                })?
            }
        };

        // Future that completes on graceful shutdown
        let wait_terminate = async move { wait_terminate.recv().await.unwrap_or(()) };

        server
            .serve_while(address, None, initialize, interact, wait_terminate)
            .await?;

        Ok::<_, anyhow::Error>(())
    }
}
