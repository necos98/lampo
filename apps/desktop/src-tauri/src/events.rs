//! Forwards domain events to the webview. Event **names** are defined once in
//! `lampo_application::events::names` (mirrored in `src/lib/api/events.ts`).

use lampo_application::events::AppEvent;
use lampo_domain::events::DomainEvent;
use lampo_domain::ports::EventPublisher;
use tauri::{AppHandle, Emitter};

pub struct TauriEventPublisher {
    handle: AppHandle,
}

impl TauriEventPublisher {
    pub fn new(handle: AppHandle) -> Self {
        Self { handle }
    }
}

impl EventPublisher for TauriEventPublisher {
    fn publish(&self, event: DomainEvent) {
        let Some(event) = AppEvent::from_domain(event) else { return };
        // No window (tray mode) is not an error: the event is simply dropped.
        if let Err(e) = self.handle.emit(event.name(), event.payload()) {
            log::debug!("event {} not delivered: {e}", event.name());
        }
    }
}
