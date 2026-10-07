use std::sync::Arc;

mod logging;

fn main() -> iced::Result {
    // Keep the guard alive until Iced exits so the non-blocking log worker can
    // flush every queued event before the process terminates.
    let _logging = match logging::initialize() {
        Ok(guard) => {
            tracing::info!(
                log_directory = %guard.directory().display(),
                "desktop logging initialized"
            );
            Some(guard)
        }
        Err(error) => {
            eprintln!("Unable to initialize desktop logging: {error}");
            None
        }
    };
    // The desktop composes the control service, which owns the receiver
    // connection. The GUI sees only the port and a hook to close it.
    let service = Arc::new(denon_avr_application::ControlService::new(
        Arc::new(denon_avr_infrastructure::X3800hConnector::default()),
        Arc::new(denon_avr_infrastructure::YamlConfigRepository::default()),
        Arc::new(denon_avr_infrastructure::SsdpDiscoveryAdapter),
        denon_avr_application::ServiceConfig::default(),
    ));
    let services = denon_avr_gui_lib::GuiServices {
        control: service.operator(),
        shutdown: {
            let service = Arc::clone(&service);
            Arc::new(move || {
                let service = Arc::clone(&service);
                Box::pin(async move { service.shutdown().await })
            })
        },
    };
    let result = iced::application(
        move || denon_avr_gui_lib::boot_with_services(services.clone()),
        denon_avr_gui_lib::update,
        denon_avr_gui_lib::view,
    )
    // The GUI closes the receiver connection before it lets the window go.
    .exit_on_close_request(false)
    .subscription(denon_avr_gui_lib::subscription)
    .theme(denon_avr_gui_lib::app_theme)
    .scale_factor(denon_avr_gui_lib::app_scale)
    .window(iced::window::Settings {
        size: iced::Size::new(1180.0, 820.0),
        min_size: Some(iced::Size::new(1100.0, 760.0)),
        ..Default::default()
    })
    .title("Denon AVR Remote")
    .run();
    tracing::info!(result = ?result, "desktop application stopped");
    result
}
