use {
    std::time::Duration,
    zeekoe::{
        merchant::{Chan, Server},
        protocol::Ping,
    },
};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Closure to perform the `Ping` protocol
    let interact = |_, (), mut chan: Chan<Ping>| async move {
        #[allow(unreachable_code)]
        Ok::<_, anyhow::Error>(loop {
            chan = chan.recv().await?.1.send("pong".to_string()).await?;
            println!("pong");
        })
    };

    // Configure the server
    let mut server: Server<Ping> = Server::new("./dev/localhost.crt", "./dev/localhost.key")?;
    server
        .max_length(1024 * 8)
        .timeout(Some(Duration::from_secs(10)))
        .max_pending_retries(Some(10));

    // Run the server
    server
        .serve_while(([127, 0, 0, 1], 8080), || async { Some(()) }, interact)
        .await?;
    Ok(())
}
