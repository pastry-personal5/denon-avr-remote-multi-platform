//! The Control API server's executable: it composes the infrastructure, starts the
//! control service, and serves until it is told to stop.

use denon_avr_api_contract::EndpointPaths;
use denon_avr_api_server::{Limits, Server, ServerConfig, StartError};
use denon_avr_application::{AgentLimits, AgentPath, ControlService, ServiceConfig};
use denon_avr_infrastructure::{
    data_directory, FileTokenStore, JsonlAuditLog, SsdpDiscoveryAdapter, SystemClock,
    X3800hConnector, YamlConfigRepository, YamlPolicySource,
};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

/// The exit status for a server that finds another running (`EX_TEMPFAIL`).
const ALREADY_RUNNING: u8 = 75;

struct Arguments {
    data_directory: PathBuf,
}

fn arguments() -> Result<Arguments, String> {
    let mut data = None;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--data-dir" => {
                data = Some(PathBuf::from(
                    args.next().ok_or("--data-dir needs a directory")?,
                ));
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(Arguments {
        data_directory: data.unwrap_or_else(data_directory),
    })
}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let arguments = match arguments() {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("denon-avr-api-server: {error}");
            return ExitCode::from(2);
        }
    };
    let data = arguments.data_directory;
    let paths = EndpointPaths::under(&data);

    let tokens = match FileTokenStore::open(&paths.credentials_directory) {
        Ok(store) => Arc::new(store),
        Err(error) => {
            eprintln!("denon-avr-api-server: {error}");
            return ExitCode::from(1);
        }
    };
    if let Some(fault) = denon_avr_application::TokenStore::fault(&*tokens) {
        tracing::warn!(%fault, "serving without Agent tokens");
    }
    let service = Arc::new(
        ControlService::start(
            Arc::new(X3800hConnector::default()),
            Arc::new(YamlConfigRepository::new(
                data.join("denon-avr-remote.yaml"),
            )),
            Arc::new(SsdpDiscoveryAdapter),
            ServiceConfig::default(),
            AgentPath {
                policy: Arc::new(YamlPolicySource::new(data.join("policy.yaml"))),
                audit: Arc::new(JsonlAuditLog::new(data.join("audit"))),
                clock: Arc::new(SystemClock),
                limits: AgentLimits::default(),
                tokens: Some(tokens.clone()),
            },
        )
        .await,
    );
    let running = match Server::start(
        service,
        tokens,
        ServerConfig {
            paths,
            agent: None,
            limits: Limits::default(),
        },
    )
    .await
    {
        Ok(running) => running,
        Err(StartError::AlreadyRunning) => {
            eprintln!("denon-avr-api-server: another Control API server is already running");
            return ExitCode::from(ALREADY_RUNNING);
        }
        Err(error) => {
            eprintln!("denon-avr-api-server: {error}");
            return ExitCode::from(1);
        }
    };
    let _ = tokio::signal::ctrl_c().await;
    running.shutdown().await;
    ExitCode::SUCCESS
}
