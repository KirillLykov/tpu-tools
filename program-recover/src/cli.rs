use {
    clap::{Args, Parser, Subcommand, crate_description, crate_name, crate_version},
    solana_clap_v3_utils::{
        input_parsers::parse_url_or_moniker, input_validators::normalize_to_url_if_moniker,
    },
    solana_loader_v3_interface::instruction::UpgradeableLoaderInstruction,
    solana_pubkey::Pubkey,
    std::path::PathBuf,
};

#[derive(Parser, Debug, PartialEq, Eq)]
#[clap(name = crate_name!(),
    version = crate_version!(),
    about = crate_description!(),
    rename_all = "kebab-case"
)]
pub struct Cli {
    #[clap(
        long = "url",
        short = 'u',
        value_parser = parse_and_normalize_url,
        help = "URL for Solana JSON RPC or moniker: mainnet-beta, testnet, devnet, localhost."
    )]
    pub json_rpc_url: String,

    #[clap(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Command {
    #[clap(about = "Discover upgradeable-loader deploy, upgrade, and close events")]
    Discover(TargetParameters),

    #[clap(about = "Reconstruct a program ELF from historical buffer writes")]
    Dump(DumpParameters),
}

#[derive(Args, Clone, Debug, PartialEq, Eq)]
#[clap(rename_all = "kebab-case")]
pub struct TargetParameters {
    #[clap(long, help = "Upgradeable program address.")]
    pub program: Pubkey,

    #[clap(
        long,
        help = "ProgramData address. If omitted, it is derived from --program."
    )]
    pub programdata: Option<Pubkey>,
}

#[derive(Args, Clone, Debug, PartialEq, Eq)]
#[clap(rename_all = "kebab-case")]
pub struct DumpParameters {
    #[clap(flatten)]
    pub target: TargetParameters,

    #[clap(long, help = "Output path for the reconstructed ELF .so file.")]
    pub output: PathBuf,

    #[clap(
        long,
        help = "Recover the deploy/upgrade event at this slot instead of auto-selecting the \
                latest version before close."
    )]
    pub version_slot: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryTarget {
    pub program: Pubkey,
    pub programdata: Pubkey,
}

impl From<TargetParameters> for RecoveryTarget {
    fn from(parameters: TargetParameters) -> Self {
        let programdata = parameters.programdata.unwrap_or_else(|| {
            Pubkey::find_program_address(
                &[parameters.program.as_ref()],
                &solana_sdk_ids::bpf_loader_upgradeable::id(),
            )
            .0
        });

        Self {
            program: parameters.program,
            programdata,
        }
    }
}

pub fn build_cli_parameters() -> Cli {
    Cli::parse()
}

fn parse_and_normalize_url(addr: &str) -> Result<String, String> {
    match parse_url_or_moniker(addr) {
        Ok(parsed) => Ok(normalize_to_url_if_moniker(&parsed)),
        Err(err) => Err(format!("Invalid URL or moniker: {err}")),
    }
}

pub fn encode_loader_instruction(instruction: &UpgradeableLoaderInstruction) -> String {
    let data = bincode::serialize(instruction).expect("loader instruction should serialize");
    bs58::encode(data).into_string()
}

#[cfg(test)]
mod tests {
    use {super::*, clap::Parser};

    fn program() -> Pubkey {
        Pubkey::from([1_u8; 32])
    }

    fn programdata() -> Pubkey {
        Pubkey::from([2_u8; 32])
    }

    #[test]
    fn test_discover_command() {
        let actual = Cli::try_parse_from([
            "test",
            "--url",
            "localhost",
            "discover",
            "--program",
            &program().to_string(),
            "--programdata",
            &programdata().to_string(),
        ])
        .unwrap();

        assert_eq!(
            actual,
            Cli {
                json_rpc_url: "http://localhost:8899".to_string(),
                command: Command::Discover(TargetParameters {
                    program: program(),
                    programdata: Some(programdata()),
                }),
            }
        );
    }

    #[test]
    fn test_dump_command() {
        let actual = Cli::try_parse_from([
            "test",
            "--url",
            "http://127.0.0.1:8899",
            "dump",
            "--program",
            &program().to_string(),
            "--output",
            "recovered.so",
            "--version-slot",
            "42",
        ])
        .unwrap();

        assert_eq!(
            actual,
            Cli {
                json_rpc_url: "http://127.0.0.1:8899".to_string(),
                command: Command::Dump(DumpParameters {
                    target: TargetParameters {
                        program: program(),
                        programdata: None,
                    },
                    output: PathBuf::from("recovered.so"),
                    version_slot: Some(42),
                }),
            }
        );
    }
}
