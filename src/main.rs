use denon_avr_remote::{AvrCommand, HeosCommand, VolumeCode};

fn usage() {
    println!(
        "Usage:
  denon-avr-remote <command> [value]

Commands:
  avr <command>       Frame an AVR command
  heos <command>      Frame a HEOS command
  volume <db-tenths>  Convert dB tenths to an AVR volume command
  help                Show this help"
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        usage();
        return;
    };

    let result = match command.as_str() {
        "avr" => args
            .next()
            .ok_or_else(|| "avr requires a command".to_owned())
            .and_then(|value| AvrCommand::new(value).map_err(|error| error.to_string()))
            .map(|command| print!("{}", String::from_utf8_lossy(&command.as_bytes()))),
        "heos" => args
            .next()
            .ok_or_else(|| "heos requires a command".to_owned())
            .and_then(|value| HeosCommand::new(value).map_err(|error| error.to_string()))
            .map(|command| print!("{}", String::from_utf8_lossy(&command.as_bytes()))),
        "volume" => args
            .next()
            .ok_or_else(|| "volume requires dB tenths".to_owned())
            .and_then(|value| {
                value
                    .parse::<i16>()
                    .map_err(|_| "volume must be an integer".to_owned())
            })
            .and_then(|value| VolumeCode::from_db_tenths(value).map_err(|error| error.to_string()))
            .map(|volume| println!("{}", volume.command())),
        "help" | "--help" | "-h" => {
            usage();
            Ok(())
        }
        _ => Err("unknown command".to_owned()),
    };

    if let Err(error) = result {
        eprintln!("error: {error}");
        usage();
        std::process::exit(2);
    }
}
