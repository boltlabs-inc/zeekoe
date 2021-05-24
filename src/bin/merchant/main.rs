use zeekoe::{
    protocol::Ping,
    transport::{
        pem::read_certificates,
        pem::read_private_key,
        server::{Chan, Server},
    },
};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let mut server: Server<Ping> = Server::new(
        read_certificates("./dev/localhost.crt")?,
        read_private_key("./dev/localhost.key")?,
    );
    server.max_length(1024 * 8);

    // Perform the `Ping` protocol
    let interact = |mut chan: Chan<Ping>, ()| async move {
        #[allow(unreachable_code)]
        Ok::<_, anyhow::Error>(loop {
            chan = chan.recv().await?.1.send("pong".to_string()).await?;
            println!("pong");
        })
    };

    server
        .serve_while(([127, 0, 0, 1], 8080), || async { Some(()) }, interact)
        .await?;
    Ok(())
}
