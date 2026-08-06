use {
    log::error,
    solana_program_recover::{
        cli::{Cli, Command, build_cli_parameters},
        error::ProgramRecoverError,
        recovery::{
            dump_program, print_discovery_summary, print_dump_summary, recover_program_history,
        },
        rpc::RpcHistoryClient,
    },
};

fn main() {
    agave_logger::setup_with_default("solana=info");

    let parameters = build_cli_parameters();
    let code = {
        if let Err(err) = run(parameters) {
            error!("ERROR: {err}");
            eprintln!("error: {err}");
            1
        } else {
            0
        }
    };
    ::std::process::exit(code);
}

#[tokio::main]
async fn run(parameters: Cli) -> Result<(), ProgramRecoverError> {
    let rpc_client = RpcHistoryClient::new(parameters.json_rpc_url);

    match parameters.command {
        Command::Discover(target) => {
            let history = recover_program_history(&rpc_client, target.into()).await?;
            print_discovery_summary(&history);
        }
        Command::Dump(parameters) => {
            let summary = dump_program(&rpc_client, parameters).await?;
            print_dump_summary(&summary);
        }
    }

    Ok(())
}
