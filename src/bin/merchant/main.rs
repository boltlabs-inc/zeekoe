use futures::Future;
use std::{pin::Pin, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use zeekoe::{
    protocol::Ping,
    transport::{pem::read_certificates, pem::read_private_key, Server, ServerChan},
};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let mut server = Server::new(
        read_certificates("localhost.crt")?,
        read_private_key("localhost.key")?,
    );
    server.max_length(1024 * 8);

    // Perform the `Ping` protocol
    let interact = |chan: ServerChan<Ping>, permit| async move {
        let (string, chan) = chan.recv().await?;
        let chan = chan.send(string).await?;
        chan.close();
        drop(permit);
        Ok::<_, anyhow::Error>(())
    };

    server
        .serve_while(([127, 0, 0, 1], 8080), limit_concurrency(1), interact)
        .await?;
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
