use {anyhow::Context, async_trait::async_trait, rand::rngs::StdRng};

use zeekoe::{
    abort,
    merchant::{config::Service, database::QueryMerchant, server::SessionKey, Chan},
    offer_abort, proceed,
    protocol::{self, close, ChannelStatus, Party::Merchant},
};

use zkabacus_crypto::{merchant::Config as MerchantConfig, CloseState, Verification};

use super::Method;

pub struct Close;

#[async_trait]
impl Method for Close {
    type Protocol = protocol::Close;

    async fn run(
        &self,
        _rng: StdRng,
        _client: &reqwest::Client,
        _service: &Service,
        merchant_config: &MerchantConfig,
        database: &dyn QueryMerchant,
        _session_key: SessionKey,
        chan: Chan<Self::Protocol>,
    ) -> Result<(), anyhow::Error> {
        let (chan, close_state) = zkabacus_close(merchant_config, database, chan)
            .await
            .context("Mutual close failed")?;

        // TODO: generate authorization signature and send to customer.

        // Give the customer the opportunity to reject an invalid auth signature.
        offer_abort!(in chan as Merchant);

        // Close the channel - all remaining operations are with the escrow agent.
        chan.close();

        // TODO: confirm that arbiter accepted the close request (posted by customer).

        database
            .compare_and_swap_channel_status(
                close_state.channel_id(),
                &ChannelStatus::Active,
                &ChannelStatus::Closed,
            )
            .await
            .context("Failed to update database to indicate channel was closed.")?;

        Ok(())
    }
}

async fn zkabacus_close(
    merchant_config: &MerchantConfig,
    database: &dyn QueryMerchant,
    chan: Chan<close::CustomerSendSignature>,
) -> Result<(Chan<close::MerchantSendAuthorization>, CloseState), anyhow::Error> {
    // Receive close signature and state from customer.
    let (close_signature, chan) = chan
        .recv()
        .await
        .context("Failed to receive close state signature.")?;

    let (close_state, chan) = chan
        .recv()
        .await
        .context("Failed to receive close state.")?;

    // Check validity of close materials from the customer.
    match merchant_config.check_close_signature(close_signature, &close_state) {
        Verification::Verified => {
            // If valid, check that the close state hasn't been seen before.
            if database
                .insert_revocation(close_state.revocation_lock(), None)
                .await
                .context("Failed to insert revocation lock into database.")?
                .is_empty()
            {
                // If it's fresh, continue with protocol.
                proceed!(in chan);
                Ok((chan, close_state))
            } else {
                // If it has been seen before, abort.
                abort!(in chan return close::Error::KnownRevocationLock)
            }
        }
        // Abort if the close materials were invalid.
        Verification::Failed => abort!(in chan return close::Error::InvalidCloseStateSignature),
    }
}
