//! Store adapter satisfying the six trait bounds of the spaces `Manager`.
//!
//! Five traits delegate to the stock [`SqliteSpacesStore`]; only
//! [`SpacesMessageStore`] is jyn-specific because spaces messages travel
//! inside `DomainOperation::Spaces` bodies instead of header extensions.

use p2panda_auth::group::GroupCrdtState;
use p2panda_auth::traits::Conditions;
use p2panda_auth::traits::Operation as AuthOperation;
use p2panda_core::cbor::decode_cbor;
use p2panda_core::{Extensions, Hash, Operation, VerifyingKey};
use p2panda_encryption::key_manager::PreKeyBundlesState;
use p2panda_encryption::key_registry::KeyRegistryState;
use p2panda_spaces::SpacesArgs;
use p2panda_store::groups::GroupsStore;
use p2panda_store::key_registry::KeyRegistryStore;
use p2panda_store::key_secrets::KeySecretsStore;
use p2panda_store::operations::OperationStore;
use p2panda_store::spaces::{SpacesMessage, SpacesMessageStore, SpacesStore, SqliteSpacesStore};
use p2panda_store::{SqliteError, SqliteStore, Transaction};
use serde::{Deserialize, Serialize};

use crate::domain::{DomainExtensions, DomainOperation};

/// jyn's spaces store: p2panda's sqlite implementations for state, plus a
/// message lookup that unwraps `DomainOperation::Spaces` bodies.
#[derive(Clone)]
pub struct JynSpacesStore {
    inner: SqliteSpacesStore<DomainExtensions>,
    store: SqliteStore,
}

impl JynSpacesStore {
    pub fn new(store: SqliteStore) -> Self {
        Self {
            inner: SqliteSpacesStore::new(store.clone()),
            store,
        }
    }

    pub fn store(&self) -> &SqliteStore {
        &self.store
    }
}

/// Decodes the `SpacesArgs` carried inside a jyn operation body, if any.
pub fn spaces_args_from_operation<C>(
    operation: &Operation<DomainExtensions>,
) -> Option<SpacesArgs<C>>
where
    C: Conditions + for<'a> Deserialize<'a>,
{
    let body = operation.body.as_ref()?;
    let domain_operation = decode_cbor::<DomainOperation, _>(&body.to_bytes()[..]).ok()?;
    let DomainOperation::Spaces { args, .. } = domain_operation else {
        return None;
    };
    decode_cbor::<SpacesArgs<C>, _>(&args[..]).ok()
}

impl<ARG> SpacesMessageStore<ARG> for JynSpacesStore
where
    ARG: Clone + for<'a> Deserialize<'a>,
{
    type Error = SqliteError;

    async fn get_spaces_message(
        &self,
        id: &Hash,
    ) -> Result<Option<SpacesMessage<ARG>>, SqliteError> {
        let operation =
            <SqliteStore as OperationStore<Operation<DomainExtensions>, Hash>>::get_operation(
                &self.store,
                id,
            )
            .await?;
        let Some(operation) = operation else {
            return Ok(None);
        };
        let Some(body) = operation.body.as_ref() else {
            return Ok(None);
        };
        let Ok(DomainOperation::Spaces { args, .. }) =
            decode_cbor::<DomainOperation, _>(&body.to_bytes()[..])
        else {
            return Ok(None);
        };
        let Ok(args) = decode_cbor::<ARG, _>(&args[..]) else {
            return Ok(None);
        };
        Ok(Some(SpacesMessage {
            id: operation.hash,
            author: operation.header.verifying_key,
            args,
        }))
    }
}

impl<S> SpacesStore<S> for JynSpacesStore
where
    S: for<'a> Deserialize<'a> + Serialize + Sync,
{
    type Error = <SqliteSpacesStore<DomainExtensions> as SpacesStore<S>>::Error;

    async fn get_space_state_tx(&self, id: &Hash) -> Result<Option<S>, Self::Error> {
        self.inner.get_space_state_tx(id).await
    }

    async fn set_space_state_tx(&self, id: &Hash, y: &S) -> Result<(), Self::Error> {
        self.inner.set_space_state_tx(id, y).await
    }

    async fn has_space(&self, id: &Hash) -> Result<bool, Self::Error> {
        <SqliteSpacesStore<DomainExtensions> as SpacesStore<S>>::has_space(&self.inner, id).await
    }

    async fn space_ids(&self) -> Result<Vec<Hash>, Self::Error> {
        <SqliteSpacesStore<DomainExtensions> as SpacesStore<S>>::space_ids(&self.inner).await
    }
}

impl KeyRegistryStore for JynSpacesStore {
    type Error = <SqliteSpacesStore<DomainExtensions> as KeyRegistryStore>::Error;

    async fn get_key_registry(
        &self,
    ) -> Result<Option<KeyRegistryState<VerifyingKey>>, Self::Error> {
        self.inner.get_key_registry().await
    }

    async fn set_key_registry(
        &self,
        state: &KeyRegistryState<VerifyingKey>,
    ) -> Result<(), Self::Error> {
        self.inner.set_key_registry(state).await
    }
}

impl KeySecretsStore for JynSpacesStore {
    type Error = <SqliteSpacesStore<DomainExtensions> as KeySecretsStore>::Error;

    async fn get_prekey_secrets(&self) -> Result<Option<PreKeyBundlesState>, Self::Error> {
        self.inner.get_prekey_secrets().await
    }

    async fn set_prekey_secrets(&self, state: &PreKeyBundlesState) -> Result<(), Self::Error> {
        self.inner.set_prekey_secrets(state).await
    }
}

impl<M, C> GroupsStore<M, C> for JynSpacesStore
where
    M: AuthOperation<VerifyingKey, Hash, C> + Serialize + for<'a> Deserialize<'a> + Sync,
    C: Conditions + Serialize + for<'a> Deserialize<'a>,
{
    type Error = <SqliteSpacesStore<DomainExtensions> as GroupsStore<M, C>>::Error;

    async fn set_groups_state_tx(
        &self,
        id: Hash,
        state: &GroupCrdtState<VerifyingKey, Hash, M, C>,
    ) -> Result<(), Self::Error> {
        self.inner.set_groups_state_tx(id, state).await
    }

    async fn get_groups_state_tx(
        &self,
        id: Hash,
    ) -> Result<Option<GroupCrdtState<VerifyingKey, Hash, M, C>>, Self::Error> {
        self.inner.get_groups_state_tx(id).await
    }
}

impl Transaction for JynSpacesStore {
    type Error = <SqliteSpacesStore<DomainExtensions> as Transaction>::Error;
    type Permit = <SqliteSpacesStore<DomainExtensions> as Transaction>::Permit;

    async fn begin(&self) -> Result<Self::Permit, Self::Error> {
        self.inner.begin().await
    }

    async fn rollback(&self, permit: Self::Permit) -> Result<(), Self::Error> {
        self.inner.rollback(permit).await
    }

    async fn commit(&self, permit: Self::Permit) -> Result<(), Self::Error> {
        self.inner.commit(permit).await
    }
}

// Unused generic-context helper to keep the compiler honest about the
// Extensions bound the stock message store would need; jyn's operations do
// not embed `SpacesArgs` in extensions, which is why the custom
// `SpacesMessageStore` impl above exists at all.
#[allow(dead_code)]
fn _assert_extensions_bound<E: Extensions>() {}
