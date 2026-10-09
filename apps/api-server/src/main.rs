//! The Control API server's executable: it composes the infrastructure, starts the
//! control service, and serves until it is told to stop.

use denon_avr_api_contract::EndpointPaths;
use denon_avr_api_server::settings::{self, SETTINGS_FILE};
use denon_avr_api_server::shutdown::{parent_gone, Reason, Signals};
use denon_avr_api_server::{InstanceLock, Limits, Server, ServerConfig, StartError};
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
    /// Stop when standard input ends, because the process that started the server
    /// held its other end.
    exit_with_parent: bool,
}

fn arguments() -> Result<Arguments, String> {
    let mut data = None;
    let mut exit_with_parent = false;
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--data-dir" => {
                data = Some(PathBuf::from(
                    args.next().ok_or("--data-dir needs a directory")?,
                ));
            }
            "--exit-with-parent" => exit_with_parent = true,
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    Ok(Arguments {
        data_directory: data.unwrap_or_else(data_directory),
        exit_with_parent,
    })
}

/// Say why the server did not start, and the status that goes with it.
fn start_failed(error: &StartError) -> ExitCode {
    eprintln!("denon-avr-api-server: {error}");
    match error {
        StartError::AlreadyRunning => ExitCode::from(ALREADY_RUNNING),
        _ => ExitCode::from(1),
    }
}

fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("denon-avr-api-server: {error}");
            return ExitCode::from(1);
        }
    };
    let code = runtime.block_on(run());
    // A read of standard input may still be waiting in the runtime's thread pool,
    // and it must not hold the process after the server has shut down.
    runtime.shutdown_background();
    code
}

async fn run() -> ExitCode {
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
    let mut signals = match Signals::install() {
        Ok(signals) => signals,
        Err(error) => {
            eprintln!("denon-avr-api-server: cannot watch for signals: {error}");
            return ExitCode::from(1);
        }
    };
    let data = arguments.data_directory;
    let paths = EndpointPaths::under(&data);

    // The settings are read before anything is opened, so a file that is wrong stops
    // the server before it serves, and the message names the file.
    let settings = match settings::load(&data.join(SETTINGS_FILE)) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("denon-avr-api-server: {error}");
            return ExitCode::from(2);
        }
    };

    // The lock comes before anything this server writes: the token files, the audit
    // log the control service opens, the ledger it rebuilds. A second server stops
    // here, having touched none of them.
    let lock = match InstanceLock::acquire(&paths) {
        Ok(lock) => lock,
        Err(error) => return start_failed(&error),
    };

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
    let running = match Server::start_locked(
        lock,
        service,
        tokens,
        ServerConfig {
            paths,
            agent: settings.agent_config(),
            limits: Limits::default(),
        },
    )
    .await
    {
        Ok(running) => running,
        Err(error) => return start_failed(&error),
    };
    let reason = tokio::select! {
        signal = signals.next() => Reason::Signal(signal),
        () = parent_gone(), if arguments.exit_with_parent => Reason::ParentGone,
    };
    tracing::info!(?reason, "shutting down");
    // A signal while the shutdown runs means now: the operator is not waiting for
    // operations to finish.
    tokio::spawn(async move {
        let signal = signals.next().await;
        eprintln!("denon-avr-api-server: stopped at once on a second signal");
        std::process::exit(128 + signal);
    });
    running.shutdown().await;
    ExitCode::SUCCESS
}
