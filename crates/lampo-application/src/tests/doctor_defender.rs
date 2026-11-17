//! The doctor's `defender` check: Defender's real-time protection slowing down PHP, from its
//! state and from what Lampo remembers of its process exclusion requests.

use std::path::PathBuf;
use std::sync::Arc;

use lampo_domain::package::{EntryRole, PackageKind};
use lampo_domain::ports::{InstalledPackageRegistry, ProcessExclusionRecordWriter};
use lampo_domain::scan_exclusion::{DefenderState, ProcessExclusionRequest, ScanExclusionChange, ScanExclusionOutcome};

use crate::context::CallContext;
use crate::dto::{CheckStatus, DefenderExclusionRequestDto, DoctorCheckDto, ScanExclusionChangeDto};
use crate::testing::*;
use crate::use_cases::*;

const ADD_EXCLUSION: &str = "lampo defender exclusion add --php --allow-elevation";
const IT_COMMANDS: &str = "lampo defender exclusion commands --php";

async fn defender_line(w: &World) -> DoctorCheckDto {
    let report = Doctor::new(w.doctor_deps(Arc::new(FakeLocator::default()))).run().await.unwrap();
    report.checks.iter().find(|c| c.id == "defender").cloned().unwrap()
}

/// `php-cgi.exe` of every installed PHP (PHP 8.3 and 8.4 in [`World::with_basics`]).
fn php_servers(w: &World) -> Vec<PathBuf> {
    w.installed.list().unwrap().iter().filter(|p| p.id.kind == PackageKind::PHP).filter_map(|p| p.entry(EntryRole::FastCgi)).map(PathBuf::from).collect()
}

/// The httpd.exe of Apache, also named by a `--php` request.
fn httpd(w: &World) -> PathBuf {
    w.installed.list().unwrap().iter().find(|p| p.id.kind == PackageKind::APACHE).and_then(|p| p.entry(EntryRole::Server)).map(PathBuf::from).unwrap()
}

fn remember(w: &World, outcome: ScanExclusionOutcome, processes: Vec<PathBuf>) {
    let mut record = w.exclusion_record.get();
    record.record(ProcessExclusionRequest { change: ScanExclusionChange::Add, outcome, processes });
    w.exclusion_record.write(&record).unwrap();
}

fn scanning() -> World {
    let w = World::with_basics();
    w.defender.set(DefenderState::RealTimeProtection);
    w
}

fn assert_warns_with_the_fixes(line: &DoctorCheckDto) {
    assert_eq!(line.status, CheckStatus::Warning, "{line:?}");
    assert!(line.message.contains("8-24 s"), "{}", line.message);
    let fix = line.fix.as_deref().unwrap();
    assert!(fix.contains(ADD_EXCLUSION) && fix.contains(IT_COMMANDS), "{fix}");
}

#[tokio::test]
async fn real_time_protection_without_any_exclusion_warns() {
    let w = scanning();
    let line = defender_line(&w).await;
    assert_warns_with_the_fixes(&line);
    assert!(line.message.contains("the PHP processes are not excluded"), "{}", line.message);
}

#[tokio::test]
async fn an_exclusion_covering_every_installed_php_is_ok() {
    let w = scanning();
    let mut processes = php_servers(&w);
    processes.push(httpd(&w));
    remember(&w, ScanExclusionOutcome::Applied, processes);
    let line = defender_line(&w).await;
    assert_eq!(line.status, CheckStatus::Ok, "{line:?}");
    assert_eq!(line.fix, None);
}

#[tokio::test]
async fn a_php_installed_after_the_exclusion_warns_and_is_named() {
    let w = scanning();
    remember(&w, ScanExclusionOutcome::Applied, php_servers(&w));
    let new_php = w.installed.add(PackageKind::PHP, "8.2.27", None).entry(EntryRole::FastCgi).unwrap().to_path_buf();
    let line = defender_line(&w).await;
    assert_warns_with_the_fixes(&line);
    assert!(line.message.contains(&format!("not covered by the exclusion Lampo applied: {}", new_php.display())), "{}", line.message);
    for covered in php_servers(&w).iter().filter(|exe| **exe != new_php) {
        assert!(!line.message.contains(&covered.display().to_string()), "{covered:?} is covered: {}", line.message);
    }
}

#[tokio::test]
async fn a_request_that_was_not_applied_warns_with_its_outcome() {
    for (outcome, reason) in [
        (ScanExclusionOutcome::Cancelled, "cancelled at the UAC prompt"),
        (ScanExclusionOutcome::Refused, "Defender refused the last request"),
        (ScanExclusionOutcome::NotConfirmed, "was not confirmed"),
        (ScanExclusionOutcome::DefenderUnavailable, "not the active antivirus"),
    ] {
        let w = scanning();
        remember(&w, outcome, php_servers(&w));
        let line = defender_line(&w).await;
        assert_warns_with_the_fixes(&line);
        assert!(line.message.contains(reason), "{outcome:?}: {}", line.message);
    }
}

#[tokio::test]
async fn an_exclusion_removed_since_warns_again() {
    let w = scanning();
    remember(&w, ScanExclusionOutcome::Applied, php_servers(&w));
    let mut record = w.exclusion_record.get();
    record.record(ProcessExclusionRequest { change: ScanExclusionChange::Remove, outcome: ScanExclusionOutcome::Applied, processes: php_servers(&w) });
    w.exclusion_record.write(&record).unwrap();
    assert_warns_with_the_fixes(&defender_line(&w).await);
}

#[tokio::test]
async fn a_record_that_does_not_load_counts_as_no_exclusion() {
    let w = scanning();
    remember(&w, ScanExclusionOutcome::Applied, php_servers(&w));
    w.exclusion_record.corrupt();
    assert_warns_with_the_fixes(&defender_line(&w).await);
}

#[tokio::test]
async fn defender_not_scanning_on_access_never_warns() {
    for (state, says) in
        [(DefenderState::Passive, "passive mode"), (DefenderState::Unavailable, "not the active antivirus"), (DefenderState::RealTimeProtectionOff, "real-time protection is off")]
    {
        let w = World::with_basics();
        w.defender.set(state);
        let line = defender_line(&w).await;
        assert_eq!((line.status, line.fix.as_deref()), (CheckStatus::Ok, None), "{state:?}");
        assert!(line.message.contains(says), "{state:?}: {}", line.message);
    }
}

#[tokio::test]
async fn an_unreadable_state_is_only_information() {
    let w = World::with_basics();
    w.defender.unreadable();
    let report = Doctor::new(w.doctor_deps(Arc::new(FakeLocator::default()))).run().await.unwrap();
    let line = report.checks.iter().find(|c| c.id == "defender").unwrap();
    assert_eq!(line.status, CheckStatus::Ok);
    assert!(line.message.contains("cannot be read"), "{}", line.message);
}

#[tokio::test]
async fn without_php_installed_there_is_nothing_to_slow_down() {
    let w = World::new();
    w.defender.set(DefenderState::RealTimeProtection);
    w.installed.add(PackageKind::APACHE, "2.4.62", None);
    let line = defender_line(&w).await;
    assert_eq!(line.status, CheckStatus::Ok, "{line:?}");
    assert!(line.message.contains("no PHP"), "{}", line.message);
}

/// The doctor's verdict after a real request through the use case: the warning goes away.
#[tokio::test]
async fn an_applied_php_request_clears_the_warning() {
    let w = scanning();
    assert_eq!(defender_line(&w).await.status, CheckStatus::Warning);
    let request = DefenderExclusionRequestDto::new(ScanExclusionChangeDto::Add, true, false);
    w.defender_exclusion(ScanExclusionOutcome::Applied).change(request, CallContext::cli(true)).await.unwrap();
    assert_eq!(w.exclusion_record.get().excluded.len(), php_servers(&w).len() + 1, "PHP and Apache");
    assert_eq!(defender_line(&w).await.status, CheckStatus::Ok);
}
