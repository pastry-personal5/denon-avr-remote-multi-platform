//! Quick Select names, as the GUI shows them.

use denon_avr_domain::{QuickSelectNameObservation, QuickSelectSlot, QuickSelectSnapshot};

/// Put the names the receiver reported into `quick_select`, stamped with the
/// connection generation they were read under.
pub fn apply_names(
    quick_select: &mut QuickSelectSnapshot,
    mut observation: QuickSelectNameObservation,
    generation: u64,
) -> QuickSelectNameObservation {
    observation.generation = generation;
    quick_select.generation = generation;
    for (index, name) in observation.names.iter().enumerate() {
        if let Some(name) = name {
            let slot = QuickSelectSlot::new(index as u8 + 1)
                .expect("Quick Select name response has four slots");
            quick_select.set_name(slot, name.clone());
        }
    }
    observation
}

#[cfg(test)]
mod tests {
    use super::*;
    use denon_avr_domain::QuickSelectName;

    #[test]
    fn reported_names_fill_their_slots_and_carry_the_generation() {
        let mut snapshot = QuickSelectSnapshot::default();
        let observation = QuickSelectNameObservation {
            names: [
                Some(QuickSelectName::new("Cinema").unwrap()),
                None,
                Some(QuickSelectName::new("Music").unwrap()),
                None,
            ],
            ..QuickSelectNameObservation::default()
        };
        let applied = apply_names(&mut snapshot, observation, 3);
        assert_eq!(applied.generation, 3);
        assert_eq!(snapshot.generation, 3);
        let first = QuickSelectSlot::new(1).unwrap();
        assert_eq!(
            snapshot
                .preset(first)
                .and_then(|p| p.name.as_ref())
                .map(|n| n.as_str()),
            Some("Cinema")
        );
    }
}
