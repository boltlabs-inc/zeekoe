use std::{env, path::Path};
use tokio_rustls::webpki::DNSNameRef;

use dialectic::prelude::*;
use libzkchannels_toolkit::{
    nonce::Nonce,
    parameters::CustomerParameters,
    proofs::PayProof,
    revlock::{
        RevocationLock, RevocationLockBlindingFactor, RevocationLockCommitment, RevocationSecret,
    },
    states::{
        ChannelId, CloseStateCommitment, CustomerBalance, MerchantBalance, State, StateCommitment,
    },
};
use zeekoe::{
    protocol::pay::Customer,
    transport::{connect, read_single_certificate, ClientConfig, TlsClientChan},
};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Configure TCP client connection
    let config = ClientConfig {
        domain: DNSNameRef::try_from_ascii_str("localhost")?.to_owned(),
        port: 8080,
        max_length: 1024 * 8,
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
    let chan: TlsClientChan<Customer> = connect(config).await?;

    // Assemble the information we need to send the server.
    //
    // let mut rng = rand::thread_rng();
    // let merchant_balance = MerchantBalance;
    // let customer_balance = CustomerBalance;
    // let customer_parameters = CustomerParameters {
    //     merchant_signing_pk: PublicKey {
    //         g1: G1Affine::identity(),
    //         y1s: vec![],
    //         g2: G2Affine::identity(),
    //         x2: G2Affine::identity(),
    //         y2s: vec![],
    //     },
    //     commitment_params: todo!(),
    // };
    // let prev_state = State::new(&mut rng, ChannelId, merchant_balance,
    // customer_balance); let (close_state_commitment,
    // close_state_blinding_factor) = state.close_state().commit(&mut rng,
    // &customer_parameters); let nonce: Nonce = *prev_state.nonce();

    // construct PayProof
    // PayProof::new(
    //     &rng,
    //     customer_parameters,
    //     &state,
    // );

    // Enact the client `Customer` protocol
    let chan = chan.send(Nonce).await?;
    let chan = chan.send(PayProof).await?;
    let chan = chan.send(RevocationLockCommitment()).await?;
    let chan = chan.send(CloseStateCommitment()).await?;
    let chan = chan.send(StateCommitment()).await?;

    offer!(in chan {
        0 => {
            println!("Merchant aborted before CustomerRevokePreviousPayToken");
            chan.close();
        },

        1 => {
            let (_close_state_blinding_signature, chan) = chan.recv().await?;
            let chan = chan.choose::<1>().await?;
            let chan = chan.send(RevocationLock).await?;
            let chan = chan.send(RevocationSecret).await?;
            let chan = chan.send(RevocationLockBlindingFactor()).await?;

            offer!(in chan {
                0 => {
                    println!("Merchant aborted before MerchantIssueNewPayToken");
                    chan.close();
                },

                1 => {
                    let (_blinded_pay_token, chan) = chan.recv().await?;
                    println!("Customer completed Pay flow successfully");
                    chan.close();
                },
            })?;
        },
    })?;
    Ok(())
}
