use std::{env, path::Path};
use tokio_rustls::webpki::DNSNameRef;

use dialectic::prelude::*;

use zeekoe::{
    protocol::Ping,
    transport::{connect, read_single_certificate, ClientChan, ClientConfig},
};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Configure TCP client connection
    let config = ClientConfig {
        domain: DNSNameRef::try_from_ascii_str("localhost")?.to_owned(),
        port: 8080,
        max_length: 1024 * 8,
        length_field_bytes: 4,
        #[cfg(feature = "allow_explicit_certificate_trust")]
        trust_explicit_certificate: if let Ok(path_string) =
            env::var("ZEEKOE_TRUST_EXPLICIT_CERTIFICATE")
        {
            let path = Path::new(&path_string);
            if path.is_relative() {
                return Err(anyhow::anyhow!("Path specified in `ZEEKOE_TRUST_EXPLICIT_CERTIFICATE` must be absolute, but the current value, \"{}\", is relative", path_string));
            }
            Some(read_single_certificate(path)?)
        } else {
            println!("no explicit cert");
            None
        },
    };

    // Connect to server
    let chan: ClientChan<<Ping as Session>::Dual> = connect(config).await?;

    // Enact the client `Ping` protocol
    let chan = chan.send("ping".to_string()).await?;
    let (response, chan) = chan.recv().await?;
    chan.close();
    println!("{}", response);

    Ok(())
}
