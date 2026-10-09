//! Supplemental reads over HTTP: the audio, video, and Audyssey information,
//! the source catalog, and the Quick Select names.

use super::*;

impl Gui {
    /// Forget what was read over HTTP for a connection that is gone: the Quick
    /// Select names, the source catalog, and the audio, video, and Audyssey
    /// information.
    pub(super) fn invalidate_supplemental(&mut self) {
        self.quick_select.invalidate();
        self.quick_select.generation = self.generation;
        self.source_catalog.invalidate(self.generation);
        self.snapshot.invalidate_http_information(self.generation);
    }

    /// Read the receiver's audio, video, and Audyssey information, one read at a
    /// time. A request that arrives while one is running runs once more after it.
    pub(super) fn request_http_information(&mut self) -> Task<Message> {
        if self.selection.is_none()
            || !http_policy::should_read(&self.selected_capabilities(), true)
        {
            return Task::none();
        }
        if self.snapshot.power.value() != Some(&PowerState::On) {
            self.snapshot.invalidate_http_information(self.generation);
            return Task::none();
        }
        if self.supplemental.http_read_in_flight {
            self.supplemental.http_read_again = true;
            return Task::none();
        }
        self.supplemental.http_read_in_flight = true;
        let id = self.next_request();
        self.command(BridgeCommand::ReadHttpInformation(id, self.generation))
    }

    pub(super) fn request_source_catalog(&mut self) -> Task<Message> {
        if self.selection.is_none() || self.supplemental.catalog_read_in_flight {
            return Task::none();
        }
        if !self.selected_capabilities().source_catalog_read {
            self.announce("The selected receiver has no validated source catalog capability.");
            return Task::none();
        }
        self.supplemental.catalog_read_in_flight = true;
        let id = self.next_request();
        self.command(BridgeCommand::ReadSourceCatalog(id, self.generation))
    }

    pub(super) fn request_quick_select_names(&mut self) -> Task<Message> {
        if self.selection.is_none()
            || self.supplemental.names_read_in_flight
            || !self.selected_capabilities().quick_select_names
        {
            return Task::none();
        }
        self.supplemental.names_read_in_flight = true;
        let id = self.next_request();
        self.command(BridgeCommand::ReadQuickSelectNames(id, self.generation))
    }
}
