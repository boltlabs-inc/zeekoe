use std::{env, path::Path, time::Duration};

use zeekoe::{
    customer::{client::Backoff, Chan, Client},
    protocol::Ping,
};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Configure the client
    let mut backoff = Backoff::with_delay(Duration::from_millis(10));
    backoff
        .exponential(2.0)
        .max_delay(Some(Duration::from_secs(5)));
    let mut client: Client<Ping> = Client::new(backoff);
    client
        .max_length(1024 * 8)
        .timeout(Some(Duration::from_secs(10)))
        .max_pending_retries(10);

    // If we've turned on explicit certificate trust, look for the trusted certificate in the
    // environment variable, only accepting it if it's an absolute path (so as to prevent the error
    // where you trust the wrong certificate because you're in the wrong working directory)
    #[cfg(feature = "allow_explicit_certificate_trust")]
    if let Ok(path_string) = env::var("ZEEKOE_TRUST_EXPLICIT_CERTIFICATE") {
        let path = Path::new(&path_string);
        if path.is_relative() {
            return Err(anyhow::anyhow!("Path specified in `ZEEKOE_TRUST_EXPLICIT_CERTIFICATE` must be absolute, but the current value, \"{}\", is relative", path_string));
        }
        client.trust_explicit_certificate(path)?;
    }

    // Connect to `localhost:8080`
    let address = "zkchannel://localhost:8080".parse().unwrap();
    let (_, mut chan): (_, Chan<Ping>) = client.connect(address).await?;

    // Enact the client `Ping` protocol
    loop {
        println!("ping");
        chan = chan.send("ping".to_string()).await?.recv().await?.1;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
