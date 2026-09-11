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
    let services = denon_avr_gui_lib::GuiServices {
        factory: Arc::new(denon_avr_infrastructure::CanonicalSessionFactory::default()),
        configuration: Arc::new(denon_avr_infrastructure::YamlConfigRepository::default()),
        discovery: Arc::new(denon_avr_infrastructure::SsdpDiscoveryAdapter),
    };
    let result = iced::application(
        move || denon_avr_gui_lib::boot_with_services(services.clone()),
        denon_avr_gui_lib::update,
        denon_avr_gui_lib::view,
    )
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
