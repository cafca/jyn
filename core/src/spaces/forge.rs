//! Forge wrapping spaces control/application messages into signed jyn
//! operations on the local profile's `Spaces` log.

use std::sync::{Arc, Mutex};

use p2panda_core::cbor::encode_cbor;
use p2panda_core::{Body, Operation, SigningKey, VerifyingKey};
use p2panda_spaces::{Forge, SpacesArgs};
use p2panda_store::spaces::SpacesMessage;

use crate::domain::{DomainExtensions, DomainLogId, DomainOperation, JynOperationDomain};

/// Operations forged for the spaces manager, waiting to be pushed into live
/// gossip by the sync service. The forge appends them to the store (so they
/// are durable and syncable) but has no network handle of its own.
pub type SpacesOutbox = Arc<Mutex<Vec<Operation<DomainExtensions>>>>;

/// Where the *next* encrypted application payload should be placed
/// (ADR-0016). [`crate::spaces::JynSpaces`] computes the expiry bucket of the
/// inner operation and parks its log id here right before `space.publish`,
/// under the spaces ops-lock; the forge consumes it when it forges the
/// matching `Application` wrapper. Control messages ignore it entirely.
pub type PlacementHint = Arc<Mutex<Option<DomainLogId>>>;

pub struct JynForge {
    domain: JynOperationDomain,
    private_key: SigningKey,
    profile_id: String,
    outbox: SpacesOutbox,
    placement_hint: PlacementHint,
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
        placement_hint: PlacementHint,
    ) -> Self {
        Self {
            domain,
            private_key,
            profile_id,
            outbox,
            placement_hint,
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
        // An encrypted application payload co-deletes with the post it carries,
        // so it goes into that post's expiry bucket (the hint JynSpaces parked).
        // Control traffic — key bundles, membership, re-keys — is not tied to
        // any post's lifetime and stays on the reserved control log. Take the
        // hint (leaving None) so it applies to exactly one Application forge.
        let placement = match &args {
            SpacesArgs::Application { .. } => self
                .placement_hint
                .lock()
                .expect("spaces placement hint lock poisoned")
                .take(),
            _ => None,
        };

        let args_bytes =
            encode_cbor(&args).map_err(|err| ForgeError(format!("encode args: {err}")))?;
        let operation = DomainOperation::Spaces {
            profile_id: self.profile_id.clone(),
            args: args_bytes,
        };
        let body_bytes = encode_cbor(&operation)
            .map_err(|err| ForgeError(format!("encode operation: {err}")))?;

        let mut domain = self.domain.clone();
        let log_id = match placement {
            Some(log_id) => log_id,
            None => DomainLogId::SPACES_CONTROL,
        };
        let header = domain
            .append_operation_in_log(&self.private_key, operation, log_id)
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
