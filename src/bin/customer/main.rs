use std::{env, path::Path, time::Duration};
use tokio_rustls::webpki::DNSNameRef;

use dialectic::prelude::*;

use zeekoe::{
    protocol::Ping,
    transport::{pem::read_single_certificate, Backoff, Client, ClientChan},
};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Configure TCP client connection
    let mut backoff = Backoff::with_delay(Duration::from_millis(10));
    backoff
        .exponential(2.0)
        .max_delay(Some(Duration::from_secs(5)))
        .max_retries(10);
    let mut client = Client::new(backoff);
    client.max_length(1024 * 8);

    #[cfg(feature = "allow_explicit_certificate_trust")]
    if let Ok(path_string) = env::var("ZEEKOE_TRUST_EXPLICIT_CERTIFICATE") {
        let path = Path::new(&path_string);
        if path.is_relative() {
            return Err(anyhow::anyhow!("Path specified in `ZEEKOE_TRUST_EXPLICIT_CERTIFICATE` must be absolute, but the current value, \"{}\", is relative", path_string));
        }
        client.trust_explicit_certificate(&read_single_certificate(path)?)?;
    } else {
        eprintln!("no explicit certificate")
    }

    let domain = DNSNameRef::try_from_ascii_str("localhost")?.to_owned();
    let port = 8080;

    // Connect to server
    let mut chan: ClientChan<<Ping as Session>::Dual> = client.connect(domain, port).await?;

    // Enact the client `Ping` protocol
    loop {
        println!("ping");
        chan = chan.send("ping".to_string()).await?.recv().await?.1;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
