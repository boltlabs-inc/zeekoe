use futures::Future;
use std::{convert::TryInto, pin::Pin, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use zeekoe::{
    protocol::Ping,
    transport::{read_certificates, read_private_key, serve_while, ServerConfig, TlsServerChan},
};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let config = ServerConfig {
        private_key: read_private_key("localhost.key")?,
        certificate_chain: read_certificates("localhost.crt")?,
        address: ([127, 0, 0, 1], 8080).try_into()?,
        max_length: 1024 * 8,
    };

    // Perform the `Ping` protocol
    let interact = |chan: TlsServerChan<Ping>, permit| async move {
        let (string, chan) = chan.recv().await?;
        let chan = chan.send(string).await?;
        chan.close();
        drop(permit);
        Ok::<_, anyhow::Error>(())
    };

    serve_while(config, limit_concurrency(1), interact).await?;
    Ok(())
}

fn limit_concurrency(
    max_concurrent: usize,
) -> impl FnMut() -> Pin<Box<dyn Future<Output = Option<OwnedSemaphorePermit>>>> {
    let permit = Arc::new(Semaphore::new(max_concurrent));
    move || {
        let permit = permit.clone();
        Box::pin(async move { Some(permit.clone().acquire_owned().await.unwrap()) })
    }
}
