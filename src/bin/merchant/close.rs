//* Close functionalities for a merchant.
//*
//* TODO: handle merchant expiry closes.
use {anyhow::Context, async_trait::async_trait, rand::rngs::StdRng};

use super::{database, Command};
use rand::SeedableRng;

use zeekoe::{
    abort,
    merchant::{cli, config::Service, database::QueryMerchant, server::SessionKey, Chan, Config},
    offer_abort, proceed,
    protocol::{self, close, ChannelStatus, Party::Merchant},
};

use zkabacus_crypto::{merchant::Config as MerchantConfig, ChannelId, CloseState, Verification};

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

        // TODO: Generate an authorization signature under the merchant's EDDSA Tezos key.
        // The signature should be over a tuple with
        // (contract id, "zkChannels mutual close", channel id, customer balance, merchant balance).

        // Give the customer the opportunity to reject an invalid authorization signature.
        offer_abort!(in chan as Merchant);

        // Close the dialectic channel.
        chan.close();

        // Update database to indicate channel is now pending close.
        // Note: mutual close can only be called on an active channel. Any other state requires
        // a unilateral close.
        database
            .compare_and_swap_channel_status(
                close_state.channel_id(),
                &ChannelStatus::Active,
                &ChannelStatus::PendingClose,
            )
            .await
            .context("Failed to update database to indicate channel is pending close")?;

        Ok(())
    }
}

/// Process a customer close event.
///
/// **Usage**: this should be called after receiving a notification that a customer close entrypoint
/// was posted on chain. Should not wait for the transaction to be confirmed at the required confirmation depth.
#[allow(unused)]
async fn process_customer_close() -> Result<(), anyhow::Error> {
    // TODO: Extract revocation lock from notification and atomically
    // - check that it is fresh (e.g. not in the database with a revocation secret),
    // - insert it into the database,
    // - retrieve any secrets already associated with the lock.

    // TODO: if the lock *does not* have a revocation secret, update channel status to PENDING_CLOSE.

    // TODO: If the lock already has an associated revocation secret, update channel status to DISPUTE.

    // TODO: If the lock already has an associated revocation secret, call the merchant dispute
    // entrypoint with:
    // - contract id
    // - revocation secret
    // E.g. call the "dispute" function from escrow API.

    todo!()
}

/// Process a confirmed customer close event.
///
/// **Usage**: this should be called after receiving a notification that a customer close entrypoint
/// was posted on chain *and* is confirmed at the required confirmation depth.
#[allow(unused)]
async fn finalize_customer_close() -> Result<(), anyhow::Error> {
    // TODO: if database status is PENDING_CLOSE, update database channel status to CLOSED and set
    // final balances.

    // TODO: if database status is DISPUTE, update merchant final balance to include the merchant
    // balance. (process_customer_close should have already set the other dispute actions in motion).

    todo!()
}

/// Process a confirmed merchant dispute event.
///
/// **Usage**: this should be called after receiving a notification that a merchant dispute
/// entrypoint operation is confirmed at the required confirmation depth.
#[allow(unused)]
async fn finalize_dispute() -> Result<(), anyhow::Error> {
    // TODO: assert that status is DISPUTE
    // If so, update database channel status to CLOSED.
    // Update final balances to indicate successful dispute (transfer customer balance to merchant).

    todo!()
}

// Process a mutual close event.
//
// **Usage**: this should be called after receiving a notification that a mutual close operation
// was posted on chain and confirmed to the required depth.
#[allow(unused)]
async fn finalize_mutual_close(
    merchant_config: &MerchantConfig,
    database: &dyn QueryMerchant,
    channel_id: &ChannelId,
) -> Result<(), anyhow::Error> {
    // Update database to indicate the channel closed successfully.
    database
        .compare_and_swap_channel_status(
            channel_id,
            &ChannelStatus::PendingClose,
            &ChannelStatus::Closed,
        )
        .await
        .context("Failed to update database to indicate channel is closed")?;

    // TODO: also update database to indicate final channel balances.
    Ok(())
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
        .context("Failed to receive close state signature")?;

    let (close_state, chan) = chan
        .recv()
        .await
        .context("Failed to receive close state.")?;

    // Confirm that customer sent a valid Pointcheval-Sanders signature under the merchant's
    // zkAbacus public key on the given close state.
    // If so, atomically check that the close state contains a fresh revocation lock and add it
    // to the database.
    // Otherwise, abort with an error.
    match merchant_config.check_close_signature(close_signature, &close_state) {
        Verification::Verified => {
            // Check that the revocation lock is fresh and insert.
            if database
                .insert_revocation(close_state.revocation_lock(), None)
                .await
                .context("Failed to insert revocation lock into database")?
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
        // Abort if the close signature was invalid.
        Verification::Failed => abort!(in chan return close::Error::InvalidCloseStateSignature),
    }
}

#[async_trait]
impl Command for cli::Close {
    async fn run(self, config: Config) -> Result<(), anyhow::Error> {
        // Retrieve zkAbacus config from the database
        let database = database(&config).await?;

        // Either initialize the merchant's config afresh, or get existing config if it exists (it should already exist)
        let merchant_config = database
            .fetch_or_create_config(&mut StdRng::from_entropy()) // TODO: allow determinism
            .await?;

        // Make sure exactly one correct command line option is satisfied.
        if (self.all && self.channel.is_some()) || (!self.all && self.channel.is_none()) {
            return Err(anyhow::anyhow!(
                "Invalid command line option: should specify exactly one of --all or --channel."
            ));
        }
        match self.channel {
            Some(channel_id) => expiry(&merchant_config, database.as_ref(), &channel_id).await,
            None => {
                // TODO: iterate through database; call expiry for every channel.
                Err(anyhow::anyhow!(
                    "Closing all channels is not yet implemented."
                ))
            }
        }
    }
}

/// Initiate close procedures with an expiry transaction.
///
/// **Usage**: this is called directly from the command line.
async fn expiry(
    _merchant_config: &MerchantConfig,
    _database: &dyn QueryMerchant,
    _channel_id: &ChannelId,
) -> Result<(), anyhow::Error> {
    // TODO: Update database status to PENDING_CLOSE

    // TODO: call expiry entrypoint, which will take
    // - contract ID
    // Raise an error if it fails.
    //
    // This function will:
    // - Generate merchant authorization EDDSA signature on the operation with the merchant's
    //   Tezos public key.
    // - Send operation to blockchain

    Ok(())
}

/// Claim the channel balances.
///
/// **Usage**: this is called in response to an on-chain event: when the expiry operation
/// is confirmed on chain to an appropriate depth _and_ the timelock period has passed without
/// any other operation being posted to the contract.
#[allow(unused)]
async fn claim_funds() {
    // TODO: Assert database status is PENDING_CLOSE

    // TODO: call merchClaim entrypoint, which will take
    // - contract ID
    // Raise an error if it fails.
    //
    // This function will transfer all the channel funds to the merchant account.
}

/// Finalize the channel balances for a merchant-closed channel.
///
/// **Usage**: this is called after the merchClaim operation is confirmed on chain to an appropriate
/// depth.
#[allow(unused)]
async fn finalize_close() -> Result<(), anyhow::Error> {
    // TODO: assert database status is PENDING_CLOSE.
    // TODO: update database status to CLOSED with the correct balances.
    Ok(())
}
