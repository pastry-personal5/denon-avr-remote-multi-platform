//! Short-lived composition over the in-process control service.
//!
//! The CLI reaches the receiver only through the control-service port. It
//! composes the service, so it still links infrastructure for now; it
//! constructs no session of its own and allocates no operation ids.

mod args;
mod commands;
mod render;
mod target;
#[cfg(test)]
mod tests;

use args::{parse_args, print_usage, Command};
use commands::run;
use denon_avr_application::{ControlService, ServiceConfig};
use denon_avr_infrastructure::{SsdpDiscoveryAdapter, X3800hConnector, YamlConfigRepository};
use std::sync::Arc;

const VERSION: &str = env!("CARGO_PKG_VERSION");

async fn execute(command: Command) -> Result<(), String> {
    match command {
        Command::Help => print_usage(),
        Command::Version => println!("denon-avr-remote {VERSION}"),
        command => {
            let service = ControlService::new(
                Arc::new(X3800hConnector::default()),
                Arc::new(YamlConfigRepository::default()),
                Arc::new(SsdpDiscoveryAdapter),
                ServiceConfig::default(),
            );
            let operator = service.operator();
            let result = run(&*operator, command).await;
            // Closes the receiver connection, which frees the receiver's single
            // control connection for other tools.
            service.shutdown().await;
            let report = result?;
            print!("{}", report.text);
            for warning in &report.warnings {
                eprintln!("warning: {warning}");
            }
        }
    }
    Ok(())
}
fn main() {
    let result = parse_args(std::env::args().skip(1)).and_then(|command| {
        tokio::runtime::Runtime::new()
            .map_err(|error| error.to_string())?
            .block_on(execute(command))
    });
    if let Err(error) = result {
        eprintln!("error: {error}");
        print_usage();
        std::process::exit(2);
    }
}
