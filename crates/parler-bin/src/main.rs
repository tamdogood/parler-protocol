//! The `parler` umbrella binary — dispatches to the CLI subcommands.

#[tokio::main]
async fn main() {
    if let Err(e) = parler_cli::run().await {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
