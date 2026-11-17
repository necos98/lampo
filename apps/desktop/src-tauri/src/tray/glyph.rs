//! Which tray glyph the service states call for, and which one is on screen. Every status change
//! is recorded, even while the glyph cannot be drawn (the app going away, see
//! [`super::track_service_status`]): the next [`Glyph::redraw`] then catches up with them.

use std::collections::HashMap;

use lampo_application::dto::ServiceStateDto;

#[derive(Debug, Default)]
pub(super) struct Glyph {
    /// Last known state of every service, by id.
    states: HashMap<String, ServiceStateDto>,
    /// `Some(running)`: the glyph last drawn; `None`: none drawn yet.
    shown: Option<bool>,
}

impl Glyph {
    pub(super) fn record(&mut self, service: String, state: ServiceStateDto) {
        self.states.insert(service, state);
    }

    /// The glyph to draw (`true`: running) when the one on screen no longer matches the recorded
    /// states; it is then taken as drawn. `None`: nothing to change.
    pub(super) fn redraw(&mut self) -> Option<bool> {
        let running = self.states.values().any(|s| *s == ServiceStateDto::Running);
        (self.shown != Some(running)).then(|| {
            self.shown = Some(running);
            running
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(states: &[(&str, ServiceStateDto)]) -> Glyph {
        let mut glyph = Glyph::default();
        for (service, state) in states {
            glyph.record((*service).to_string(), *state);
        }
        glyph
    }

    #[test]
    fn the_first_redraw_always_draws() {
        assert_eq!(glyph(&[]).redraw(), Some(false));
        assert_eq!(glyph(&[("apache", ServiceStateDto::Running)]).redraw(), Some(true));
    }

    #[test]
    fn redraws_only_when_any_running_flips() {
        let mut g = glyph(&[("apache", ServiceStateDto::Running)]);
        assert_eq!(g.redraw(), Some(true));
        g.record("mysql-8.4".into(), ServiceStateDto::Running);
        assert_eq!(g.redraw(), None, "still running");
        g.record("apache".into(), ServiceStateDto::Stopped);
        assert_eq!(g.redraw(), None, "mysql still runs");
        g.record("mysql-8.4".into(), ServiceStateDto::Failed);
        assert_eq!(g.redraw(), Some(false));
        assert_eq!(g.redraw(), None, "already drawn");
    }

    #[test]
    fn changes_recorded_without_drawing_are_drawn_by_the_next_redraw() {
        // `prepare_update` stops everything while the shutdown gate forbids drawing, then the
        // update is cancelled.
        let mut g = glyph(&[("apache", ServiceStateDto::Running), ("mysql-8.4", ServiceStateDto::Running)]);
        assert_eq!(g.redraw(), Some(true));
        g.record("apache".into(), ServiceStateDto::Stopping);
        g.record("apache".into(), ServiceStateDto::Stopped);
        g.record("mysql-8.4".into(), ServiceStateDto::Stopped);
        assert_eq!(g.redraw(), Some(false), "the grey glyph once the gate reopens");
    }

    #[test]
    fn a_change_undone_before_the_redraw_draws_nothing() {
        let mut g = glyph(&[("apache", ServiceStateDto::Running)]);
        assert_eq!(g.redraw(), Some(true));
        g.record("apache".into(), ServiceStateDto::Stopped);
        g.record("apache".into(), ServiceStateDto::Running);
        assert_eq!(g.redraw(), None);
    }
}
