use {
    crate::error::ProgramRecoverError,
    solana_pubkey::Pubkey,
    solana_signature::Signature,
    solana_transaction_status::{
        EncodedConfirmedTransactionWithStatusMeta, EncodedTransaction, UiCompiledInstruction,
        UiInstruction, UiLoadedAddresses, UiMessage, UiRawMessage, UiTransactionStatusMeta,
        option_serializer::OptionSerializer,
    },
    std::{collections::BTreeMap, str::FromStr},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedInstruction {
    pub program_id: Pubkey,
    pub accounts: Vec<Pubkey>,
    pub data: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTransaction {
    pub slot: u64,
    pub instructions: Vec<ResolvedInstruction>,
}

pub fn resolve_successful_transaction(
    signature: &Signature,
    transaction: &EncodedConfirmedTransactionWithStatusMeta,
) -> Result<Option<ResolvedTransaction>, ProgramRecoverError> {
    let Some(meta) = transaction.transaction.meta.as_ref() else {
        return Err(ProgramRecoverError::MissingTransactionMeta {
            signature: *signature,
        });
    };

    if meta.err.is_some() {
        return Ok(None);
    }

    let EncodedTransaction::Json(ui_transaction) = &transaction.transaction.transaction else {
        return Err(ProgramRecoverError::UnsupportedTransactionEncoding {
            signature: *signature,
        });
    };
    let UiMessage::Raw(message) = &ui_transaction.message else {
        return Err(ProgramRecoverError::UnsupportedMessageEncoding {
            signature: *signature,
        });
    };

    let keys = combined_account_keys(signature, message, meta)?;
    let instructions = resolve_instructions(signature, message, meta, &keys)?;

    Ok(Some(ResolvedTransaction {
        slot: transaction.slot,
        instructions,
    }))
}

fn combined_account_keys(
    signature: &Signature,
    message: &UiRawMessage,
    meta: &UiTransactionStatusMeta,
) -> Result<Vec<Pubkey>, ProgramRecoverError> {
    let mut keys = Vec::with_capacity(message.account_keys.len());
    for key in &message.account_keys {
        keys.push(parse_pubkey(signature, key)?);
    }

    if let OptionSerializer::Some(loaded_addresses) = meta.loaded_addresses.as_ref() {
        extend_loaded_addresses(signature, &mut keys, loaded_addresses)?;
    }

    Ok(keys)
}

fn extend_loaded_addresses(
    signature: &Signature,
    keys: &mut Vec<Pubkey>,
    loaded_addresses: &UiLoadedAddresses,
) -> Result<(), ProgramRecoverError> {
    for key in &loaded_addresses.writable {
        keys.push(parse_pubkey(signature, key)?);
    }
    for key in &loaded_addresses.readonly {
        keys.push(parse_pubkey(signature, key)?);
    }
    Ok(())
}

fn parse_pubkey(signature: &Signature, value: &str) -> Result<Pubkey, ProgramRecoverError> {
    Pubkey::from_str(value).map_err(|source| ProgramRecoverError::InvalidTransactionPubkey {
        signature: *signature,
        value: value.to_string(),
        source,
    })
}

fn resolve_instructions(
    signature: &Signature,
    message: &UiRawMessage,
    meta: &UiTransactionStatusMeta,
    keys: &[Pubkey],
) -> Result<Vec<ResolvedInstruction>, ProgramRecoverError> {
    let mut inner_by_index = BTreeMap::new();
    if let OptionSerializer::Some(inner_instruction_groups) = meta.inner_instructions.as_ref() {
        for inner_instruction_group in inner_instruction_groups {
            inner_by_index
                .entry(inner_instruction_group.index)
                .or_insert_with(Vec::new)
                .extend(inner_instruction_group.instructions.iter());
        }
    }

    let mut resolved = Vec::new();
    for (top_level_index, instruction) in message.instructions.iter().enumerate() {
        let top_level_index = u8::try_from(top_level_index).map_err(|_err| {
            ProgramRecoverError::InvalidInnerInstructionIndex {
                signature: *signature,
                index: u8::MAX,
            }
        })?;
        resolved.push(resolve_compiled_instruction(signature, keys, instruction)?);

        if let Some(inner_instructions) = inner_by_index.remove(&top_level_index) {
            for inner_instruction in inner_instructions {
                resolved.push(resolve_ui_instruction(signature, keys, inner_instruction)?);
            }
        }
    }

    let Some((&invalid_index, _instructions)) = inner_by_index.first_key_value() else {
        return Ok(resolved);
    };
    Err(ProgramRecoverError::InvalidInnerInstructionIndex {
        signature: *signature,
        index: invalid_index,
    })
}

fn resolve_ui_instruction(
    signature: &Signature,
    keys: &[Pubkey],
    instruction: &UiInstruction,
) -> Result<ResolvedInstruction, ProgramRecoverError> {
    let UiInstruction::Compiled(instruction) = instruction else {
        return Err(ProgramRecoverError::UnsupportedInnerInstructionEncoding {
            signature: *signature,
        });
    };
    resolve_compiled_instruction(signature, keys, instruction)
}

fn resolve_compiled_instruction(
    signature: &Signature,
    keys: &[Pubkey],
    instruction: &UiCompiledInstruction,
) -> Result<ResolvedInstruction, ProgramRecoverError> {
    let program_id = resolve_key(signature, keys, instruction.program_id_index)?;
    let mut accounts = Vec::with_capacity(instruction.accounts.len());
    for account_index in &instruction.accounts {
        accounts.push(resolve_key(signature, keys, *account_index)?);
    }

    Ok(ResolvedInstruction {
        program_id,
        accounts,
        data: instruction.data.clone(),
    })
}

fn resolve_key(
    signature: &Signature,
    keys: &[Pubkey],
    index: u8,
) -> Result<Pubkey, ProgramRecoverError> {
    keys.get(usize::from(index))
        .copied()
        .ok_or(ProgramRecoverError::InvalidAccountIndex {
            signature: *signature,
            index,
            key_count: keys.len(),
        })
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        solana_message_v3::MessageHeader,
        solana_transaction_status::{
            EncodedTransactionWithStatusMeta, UiInnerInstructions, UiLoadedAddresses, UiRawMessage,
            UiTransaction,
        },
    };

    fn signature() -> Signature {
        Signature::from([1_u8; 64])
    }

    fn key(value: u8) -> Pubkey {
        Pubkey::from([value; 32])
    }

    fn transaction_with(
        message: UiRawMessage,
        meta: UiTransactionStatusMeta,
    ) -> EncodedConfirmedTransactionWithStatusMeta {
        EncodedConfirmedTransactionWithStatusMeta {
            slot: 42,
            transaction: EncodedTransactionWithStatusMeta {
                transaction: EncodedTransaction::Json(UiTransaction {
                    signatures: vec![signature().to_string()],
                    message: UiMessage::Raw(message),
                }),
                meta: Some(meta),
                version: None,
            },
            block_time: None,
            transaction_index: None,
        }
    }

    fn raw_message(instructions: Vec<UiCompiledInstruction>) -> UiRawMessage {
        UiRawMessage {
            header: MessageHeader {
                num_required_signatures: 0,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 0,
            },
            account_keys: vec![key(1).to_string(), key(2).to_string()],
            recent_blockhash: key(3).to_string(),
            instructions,
            address_table_lookups: None,
        }
    }

    fn meta_with(
        loaded_addresses: OptionSerializer<UiLoadedAddresses>,
        inner_instructions: OptionSerializer<Vec<UiInnerInstructions>>,
    ) -> UiTransactionStatusMeta {
        UiTransactionStatusMeta {
            err: None,
            status: Ok(()),
            fee: 0,
            pre_balances: vec![],
            post_balances: vec![],
            inner_instructions,
            log_messages: OptionSerializer::None,
            pre_token_balances: OptionSerializer::None,
            post_token_balances: OptionSerializer::None,
            rewards: OptionSerializer::None,
            loaded_addresses,
            return_data: OptionSerializer::Skip,
            compute_units_consumed: OptionSerializer::Skip,
            cost_units: OptionSerializer::Skip,
        }
    }

    #[test]
    fn test_resolves_loaded_address_indexes() {
        let transaction = transaction_with(
            raw_message(vec![UiCompiledInstruction {
                program_id_index: 2,
                accounts: vec![0, 3],
                data: "abc".to_string(),
                stack_height: None,
            }]),
            meta_with(
                OptionSerializer::Some(UiLoadedAddresses {
                    writable: vec![key(4).to_string()],
                    readonly: vec![key(5).to_string()],
                }),
                OptionSerializer::None,
            ),
        );

        let resolved = resolve_successful_transaction(&signature(), &transaction)
            .unwrap()
            .unwrap();

        assert_eq!(
            resolved.instructions,
            vec![ResolvedInstruction {
                program_id: key(4),
                accounts: vec![key(1), key(5)],
                data: "abc".to_string(),
            }]
        );
    }

    #[test]
    fn test_interleaves_top_level_and_inner_instructions() {
        let transaction = transaction_with(
            raw_message(vec![
                UiCompiledInstruction {
                    program_id_index: 0,
                    accounts: vec![],
                    data: "top0".to_string(),
                    stack_height: None,
                },
                UiCompiledInstruction {
                    program_id_index: 1,
                    accounts: vec![],
                    data: "top1".to_string(),
                    stack_height: None,
                },
            ]),
            meta_with(
                OptionSerializer::None,
                OptionSerializer::Some(vec![UiInnerInstructions {
                    index: 0,
                    instructions: vec![UiInstruction::Compiled(UiCompiledInstruction {
                        program_id_index: 1,
                        accounts: vec![],
                        data: "inner0".to_string(),
                        stack_height: Some(2),
                    })],
                }]),
            ),
        );

        let resolved = resolve_successful_transaction(&signature(), &transaction)
            .unwrap()
            .unwrap();

        let data: Vec<&str> = resolved
            .instructions
            .iter()
            .map(|instruction| instruction.data.as_str())
            .collect();
        assert_eq!(data, vec!["top0", "inner0", "top1"]);
    }
}
