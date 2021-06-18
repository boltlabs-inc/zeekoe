//! The definitions of the handshake protocol used when starting new connections and resuming broken
//! ones, and implementations of both the client and server side handshakes.

use {
    dialectic::prelude::*,
    dialectic_reconnect::resume,
    serde::{Deserialize, Serialize},
    uuid::Uuid,
};

/// A unique identifier for a client-server session, used when resuming lost connections.
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SessionKey {
    client_key: Uuid,
    server_key: Uuid,
}

impl SessionKey {
    pub fn to_bytes(&self) -> [u8; 32] {
        let mut bytes = [0; 32];
        for (index, byte) in self
            .client_key
            .as_bytes()
            .iter()
            .chain(self.server_key.as_bytes().iter())
            .enumerate()
        {
            bytes[index] = *byte;
        }
        bytes
    }
}

pub(crate) type Handshake = Session! {
    choose {
        0 => {
            // Send a freshly generated client session ID
            send Uuid;
            // Receive a freshly generated server session ID
            recv Uuid;
        },
        1 => {
            // Send the current pair of client/server session ID
            send SessionKey;
        }
    }
};

pub(crate) mod server {
    use super::*;

    #[Transmitter(Tx for Uuid)]
    #[Receiver(Rx for Uuid, SessionKey)]
    pub(crate) async fn handshake<Tx, Rx, E>(
        chan: Chan<<Handshake as Session>::Dual, Tx, Rx>,
    ) -> Result<(resume::ResumeKind, SessionKey), E>
    where
        E: From<Tx::Error> + From<Rx::Error>,
    {
        offer!(in chan {
            0 => {
                let (client_key, chan) = chan.recv().await?;
                let server_key = Uuid::new_v4();
                chan.send(server_key).await?.close();
                Ok((resume::ResumeKind::New, SessionKey { client_key, server_key }))
            },
            1 => {
                let (session_key, chan) = chan.recv().await?;
                chan.close();
                Ok((resume::ResumeKind::Existing, session_key))
            }
        })?
    }
}

pub(super) mod client {
    use super::*;

    #[Transmitter(Tx for Uuid, SessionKey)]
    #[Receiver(Rx for Uuid)]
    pub(crate) async fn init<Tx, Rx, E>(chan: Chan<Handshake, Tx, Rx>) -> Result<SessionKey, E>
    where
        E: From<Tx::Error> + From<Rx::Error>,
    {
        let client_key = Uuid::new_v4();
        let chan = chan.choose::<0>().await?.send(client_key).await?;
        let (server_key, chan) = chan.recv().await?;
        chan.close();
        Ok(SessionKey {
            client_key,
            server_key,
        })
    }

    #[Transmitter(Tx for Uuid, SessionKey)]
    #[Receiver(Rx for Uuid)]
    pub(crate) async fn retry<Tx, Rx, E>(
        key: SessionKey,
        chan: Chan<Handshake, Tx, Rx>,
    ) -> Result<(), E>
    where
        E: From<Tx::Error>,
    {
        chan.choose::<1>().await?.send(key).await?.close();
        Ok(())
    }
}
