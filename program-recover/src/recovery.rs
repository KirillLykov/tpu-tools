use {
    crate::{
        cli::{DumpParameters, RecoveryTarget},
        error::ProgramRecoverError,
        loader::{
            CloseEvent, LoaderEvent, VersionEvent, collect_loader_events, extract_write_chunk,
            is_selected_consuming_instruction,
        },
        reconstruct::reconstruct,
        rpc::{RpcHistoryClient, parse_signature},
        transaction::resolve_successful_transaction,
    },
    std::{fs, path::PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramHistory {
    pub target: RecoveryTarget,
    pub versions: Vec<VersionEvent>,
    pub closes: Vec<CloseEvent>,
    pub unavailable_transactions: usize,
    pub skipped_failed_transactions: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DumpSummary {
    pub selected_version: VersionEvent,
    pub output: PathBuf,
    pub bytes_written: usize,
    pub chunk_count: usize,
    pub unavailable_programdata_transactions: usize,
    pub unavailable_buffer_transactions: usize,
    pub skipped_failed_transactions: usize,
}

pub async fn recover_program_history(
    rpc_client: &RpcHistoryClient,
    target: RecoveryTarget,
) -> Result<ProgramHistory, ProgramRecoverError> {
    let signatures = rpc_client.all_signatures(&target.programdata).await?;
    if signatures.is_empty() {
        return Err(ProgramRecoverError::NoSignatures {
            address: target.programdata,
        });
    }

    let mut history = ProgramHistory {
        target,
        versions: Vec::new(),
        closes: Vec::new(),
        unavailable_transactions: 0,
        skipped_failed_transactions: 0,
    };
    let mut next_sequence = 0;

    for signature_status in signatures {
        let signature = parse_signature(&signature_status.signature)?;
        if signature_status.err.is_some() {
            increment_counter(&mut history.skipped_failed_transactions)?;
            continue;
        }

        let Some(transaction) = rpc_client.get_transaction(&signature).await? else {
            increment_counter(&mut history.unavailable_transactions)?;
            continue;
        };
        let Some(resolved_transaction) = resolve_successful_transaction(&signature, &transaction)?
        else {
            increment_counter(&mut history.skipped_failed_transactions)?;
            continue;
        };

        for event in collect_loader_events(
            &history.target.program,
            &history.target.programdata,
            &signature,
            &resolved_transaction,
            &mut next_sequence,
        )? {
            match event {
                LoaderEvent::Version(event) => history.versions.push(event),
                LoaderEvent::Close(event) => history.closes.push(event),
            }
        }
    }

    Ok(history)
}

pub async fn dump_program(
    rpc_client: &RpcHistoryClient,
    parameters: DumpParameters,
) -> Result<DumpSummary, ProgramRecoverError> {
    let target: RecoveryTarget = parameters.target.into();
    let history = recover_program_history(rpc_client, target.clone()).await?;
    let selected_version = select_version(&history, parameters.version_slot)?;
    let recovery = recover_buffer(rpc_client, &target, &selected_version).await?;

    fs::write(&parameters.output, &recovery.image).map_err(|source| {
        ProgramRecoverError::OutputWrite {
            path: parameters.output.clone(),
            source,
        }
    })?;

    Ok(DumpSummary {
        selected_version,
        output: parameters.output,
        bytes_written: recovery.image.len(),
        chunk_count: recovery.chunk_count,
        unavailable_programdata_transactions: history.unavailable_transactions,
        unavailable_buffer_transactions: recovery.unavailable_transactions,
        skipped_failed_transactions: history
            .skipped_failed_transactions
            .checked_add(recovery.skipped_failed_transactions)
            .ok_or(ProgramRecoverError::CounterOverflow)?,
    })
}

pub fn select_version(
    history: &ProgramHistory,
    version_slot: Option<u64>,
) -> Result<VersionEvent, ProgramRecoverError> {
    if let Some(slot) = version_slot {
        let matches: Vec<&VersionEvent> = history
            .versions
            .iter()
            .filter(|version| version.slot == slot)
            .collect();
        return match matches.as_slice() {
            [] => Err(ProgramRecoverError::VersionSlotNotFound { slot }),
            [event] => Ok((*event).clone()),
            _ => Err(ProgramRecoverError::AmbiguousVersionSlot {
                slot,
                count: matches.len(),
            }),
        };
    }

    if history.versions.is_empty() {
        return Err(ProgramRecoverError::NoVersionEvents {
            program: history.target.program,
            programdata: history.target.programdata,
        });
    }

    let close_sequence = history.closes.first().map(|event| event.sequence);
    let selected = history
        .versions
        .iter()
        .rfind(|version| close_sequence.is_none_or(|sequence| version.sequence < sequence));

    selected
        .cloned()
        .ok_or(ProgramRecoverError::NoVersionBeforeClose {
            program: history.target.program,
        })
}

pub fn print_discovery_summary(history: &ProgramHistory) {
    println!("Program: {}", history.target.program);
    println!("ProgramData: {}", history.target.programdata);
    println!();
    println!("slot       kind       buffer                                      signature");
    println!("---------  ---------  --------------------------------------------  ---------");
    for version in &history.versions {
        println!(
            "{:<9}  {:<9}  {}  {}",
            version.slot,
            version.kind.as_str(),
            version.buffer,
            version.signature,
        );
    }
    for close in &history.closes {
        println!(
            "{:<9}  {:<9}  {:<44}  {}",
            close.slot, "close", "", close.signature
        );
    }
    if history.unavailable_transactions != 0 {
        println!();
        println!(
            "unavailable ProgramData transactions: {}",
            history.unavailable_transactions
        );
    }
    if history.skipped_failed_transactions != 0 {
        println!(
            "skipped failed ProgramData transactions: {}",
            history.skipped_failed_transactions
        );
    }
}

pub fn print_dump_summary(summary: &DumpSummary) {
    println!("recovered: {}", summary.output.display());
    println!("selected slot: {}", summary.selected_version.slot);
    println!("selected signature: {}", summary.selected_version.signature);
    println!("selected kind: {}", summary.selected_version.kind.as_str());
    println!("buffer: {}", summary.selected_version.buffer);
    println!("chunks: {}", summary.chunk_count);
    println!("bytes: {}", summary.bytes_written);
    if summary.unavailable_programdata_transactions != 0 {
        println!(
            "unavailable ProgramData transactions: {}",
            summary.unavailable_programdata_transactions
        );
    }
    if summary.unavailable_buffer_transactions != 0 {
        println!(
            "unavailable buffer transactions: {}",
            summary.unavailable_buffer_transactions
        );
    }
    if summary.skipped_failed_transactions != 0 {
        println!(
            "skipped failed transactions: {}",
            summary.skipped_failed_transactions
        );
    }
}

struct BufferRecovery {
    image: Vec<u8>,
    chunk_count: usize,
    unavailable_transactions: usize,
    skipped_failed_transactions: usize,
}

async fn recover_buffer(
    rpc_client: &RpcHistoryClient,
    target: &RecoveryTarget,
    selected_version: &VersionEvent,
) -> Result<BufferRecovery, ProgramRecoverError> {
    let signatures = rpc_client.all_signatures(&selected_version.buffer).await?;
    if signatures.is_empty() {
        return Err(ProgramRecoverError::NoSignatures {
            address: selected_version.buffer,
        });
    }

    let mut chunks = Vec::new();
    let mut unavailable_transactions = 0;
    let mut skipped_failed_transactions = 0;
    let mut selected_version_reached = false;

    for signature_status in signatures {
        let signature = parse_signature(&signature_status.signature)?;
        if signature_status.err.is_some() {
            increment_counter(&mut skipped_failed_transactions)?;
            continue;
        }

        let Some(transaction) = rpc_client.get_transaction(&signature).await? else {
            if signature == selected_version.signature {
                return Err(ProgramRecoverError::TransactionUnavailable { signature });
            }
            increment_counter(&mut unavailable_transactions)?;
            continue;
        };
        let Some(resolved_transaction) = resolve_successful_transaction(&signature, &transaction)?
        else {
            increment_counter(&mut skipped_failed_transactions)?;
            continue;
        };

        for instruction in &resolved_transaction.instructions {
            if let Some(chunk) = extract_write_chunk(&selected_version.buffer, instruction)? {
                chunks.push(chunk);
            }

            if signature == selected_version.signature
                && is_selected_consuming_instruction(
                    &target.program,
                    &target.programdata,
                    selected_version,
                    instruction,
                )?
            {
                selected_version_reached = true;
                break;
            }
        }

        if selected_version_reached {
            break;
        }
    }

    if !selected_version_reached {
        return Err(ProgramRecoverError::SelectedVersionNotReached {
            signature: selected_version.signature,
        });
    }
    if chunks.is_empty() {
        return Err(ProgramRecoverError::NoWriteInstructions {
            buffer: selected_version.buffer,
        });
    }

    let chunk_count = chunks.len();
    let image = reconstruct(&chunks)?;

    Ok(BufferRecovery {
        image,
        chunk_count,
        unavailable_transactions,
        skipped_failed_transactions,
    })
}

fn increment_counter(counter: &mut usize) -> Result<(), ProgramRecoverError> {
    *counter = counter
        .checked_add(1)
        .ok_or(ProgramRecoverError::CounterOverflow)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        crate::loader::{CloseEvent, VersionKind},
        solana_pubkey::Pubkey,
        solana_signature::Signature,
    };

    fn key(value: u8) -> Pubkey {
        Pubkey::from([value; 32])
    }

    fn signature(value: u8) -> Signature {
        Signature::from([value; 64])
    }

    fn target() -> RecoveryTarget {
        RecoveryTarget {
            program: key(1),
            programdata: key(2),
        }
    }

    fn version(slot: u64, sequence: u64) -> VersionEvent {
        VersionEvent {
            slot,
            signature: signature(3),
            buffer: key(4),
            kind: VersionKind::Upgrade,
            sequence,
        }
    }

    #[test]
    fn test_selects_latest_version_before_close() {
        let history = ProgramHistory {
            target: target(),
            versions: vec![version(10, 0), version(20, 1), version(40, 3)],
            closes: vec![CloseEvent {
                slot: 30,
                signature: signature(5),
                sequence: 2,
            }],
            unavailable_transactions: 0,
            skipped_failed_transactions: 0,
        };

        assert_eq!(select_version(&history, None).unwrap(), version(20, 1));
    }

    #[test]
    fn test_selects_explicit_slot() {
        let history = ProgramHistory {
            target: target(),
            versions: vec![version(10, 0), version(20, 1)],
            closes: vec![],
            unavailable_transactions: 0,
            skipped_failed_transactions: 0,
        };

        assert_eq!(select_version(&history, Some(10)).unwrap(), version(10, 0));
    }
}
