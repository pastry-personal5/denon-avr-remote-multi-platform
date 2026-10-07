//! AVR response correlation.

/// The family a query's response belongs to: the command up to its query marker.
pub fn get_command_family(command: &str) -> &str {
    command
        .split_once('?')
        .map_or(command, |(family, _)| family)
        .trim_end()
}

/// Whether `response` answers a query of `family`, as opposed to being an
/// unsolicited line that happens to share a prefix.
pub fn response_matches(family: &str, response: &str) -> bool {
    let Some(suffix) = response.strip_prefix(family) else {
        return false;
    };
    family != "MV"
        || suffix == "---"
        || (!suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_family_is_the_command_prefix_before_query_marker() {
        assert_eq!(get_command_family("SI?"), "SI");
        assert_eq!(get_command_family("Z2?"), "Z2");
        assert_eq!(get_command_family("PSMULTEQ: ?"), "PSMULTEQ:");
        assert_eq!(get_command_family("PSDYNEQ ?"), "PSDYNEQ");
        assert_eq!(get_command_family("CUSTOM"), "CUSTOM");
    }

    #[test]
    fn response_matching_rejects_wrong_or_malformed_families() {
        assert!(response_matches("SI", "SICD"));
        assert!(!response_matches("SI", "MSSTEREO"));
        assert!(response_matches("MV", "MV---"));
        assert!(response_matches("MV", "MV805"));
        assert!(!response_matches("MV", "MV80.5"));
        assert!(!response_matches("MV", "MVMAX 615"));
        assert!(response_matches("MSQUICK1", "MSQUICK1"));
        assert!(!response_matches("MSQUICK1", "MSQUICKX"));
    }
}
