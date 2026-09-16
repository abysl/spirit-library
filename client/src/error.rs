use std::fmt;

/// Everything that can go wrong using a [`crate::Client`].
///
/// Spirit's own crates deliberately return `String`/`Box<dyn Error>` to stay
/// dependency-light; this is the boundary where a real, typed error is worth
/// having.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// A store operation failed: minting a record, storing a blob, editing
    /// a collection, and so on.
    #[error("store: {0}")]
    Store(String),

    /// A network/mesh operation failed (seeding a peer, pairing, joining).
    #[error("network: {0}")]
    Network(String),

    /// A network operation was attempted on a client that is neither
    /// [`crate::Mode::Embedded`] nor successfully [`crate::Mode::Attach`]ed —
    /// there is no live daemon to ask.
    #[error("no daemon is running for this store; open with Mode::Embedded or Mode::Attach")]
    NoDaemon,

    /// `Mode::Attach` found no daemon answering on the local socket for this
    /// store directory.
    #[error("no daemon answered the local socket for {0}")]
    NotRunning(std::path::PathBuf),

    /// Input that doesn't parse as what it claims to be (a hash, a seed, a
    /// pairing URL).
    #[error("invalid input: {0}")]
    Invalid(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// A blocking-thread task this crate spawned internally panicked.
    #[error("background task panicked: {0}")]
    Join(#[from] tokio::task::JoinError),
}

impl ClientError {
    pub(crate) fn store(error: impl fmt::Display) -> Self {
        Self::Store(error.to_string())
    }

    pub(crate) fn network(error: impl fmt::Display) -> Self {
        Self::Network(error.to_string())
    }
}
