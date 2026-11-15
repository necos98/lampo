//! Human progress lines for long operations, written to stderr (never stdout), throttled to one
//! line per stage change and per 10% so that non-interactive logs stay short.

use std::sync::Mutex;

use lampo_application::events::{ProgressDto, ProgressStageDto};

use crate::io::Sink;

#[derive(Default)]
struct State {
    key: Option<(ProgressStageDto, Option<String>)>,
    last_decile: u64,
}

pub struct ProgressPrinter {
    sink: Sink,
    state: Mutex<State>,
}

fn stage_label(stage: ProgressStageDto) -> &'static str {
    match stage {
        ProgressStageDto::Resolving => "resolving",
        ProgressStageDto::Downloading => "downloading",
        ProgressStageDto::Verifying => "verifying",
        ProgressStageDto::Extracting => "extracting",
        ProgressStageDto::Configuring => "configuring",
        ProgressStageDto::Initializing => "initializing",
        ProgressStageDto::Running => "running",
    }
}

fn is_bytes(stage: ProgressStageDto) -> bool {
    matches!(stage, ProgressStageDto::Downloading | ProgressStageDto::Verifying | ProgressStageDto::Extracting)
}

fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

impl ProgressPrinter {
    pub fn new(sink: Sink) -> Self {
        Self { sink, state: Mutex::new(State::default()) }
    }

    pub fn report(&self, progress: &ProgressDto) {
        // Output lines of a running program are shown as they come (they are the progress).
        if let Some(message) = &progress.message {
            self.sink.line(message);
            return;
        }
        let Ok(mut state) = self.state.lock() else { return };
        let key = (progress.stage, progress.subject.clone());
        let decile = progress.total.filter(|t| *t > 0).map(|t| progress.done.saturating_mul(10) / t).unwrap_or(0);
        let changed = state.key.as_ref() != Some(&key);
        if !changed && decile <= state.last_decile {
            return;
        }
        state.key = Some(key);
        state.last_decile = decile;

        let subject = progress.subject.as_deref().map(|s| format!(" {s}")).unwrap_or_default();
        let amount = match progress.total.filter(|t| *t > 0) {
            Some(total) if is_bytes(progress.stage) => format!(": {} / {} ({}%)", mib(progress.done), mib(total), progress.done.saturating_mul(100) / total),
            Some(total) => format!(": {}/{total}", progress.done),
            None if progress.done > 0 => format!(": {}", if is_bytes(progress.stage) { mib(progress.done) } else { progress.done.to_string() }),
            None => String::new(),
        };
        self.sink.line(&format!("{}{subject}{amount}", stage_label(progress.stage)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::{Capture, Sink};

    fn p(stage: ProgressStageDto, done: u64, total: Option<u64>) -> ProgressDto {
        ProgressDto { stage, done, total, subject: Some("php 8.3.12".into()), message: None }
    }

    #[test]
    fn prints_on_stage_change_and_every_ten_percent_only() {
        let buf = Capture::default();
        let printer = ProgressPrinter::new(Sink::new(buf.clone()));
        let mb = 1024 * 1024;
        for done in [0, 1, 2, 5, 10, 11, 20, 40] {
            printer.report(&p(ProgressStageDto::Downloading, done * mb / 10, Some(40 * mb / 10)));
        }
        printer.report(&p(ProgressStageDto::Extracting, 0, None));
        let text = buf.text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.first().copied(), Some("downloading php 8.3.12: 0.0 MiB / 4.0 MiB (0%)"));
        assert!(lines.len() >= 3 && lines.len() <= 6, "{lines:?}");
        assert_eq!(lines.last().copied(), Some("extracting php 8.3.12"));
    }

    #[test]
    fn output_lines_are_printed_verbatim() {
        let buf = Capture::default();
        let printer = ProgressPrinter::new(Sink::new(buf.clone()));
        for line in ["> git clone", "Receiving objects: 50%", "Receiving objects: 100%"] {
            printer.report(&ProgressDto { stage: ProgressStageDto::Running, done: 0, total: Some(1), subject: Some("git clone".into()), message: Some(line.into()) });
        }
        assert_eq!(buf.text().lines().collect::<Vec<_>>(), ["> git clone", "Receiving objects: 50%", "Receiving objects: 100%"]);
    }
}
