use std::process::ExitCode;

fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args().skip(1).collect();

    if arguments == ["mcp"] {
        return match lattice_cli::serve_mcp_stdio() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("lattice MCP server failed: {error}");
                ExitCode::FAILURE
            }
        };
    }

    match lattice_cli::dispatch(&arguments) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(usage) => {
            eprintln!("{usage}");
            ExitCode::from(2)
        }
    }
}
