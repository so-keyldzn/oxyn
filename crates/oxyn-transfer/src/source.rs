//! Pure transfer source facts, captured by the executor from the original live owner.
use oxyn_core::{
    ConnectionConfig, ConnectionId, DriverId, NativeTransferFormat, NativeTransferSupport,
    SessionId,
};

/// The boundary receives no credential reference or secret resolver.
#[derive(Clone)]
pub struct TransferSource {
    /// Original configured connection.
    pub connection: ConnectionId,
    /// Original live owner.
    pub session: SessionId,
    /// Driver protocol; not proof of a compatible native product.
    pub driver: DriverId,
    /// Actual product from the session's catalog, not a webview claim.
    pub product: String,
    /// Actual version, validated by the adapter before invocation.
    pub version: String,
    /// Human label as reviewed; never part of an invocation.
    pub name: String,
    /// Effective source read-only protection, including retained policy.
    pub read_only: bool,
    /// Non-secret destination parameters, deliberately never Debug/serialized.
    pub(crate) params: std::collections::BTreeMap<String, String>,
}
impl TransferSource {
    /// Capture only non-secret destination settings; never read a keychain.
    #[must_use]
    pub fn new(
        config: &ConnectionConfig,
        session: SessionId,
        product: String,
        version: String,
        read_only: bool,
    ) -> Self {
        Self {
            connection: config.id,
            session,
            driver: config.driver.clone(),
            product,
            version,
            name: config.name.clone(),
            read_only: read_only || config.read_only,
            params: config
                .params
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        }
    }
    /// Capability descriptors keep the same UI usable for unsupported engines.
    #[must_use]
    pub fn support(&self) -> NativeTransferSupport {
        let available = crate::postgres::Target::from_source(self);
        let reason = available.as_ref().err().map(ToString::to_string);
        let supported = available.is_ok();
        NativeTransferSupport {
            engine: self.driver.as_str().to_owned(),
            export: supported,
            restore: supported && !self.read_only,
            reason: reason.or_else(|| {
                self.read_only.then(|| {
                    "This connection is marked read-only; database restoration is refused".into()
                })
            }),
            export_formats: if supported {
                vec![
                    NativeTransferFormat::CustomArchive,
                    NativeTransferFormat::Sql,
                ]
            } else {
                vec![]
            },
            restore_formats: if supported && !self.read_only {
                vec![NativeTransferFormat::CustomArchive]
            } else {
                vec![]
            },
        }
    }
}

impl std::fmt::Debug for TransferSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransferSource")
            .field("connection", &self.connection)
            .field("session", &self.session)
            .field("driver", &self.driver)
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}
