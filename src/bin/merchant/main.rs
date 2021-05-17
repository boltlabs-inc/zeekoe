use futures::Future;
use std::{convert::TryInto, pin::Pin, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use dialectic::prelude::*;
use libzkchannels_toolkit::states::{BlindedPayToken, CloseStateBlindedSignature};
use zeekoe::{
    protocol::pay::Merchant,
    transport::{read_certificates, read_private_key, serve_while, ServerConfig, TlsServerChan},
};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let config = ServerConfig {
        private_key: read_private_key("dev/localhost.key")?,
        certificate_chain: read_certificates("dev/localhost.crt")?,
        address: ([127, 0, 0, 1], 8080).try_into()?,
        max_length: 1024 * 8,
    };

    // Perform the server `Merchant` protocol
    let interact = |chan: TlsServerChan<Merchant>, permit| async move {
        let (_nonce, chan) = chan.recv().await?;
        let (_pay_proof, chan) = chan.recv().await?;
        let (_revlock_commitment, chan) = chan.recv().await?;
        let (_close_state_commitment, chan) = chan.recv().await?;
        let (_state_commitment, chan) = chan.recv().await?;
        let chan = chan.choose::<1>().await?;
        let chan = chan.send(CloseStateBlindedSignature).await?;

        offer!(in chan {
            0 => {
                println!("Customer aborted pay");
                chan.close();
            },

            1 => {
                let (_revlock, chan) = chan.recv().await?;
                let (_revsecret, chan) = chan.recv().await?;
                let (_revlock_blinding_factor, chan) = chan.recv().await?;
                let chan = chan.choose::<1>().await?;
                let chan = chan.send(BlindedPayToken()).await?;
                println!("Merchant completed Pay flow successfully");
                chan.close();
            },
        })?;

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
