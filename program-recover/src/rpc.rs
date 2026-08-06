use {
    crate::error::ProgramRecoverError,
    serde_json::json,
    solana_commitment_config::CommitmentConfig,
    solana_pubkey::Pubkey,
    solana_rpc_client::{
        nonblocking::rpc_client::RpcClient, rpc_client::GetConfirmedSignaturesForAddress2Config,
    },
    solana_rpc_client_api::{
        config::RpcTransactionConfig, request::RpcRequest,
        response::RpcConfirmedTransactionStatusWithSignature,
    },
    solana_signature::Signature,
    solana_transaction_status::{EncodedConfirmedTransactionWithStatusMeta, UiTransactionEncoding},
    std::str::FromStr,
};

const SIGNATURE_PAGE_LIMIT: usize = 1_000;

pub struct RpcHistoryClient {
    client: RpcClient,
}

impl RpcHistoryClient {
    pub fn new(json_rpc_url: String) -> Self {
        Self {
            client: RpcClient::new(json_rpc_url),
        }
    }

    pub async fn all_signatures(
        &self,
        address: &Pubkey,
    ) -> Result<Vec<RpcConfirmedTransactionStatusWithSignature>, ProgramRecoverError> {
        let mut result = Vec::new();
        let mut before = None;

        loop {
            let config = GetConfirmedSignaturesForAddress2Config {
                before,
                until: None,
                limit: Some(SIGNATURE_PAGE_LIMIT),
                commitment: Some(CommitmentConfig::finalized()),
            };
            let mut page = self
                .client
                .get_signatures_for_address_with_config(address, config)
                .await?;
            let page_len = page.len();

            let Some(last_entry) = page.last() else {
                break;
            };
            before = Some(parse_signature(&last_entry.signature)?);
            result.append(&mut page);

            if page_len < SIGNATURE_PAGE_LIMIT {
                break;
            }
        }

        result.reverse();
        Ok(result)
    }

    pub async fn get_transaction(
        &self,
        signature: &Signature,
    ) -> Result<Option<EncodedConfirmedTransactionWithStatusMeta>, ProgramRecoverError> {
        let config = RpcTransactionConfig {
            encoding: Some(UiTransactionEncoding::Json),
            commitment: Some(CommitmentConfig::finalized()),
            max_supported_transaction_version: Some(0),
        };
        let transaction = self
            .client
            .send(
                RpcRequest::GetTransaction,
                json!([signature.to_string(), config]),
            )
            .await?;

        Ok(transaction)
    }
}

pub fn parse_signature(signature: &str) -> Result<Signature, ProgramRecoverError> {
    Signature::from_str(signature)
        .map_err(|source| ProgramRecoverError::invalid_signature(signature.to_string(), source))
}
