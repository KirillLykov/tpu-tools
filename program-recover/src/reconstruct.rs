use crate::error::ProgramRecoverError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteChunk {
    pub offset: usize,
    pub bytes: Vec<u8>,
}

pub fn reconstruct(chunks: &[WriteChunk]) -> Result<Vec<u8>, ProgramRecoverError> {
    let final_len = chunks
        .iter()
        .map(chunk_end)
        .collect::<Result<Vec<usize>, ProgramRecoverError>>()?
        .into_iter()
        .max()
        .ok_or(ProgramRecoverError::NoWriteChunks)?;

    let mut image = vec![0_u8; final_len];
    let mut written = vec![false; final_len];

    for chunk in chunks {
        let end = chunk_end(chunk)?;
        image[chunk.offset..end].copy_from_slice(&chunk.bytes);
        written[chunk.offset..end].fill(true);
    }

    if !image.starts_with(b"\x7fELF") {
        return Err(ProgramRecoverError::MissingElfMagic);
    }

    if let Some(first_gap) = written.iter().position(|written| !*written) {
        return Err(ProgramRecoverError::MissingWriteData { first_gap });
    }

    Ok(image)
}

fn chunk_end(chunk: &WriteChunk) -> Result<usize, ProgramRecoverError> {
    chunk
        .offset
        .checked_add(chunk.bytes.len())
        .ok_or(ProgramRecoverError::WriteOffsetOverflow {
            offset: chunk.offset,
            length: chunk.bytes.len(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconstructs_chunks_in_order() {
        let image = reconstruct(&[
            WriteChunk {
                offset: 0,
                bytes: b"\x7fELFaa".to_vec(),
            },
            WriteChunk {
                offset: 4,
                bytes: b"bb".to_vec(),
            },
        ])
        .unwrap();

        assert_eq!(image, b"\x7fELFbb".to_vec());
    }

    #[test]
    fn test_later_chunks_overwrite_earlier_chunks() {
        let image = reconstruct(&[
            WriteChunk {
                offset: 0,
                bytes: b"\x7fELFold".to_vec(),
            },
            WriteChunk {
                offset: 4,
                bytes: b"new".to_vec(),
            },
        ])
        .unwrap();

        assert_eq!(image, b"\x7fELFnew".to_vec());
    }

    #[test]
    fn test_rejects_missing_elf_magic() {
        assert!(matches!(
            reconstruct(&[WriteChunk {
                offset: 0,
                bytes: b"not-elf".to_vec(),
            }]),
            Err(ProgramRecoverError::MissingElfMagic)
        ));
    }

    #[test]
    fn test_rejects_gaps() {
        assert!(matches!(
            reconstruct(&[
                WriteChunk {
                    offset: 0,
                    bytes: b"\x7fELF".to_vec(),
                },
                WriteChunk {
                    offset: 5,
                    bytes: b"x".to_vec(),
                },
            ]),
            Err(ProgramRecoverError::MissingWriteData { first_gap: 4 })
        ));
    }
}
