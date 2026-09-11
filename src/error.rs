use thiserror::Error;

/// Messages deliberately exclude paths, metadata, keys and plaintext.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("METADATA_INVALID")]
    MetadataInvalid,
    #[error("METADATA_RECOVERY_REQUIRED")]
    MetadataRecoveryRequired,
    #[error("UNSUPPORTED_PROFILE")]
    UnsupportedProfile,
    #[error("REENCRYPTION_UNSUPPORTED")]
    ReencryptionUnsupported,
    #[error("RESOURCE_LIMIT")]
    ResourceLimit,
    #[error("UNLOCK_FAILED")]
    UnlockFailed,
    #[error("BACKEND_IO")]
    BackendIo,
    #[error("INVALID_RANGE")]
    InvalidRange,
    #[error("FS_TYPE_MISMATCH")]
    FsTypeMismatch,
    #[error("FS_INVALID")]
    FsInvalid,
    #[error("FS_RECOVERY_REQUIRED")]
    FsRecoveryRequired,
    #[error("FS_UNSUPPORTED_FEATURE")]
    FsUnsupportedFeature,
    #[error("FS_DRIVER_UNAVAILABLE")]
    FsDriverUnavailable,
    #[error("FS_GATE_UNPASSED")]
    FsGateUnpassed,
    #[error("WRITEBACK_FAILED")]
    WritebackFailed,
    #[error("UNSUPPORTED_OPERATION")]
    UnsupportedOperation,
    #[error("VOLUME_DISCOVERY_FAILED")]
    VolumeDiscoveryFailed,
    #[error("UNCLEAN_CLOSE")]
    UncleanClose,
    #[error("DEVICE_PUBLISH_FAILED")]
    DevicePublishFailed,
    #[error("READ_ONLY")]
    ReadOnly,
    #[error("STOPPING")]
    Stopping,
    #[error("CONSOLE_REQUIRED")]
    ConsoleRequired,
    #[error("CRYPTO_ERROR")]
    Crypto,
}
pub type Result<T> = std::result::Result<T, Error>;
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::BackendIo
    }
}
impl From<openssl::error::ErrorStack> for Error {
    fn from(_: openssl::error::ErrorStack) -> Self {
        Self::Crypto
    }
}
