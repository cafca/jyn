//! Forge wrapping spaces control/application messages into signed jyn
//! operations on the local profile's `Spaces` log.

use std::sync::{Arc, Mutex};

use p2panda_core::cbor::encode_cbor;
use p2panda_core::{Body, Operation, SigningKey, VerifyingKey};
use p2panda_spaces::{Forge, SpacesArgs};
use p2panda_store::spaces::SpacesMessage;

use crate::domain::{DomainExtensions, DomainOperation, JynOperationDomain};

/// Operations forged for the spaces manager, waiting to be pushed into live
/// gossip by the sync service. The forge appends them to the store (so they
/// are durable and syncable) but has no network handle of its own.
pub type SpacesOutbox = Arc<Mutex<Vec<Operation<DomainExtensions>>>>;

pub struct JynForge {
    domain: JynOperationDomain,
    private_key: SigningKey,
    profile_id: String,
    outbox: SpacesOutbox,
}

impl std::fmt::Debug for JynForge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JynForge")
            .field("profile_id", &self.profile_id)
            .finish_non_exhaustive()
    }
}

impl JynForge {
    pub fn new(
        domain: JynOperationDomain,
        private_key: SigningKey,
        profile_id: String,
        outbox: SpacesOutbox,
    ) -> Self {
        Self {
            domain,
            private_key,
            profile_id,
            outbox,
        }
    }
}

#[derive(Debug)]
pub struct ForgeError(String);

impl std::fmt::Display for ForgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "spaces forge error: {}", self.0)
    }
}

impl std::error::Error for ForgeError {}

impl Forge<()> for JynForge {
    type Message = SpacesMessage<SpacesArgs<()>>;
    type Error = ForgeError;

    fn verifying_key(&self) -> VerifyingKey {
        self.private_key.verifying_key()
    }

    async fn forge(&self, args: SpacesArgs<()>) -> Result<Self::Message, Self::Error> {
        let args_bytes =
            encode_cbor(&args).map_err(|err| ForgeError(format!("encode args: {err}")))?;
        let operation = DomainOperation::Spaces {
            profile_id: self.profile_id.clone(),
            args: args_bytes,
        };
        let body_bytes = encode_cbor(&operation)
            .map_err(|err| ForgeError(format!("encode operation: {err}")))?;

        let mut domain = self.domain.clone();
        let header = domain
            .append_operation(&self.private_key, operation)
            .await
            .map_err(|err| ForgeError(format!("append operation: {err:#}")))?;

        let hash = header.hash();
        let author = header.verifying_key;
        let full_operation = Operation {
            hash,
            header,
            body: Some(Body::from(body_bytes)),
        };
        self.outbox
            .lock()
            .expect("spaces outbox lock poisoned")
            .push(full_operation);

        Ok(SpacesMessage {
            id: hash,
            author,
            args,
        })
    }
}
