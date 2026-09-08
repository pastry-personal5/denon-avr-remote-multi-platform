use std::sync::Arc;

fn main() -> iced::Result {
    let services = denon_avr_gui_lib::GuiServices {
        factory: Arc::new(denon_avr_infrastructure::AvrSessionFactory::default()),
        configuration: Arc::new(denon_avr_infrastructure::YamlConfigRepository::default()),
        discovery: Arc::new(denon_avr_infrastructure::SsdpDiscoveryAdapter),
    };
    iced::application(
        move || denon_avr_gui_lib::boot_with_services(services.clone()),
        denon_avr_gui_lib::update,
        denon_avr_gui_lib::view,
    )
    .subscription(denon_avr_gui_lib::subscription)
    .theme(denon_avr_gui_lib::app_theme)
    .window(iced::window::Settings {
        size: iced::Size::new(1400.0, 880.0),
        min_size: Some(iced::Size::new(1400.0, 880.0)),
        ..Default::default()
    })
    .title("Denon AVR Remote")
    .run()
}
