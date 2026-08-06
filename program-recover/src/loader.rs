use {
    crate::{
        error::ProgramRecoverError,
        reconstruct::WriteChunk,
        transaction::{ResolvedInstruction, ResolvedTransaction},
    },
    solana_loader_v3_interface::instruction::UpgradeableLoaderInstruction,
    solana_pubkey::Pubkey,
    solana_signature::Signature,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionKind {
    Deploy,
    Upgrade,
}

impl VersionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deploy => "deploy",
            Self::Upgrade => "upgrade",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionEvent {
    pub slot: u64,
    pub signature: Signature,
    pub buffer: Pubkey,
    pub kind: VersionKind,
    pub sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloseEvent {
    pub slot: u64,
    pub signature: Signature,
    pub sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoaderEvent {
    Version(VersionEvent),
    Close(CloseEvent),
}

pub fn decode_loader_instruction(
    data_base58: &str,
) -> Result<UpgradeableLoaderInstruction, ProgramRecoverError> {
    let raw = bs58::decode(data_base58).into_vec()?;
    let instruction = bincode::deserialize(&raw)?;
    Ok(instruction)
}

pub fn collect_loader_events(
    program: &Pubkey,
    programdata: &Pubkey,
    signature: &Signature,
    transaction: &ResolvedTransaction,
    next_sequence: &mut u64,
) -> Result<Vec<LoaderEvent>, ProgramRecoverError> {
    let mut events = Vec::new();
    for instruction in &transaction.instructions {
        let Some(event) = classify_version_or_close(
            program,
            programdata,
            signature,
            transaction.slot,
            instruction,
        )?
        else {
            continue;
        };
        events.push(with_sequence(event, next_sequence)?);
    }
    Ok(events)
}

pub fn extract_write_chunk(
    buffer: &Pubkey,
    instruction: &ResolvedInstruction,
) -> Result<Option<WriteChunk>, ProgramRecoverError> {
    if instruction.program_id != solana_sdk_ids::bpf_loader_upgradeable::id() {
        return Ok(None);
    }

    let decoded = decode_loader_instruction(&instruction.data)?;
    let UpgradeableLoaderInstruction::Write { offset, bytes } = decoded else {
        return Ok(None);
    };
    let Some(write_buffer) = instruction.accounts.first() else {
        return Ok(None);
    };
    if write_buffer != buffer {
        return Ok(None);
    }

    let offset = usize::try_from(offset)
        .map_err(|_err| ProgramRecoverError::WriteOffsetTooLarge { offset })?;

    Ok(Some(WriteChunk { offset, bytes }))
}

pub fn is_selected_consuming_instruction(
    program: &Pubkey,
    programdata: &Pubkey,
    selected: &VersionEvent,
    instruction: &ResolvedInstruction,
) -> Result<bool, ProgramRecoverError> {
    if instruction.program_id != solana_sdk_ids::bpf_loader_upgradeable::id() {
        return Ok(false);
    }

    let decoded = decode_loader_instruction(&instruction.data)?;
    let matches = match (selected.kind, decoded) {
        (
            VersionKind::Deploy,
            UpgradeableLoaderInstruction::DeployWithMaxDataLen { max_data_len: _ },
        ) => {
            account_at(&instruction.accounts, 1) == Some(programdata)
                && account_at(&instruction.accounts, 2) == Some(program)
                && account_at(&instruction.accounts, 3) == Some(&selected.buffer)
        }
        (VersionKind::Upgrade, UpgradeableLoaderInstruction::Upgrade) => {
            account_at(&instruction.accounts, 0) == Some(programdata)
                && account_at(&instruction.accounts, 1) == Some(program)
                && account_at(&instruction.accounts, 2) == Some(&selected.buffer)
        }
        _ => false,
    };

    Ok(matches)
}

fn classify_version_or_close(
    program: &Pubkey,
    programdata: &Pubkey,
    signature: &Signature,
    slot: u64,
    instruction: &ResolvedInstruction,
) -> Result<Option<LoaderEvent>, ProgramRecoverError> {
    if instruction.program_id != solana_sdk_ids::bpf_loader_upgradeable::id() {
        return Ok(None);
    }

    let decoded = decode_loader_instruction(&instruction.data)?;
    let event = match decoded {
        UpgradeableLoaderInstruction::DeployWithMaxDataLen { max_data_len: _ }
            if account_at(&instruction.accounts, 1) == Some(programdata)
                && account_at(&instruction.accounts, 2) == Some(program) =>
        {
            let Some(buffer) = account_at(&instruction.accounts, 3) else {
                return Ok(None);
            };
            Some(LoaderEvent::Version(VersionEvent {
                slot,
                signature: *signature,
                buffer: *buffer,
                kind: VersionKind::Deploy,
                sequence: 0,
            }))
        }
        UpgradeableLoaderInstruction::Upgrade
            if account_at(&instruction.accounts, 0) == Some(programdata)
                && account_at(&instruction.accounts, 1) == Some(program) =>
        {
            let Some(buffer) = account_at(&instruction.accounts, 2) else {
                return Ok(None);
            };
            Some(LoaderEvent::Version(VersionEvent {
                slot,
                signature: *signature,
                buffer: *buffer,
                kind: VersionKind::Upgrade,
                sequence: 0,
            }))
        }
        UpgradeableLoaderInstruction::Close
            if account_at(&instruction.accounts, 0) == Some(programdata)
                && account_at(&instruction.accounts, 3)
                    .is_none_or(|account| account == program) =>
        {
            Some(LoaderEvent::Close(CloseEvent {
                slot,
                signature: *signature,
                sequence: 0,
            }))
        }
        _ => None,
    };

    Ok(event)
}

fn with_sequence(
    event: LoaderEvent,
    next_sequence: &mut u64,
) -> Result<LoaderEvent, ProgramRecoverError> {
    let sequence = *next_sequence;
    *next_sequence = next_sequence
        .checked_add(1)
        .ok_or(ProgramRecoverError::EventSequenceOverflow)?;

    Ok(match event {
        LoaderEvent::Version(mut event) => {
            event.sequence = sequence;
            LoaderEvent::Version(event)
        }
        LoaderEvent::Close(mut event) => {
            event.sequence = sequence;
            LoaderEvent::Close(event)
        }
    })
}

fn account_at(accounts: &[Pubkey], index: usize) -> Option<&Pubkey> {
    accounts.get(index)
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::{cli::encode_loader_instruction, transaction::ResolvedInstruction},
    };

    fn key(value: u8) -> Pubkey {
        Pubkey::from([value; 32])
    }

    fn signature() -> Signature {
        Signature::from([2_u8; 64])
    }

    fn loader_instruction(
        accounts: Vec<Pubkey>,
        instruction: UpgradeableLoaderInstruction,
    ) -> ResolvedInstruction {
        ResolvedInstruction {
            program_id: solana_sdk_ids::bpf_loader_upgradeable::id(),
            accounts,
            data: encode_loader_instruction(&instruction),
        }
    }

    #[test]
    fn test_decodes_loader_write_instruction() {
        let data = encode_loader_instruction(&UpgradeableLoaderInstruction::Write {
            offset: 7,
            bytes: vec![1, 2, 3],
        });

        assert_eq!(
            decode_loader_instruction(&data).unwrap(),
            UpgradeableLoaderInstruction::Write {
                offset: 7,
                bytes: vec![1, 2, 3],
            }
        );
    }

    #[test]
    fn test_collects_deploy_upgrade_and_close_events() {
        let program = key(1);
        let programdata = key(2);
        let deploy_buffer = key(3);
        let upgrade_buffer = key(4);
        let transaction = ResolvedTransaction {
            slot: 100,
            instructions: vec![
                loader_instruction(
                    vec![
                        key(9),
                        programdata,
                        program,
                        deploy_buffer,
                        key(10),
                        key(11),
                        key(12),
                        key(13),
                    ],
                    UpgradeableLoaderInstruction::DeployWithMaxDataLen { max_data_len: 64 },
                ),
                loader_instruction(
                    vec![
                        programdata,
                        program,
                        upgrade_buffer,
                        key(14),
                        key(15),
                        key(16),
                        key(17),
                    ],
                    UpgradeableLoaderInstruction::Upgrade,
                ),
                loader_instruction(
                    vec![programdata, key(18), key(19), program],
                    UpgradeableLoaderInstruction::Close,
                ),
            ],
        };

        let mut sequence = 0;
        let events = collect_loader_events(
            &program,
            &programdata,
            &signature(),
            &transaction,
            &mut sequence,
        )
        .unwrap();

        assert_eq!(
            events,
            vec![
                LoaderEvent::Version(VersionEvent {
                    slot: 100,
                    signature: signature(),
                    buffer: deploy_buffer,
                    kind: VersionKind::Deploy,
                    sequence: 0,
                }),
                LoaderEvent::Version(VersionEvent {
                    slot: 100,
                    signature: signature(),
                    buffer: upgrade_buffer,
                    kind: VersionKind::Upgrade,
                    sequence: 1,
                }),
                LoaderEvent::Close(CloseEvent {
                    slot: 100,
                    signature: signature(),
                    sequence: 2,
                }),
            ]
        );
    }

    #[test]
    fn test_extracts_write_chunk_for_target_buffer() {
        let buffer = key(1);
        let instruction = loader_instruction(
            vec![buffer, key(2)],
            UpgradeableLoaderInstruction::Write {
                offset: 4,
                bytes: vec![127, 69],
            },
        );

        assert_eq!(
            extract_write_chunk(&buffer, &instruction).unwrap(),
            Some(WriteChunk {
                offset: 4,
                bytes: vec![127, 69],
            })
        );
    }
}
