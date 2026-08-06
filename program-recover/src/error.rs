use {
    solana_pubkey::{ParsePubkeyError, Pubkey},
    solana_rpc_client_api::client_error::Error as RpcClientError,
    solana_signature::{ParseSignatureError, Signature},
    std::{io, path::PathBuf},
    thiserror::Error,
};

#[derive(Debug, Error)]
pub enum ProgramRecoverError {
    #[error(transparent)]
    RpcClient(#[from] RpcClientError),

    #[error("failed to decode base58 loader instruction data: {0}")]
    Base58Decode(#[from] bs58::decode::Error),

    #[error("failed to decode upgradeable-loader instruction: {0}")]
    LoaderInstructionDecode(#[from] bincode::Error),

    #[error("invalid signature `{value}` returned by RPC: {source}")]
    InvalidSignature {
        value: String,
        source: ParseSignatureError,
    },

    #[error("invalid pubkey `{value}` in transaction {signature}: {source}")]
    InvalidTransactionPubkey {
        signature: Signature,
        value: String,
        source: ParsePubkeyError,
    },

    #[error("transaction {signature} is unavailable from this RPC")]
    TransactionUnavailable { signature: Signature },

    #[error("transaction {signature} did not include metadata")]
    MissingTransactionMeta { signature: Signature },

    #[error("transaction {signature} was not returned with JSON encoding")]
    UnsupportedTransactionEncoding { signature: Signature },

    #[error("transaction {signature} was not returned with raw message encoding")]
    UnsupportedMessageEncoding { signature: Signature },

    #[error("transaction {signature} contains parsed inner instruction data")]
    UnsupportedInnerInstructionEncoding { signature: Signature },

    #[error(
        "transaction {signature} instruction account index {index} is out of range for \
         {key_count} keys"
    )]
    InvalidAccountIndex {
        signature: Signature,
        index: u8,
        key_count: usize,
    },

    #[error("transaction {signature} has inner instructions for missing top-level index {index}")]
    InvalidInnerInstructionIndex { signature: Signature, index: u8 },

    #[error("no signatures found for address {address}")]
    NoSignatures { address: Pubkey },

    #[error(
        "no deploy or upgrade events found for program {program} and ProgramData {programdata}"
    )]
    NoVersionEvents {
        program: Pubkey,
        programdata: Pubkey,
    },

    #[error(
        "no deploy or upgrade event was found before the ProgramData close for program {program}"
    )]
    NoVersionBeforeClose { program: Pubkey },

    #[error("no deploy or upgrade event found at slot {slot}")]
    VersionSlotNotFound { slot: u64 },

    #[error("slot {slot} has {count} deploy/upgrade events; select a unique slot")]
    AmbiguousVersionSlot { slot: u64, count: usize },

    #[error("event sequence counter overflowed")]
    EventSequenceOverflow,

    #[error("counter overflowed")]
    CounterOverflow,

    #[error("loader Write offset {offset} does not fit into usize")]
    WriteOffsetTooLarge { offset: u32 },

    #[error("loader Write offset {offset} plus length {length} overflowed")]
    WriteOffsetOverflow { offset: usize, length: usize },

    #[error("no Write instructions found for buffer {buffer}")]
    NoWriteInstructions { buffer: Pubkey },

    #[error("no Write chunks provided")]
    NoWriteChunks,

    #[error("selected deploy/upgrade transaction {signature} was not reached in buffer history")]
    SelectedVersionNotReached { signature: Signature },

    #[error("reconstructed image does not start with an ELF header")]
    MissingElfMagic,

    #[error("missing historical write data beginning at offset {first_gap}")]
    MissingWriteData { first_gap: usize },

    #[error("failed to write `{}`: {source}", path.display())]
    OutputWrite { path: PathBuf, source: io::Error },
}

impl ProgramRecoverError {
    pub fn invalid_signature(value: String, source: ParseSignatureError) -> ProgramRecoverError {
        ProgramRecoverError::InvalidSignature { value, source }
    }
}
