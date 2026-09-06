//! Shared, transport-independent AVR response correlation and field parsing.

pub fn command_family(command: &str) -> &str {
    command
        .split_once('?')
        .map_or(command, |(family, _)| family)
}

pub fn response_matches(family: &str, response: &str) -> bool {
    let Some(suffix) = response.strip_prefix(family) else {
        return false;
    };
    family != "MV"
        || suffix == "---"
        || (!suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
}

pub fn parse_power(response: &str) -> Result<String, String> {
    match response {
        "PWON" => Ok("on".into()),
        "PWSTANDBY" => Ok("standby".into()),
        value => Err(format!("unexpected power response {value}")),
    }
}
pub fn parse_input(response: &str) -> Result<String, String> {
    prefixed(response, "SI", "input")
}
pub fn parse_surround(response: &str) -> Result<String, String> {
    prefixed(response, "MS", "surround")
}
fn prefixed(response: &str, prefix: &str, name: &str) -> Result<String, String> {
    response
        .strip_prefix(prefix)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("unexpected {name} response {response}"))
}
pub fn parse_volume(response: &str) -> Result<String, String> {
    let code = response
        .strip_prefix("MV")
        .ok_or_else(|| format!("unexpected volume response {response}"))?;
    if code == "---" {
        return Ok("unknown".into());
    }
    let half = code.ends_with('5');
    let base_text = if half { &code[..code.len() - 1] } else { code };
    let base = base_text
        .parse::<i16>()
        .map_err(|_| format!("invalid volume code {code}"))?;
    if !(0..=98).contains(&base) || (half && code.len() != 3) || (!half && code.len() != 2) {
        return Err(format!("invalid volume code {code}"));
    }
    let db = ((base - 80) * 10 + i16::from(half) * 5) as f32 / 10.0;
    Ok(format!("code {code} ({db:.1} dB)"))
}
pub fn parse_mute(response: &str) -> Result<String, String> {
    match response {
        "MUON" => Ok("on".into()),
        "MUOFF" => Ok("off".into()),
        v => Err(format!("unexpected mute response {v}")),
    }
}
