//! The PHP process exclusion after a PHP is installed or uninstalled: what Lampo proposes
//! ([`DefenderExclusionAdvice`]) and the removal of the exclusions of the executables no longer
//! installed (`uninstalled`).

use std::path::PathBuf;

use lampo_domain::package::{EntryRole, InstalledPackage, PackageKind};
use lampo_domain::ports::{InstalledPackageRegistry, ProcessExclusionRecordWriter};
use lampo_domain::scan_exclusion::{DefenderState, ProcessExclusionRequest, ScanExclusionChange, ScanExclusionOutcome};

use super::{cli, gui};
use crate::context::CallContext;
use crate::dto::{DefenderExclusionRequestDto, DefenderExclusionSuggestionDto, ScanExclusionChangeDto, ScanExclusionOutcomeDto};
use crate::error::ErrorCode;
use crate::testing::World;

const ADD_COMMAND: &str = "lampo defender exclusion add --php --allow-elevation";
const REMOVE_COMMAND: &str = "lampo defender exclusion remove --uninstalled --allow-elevation";

fn entry(package: &InstalledPackage, role: EntryRole) -> PathBuf {
    package.entry(role).map(PathBuf::from).unwrap()
}

fn package(w: &World, kind: PackageKind, version: &str) -> InstalledPackage {
    w.installed.list().unwrap().into_iter().find(|p| p.id.kind == kind && p.id.version.to_string() == version).unwrap()
}

/// `php-cgi.exe` of PHP 8.3 (installed by [`World::with_basics`]).
fn php_83(w: &World) -> PathBuf {
    entry(&package(w, PackageKind::PHP, "8.3.14"), EntryRole::FastCgi)
}

/// Every server executable of [`World::with_basics`]: PHP 8.3, PHP 8.4, Apache.
fn servers(w: &World) -> Vec<PathBuf> {
    let mut servers = vec![php_83(w), entry(&package(w, PackageKind::PHP, "8.4.1"), EntryRole::FastCgi), entry(&package(w, PackageKind::APACHE, "2.4.62"), EntryRole::Server)];
    servers.sort();
    servers
}

/// Lampo applied the exclusion of `processes` (as an `add --php` that Defender confirmed).
fn applied(w: &World, processes: Vec<PathBuf>) {
    let mut record = w.exclusion_record.get();
    record.record(ProcessExclusionRequest { change: ScanExclusionChange::Add, outcome: ScanExclusionOutcome::Applied, processes });
    w.exclusion_record.write(&record).unwrap();
}

/// Every server executable excluded, Defender scanning: nothing to propose yet.
fn covered() -> World {
    let w = World::with_basics();
    w.defender.set(DefenderState::RealTimeProtection);
    applied(&w, servers(&w));
    w
}

fn display(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|p| p.display().to_string()).collect()
}

fn add_suggestion(processes: &[PathBuf]) -> DefenderExclusionSuggestionDto {
    DefenderExclusionSuggestionDto {
        change: ScanExclusionChangeDto::Add,
        processes: display(processes),
        request: DefenderExclusionRequestDto::new(ScanExclusionChangeDto::Add, true, false),
        command: ADD_COMMAND.into(),
    }
}

fn remove_suggestion(processes: &[PathBuf]) -> DefenderExclusionSuggestionDto {
    DefenderExclusionSuggestionDto {
        change: ScanExclusionChangeDto::Remove,
        processes: display(processes),
        request: DefenderExclusionRequestDto::remove_uninstalled(),
        command: REMOVE_COMMAND.into(),
    }
}

/// Installs PHP 8.5 and returns its `php-cgi.exe`.
fn install_php_85(w: &World) -> PathBuf {
    entry(&w.installed.add(PackageKind::PHP, "8.5.0", None), EntryRole::FastCgi)
}

#[tokio::test]
async fn installing_a_php_not_covered_proposes_to_update_the_exclusion() {
    let w = covered();
    assert!(w.defender_exclusion_advice().suggestions().await.is_empty(), "every PHP covered");
    let new_php = install_php_85(&w);
    assert_eq!(w.defender_exclusion_advice().suggestions().await, vec![add_suggestion(&[new_php])]);
}

#[tokio::test]
async fn nothing_is_proposed_when_the_exclusion_was_never_applied() {
    let w = World::with_basics();
    w.defender.set(DefenderState::RealTimeProtection);
    install_php_85(&w);
    assert!(w.defender_exclusion_advice().suggestions().await.is_empty(), "the doctor tells them");

    let mut record = w.exclusion_record.get();
    record.record(ProcessExclusionRequest { change: ScanExclusionChange::Add, outcome: ScanExclusionOutcome::Cancelled, processes: servers(&w) });
    w.exclusion_record.write(&record).unwrap();
    assert!(w.defender_exclusion_advice().suggestions().await.is_empty(), "a request that did not apply is not an exclusion");
    assert_eq!(w.defender.reads(), 0, "Defender's state is not read for nothing");
}

#[tokio::test]
async fn nothing_is_added_unless_real_time_protection_scans() {
    for state in [DefenderState::RealTimeProtectionOff, DefenderState::Passive, DefenderState::Unavailable] {
        let w = covered();
        w.defender.set(state);
        install_php_85(&w);
        assert!(w.defender_exclusion_advice().suggestions().await.is_empty(), "{state:?}");
    }
    let w = covered();
    w.defender.unreadable();
    install_php_85(&w);
    assert!(w.defender_exclusion_advice().suggestions().await.is_empty(), "an unknown state proposes nothing");
}

#[tokio::test]
async fn uninstalling_a_covered_php_proposes_to_remove_its_exclusion() {
    let w = covered();
    let php = package(&w, PackageKind::PHP, "8.3.14");
    w.installed.remove(&php.id).await.unwrap();
    assert_eq!(w.defender_exclusion_advice().suggestions().await, vec![remove_suggestion(&[entry(&php, EntryRole::FastCgi)])]);
}

#[tokio::test]
async fn a_removal_is_proposed_while_defender_runs_even_without_real_time_protection() {
    for (state, proposed) in [(DefenderState::RealTimeProtectionOff, true), (DefenderState::Passive, false), (DefenderState::Unavailable, false)] {
        let w = covered();
        w.defender.set(state);
        let php = package(&w, PackageKind::PHP, "8.3.14");
        w.installed.remove(&php.id).await.unwrap();
        assert_eq!(!w.defender_exclusion_advice().suggestions().await.is_empty(), proposed, "{state:?}");
    }
}

#[tokio::test]
async fn adding_and_removing_are_proposed_together_adding_first() {
    let w = covered();
    let old = package(&w, PackageKind::PHP, "8.3.14");
    w.installed.remove(&old.id).await.unwrap();
    let new_php = install_php_85(&w);
    assert_eq!(w.defender_exclusion_advice().suggestions().await, vec![add_suggestion(&[new_php]), remove_suggestion(&[entry(&old, EntryRole::FastCgi)])]);
}

#[tokio::test]
async fn a_record_that_does_not_load_proposes_nothing() {
    let w = covered();
    install_php_85(&w);
    w.exclusion_record.corrupt();
    assert!(w.defender_exclusion_advice().suggestions().await.is_empty());
}

/// The proposed request, made: the uninstalled executable only, then nothing left to propose.
#[tokio::test]
async fn the_proposed_removal_names_only_the_uninstalled_executables() {
    let w = covered();
    let still_installed: Vec<PathBuf> = servers(&w).into_iter().filter(|exe| *exe != php_83(&w)).collect();
    let php = package(&w, PackageKind::PHP, "8.3.14");
    w.installed.remove(&php.id).await.unwrap();
    let [suggestion] = <[_; 1]>::try_from(w.defender_exclusion_advice().suggestions().await).unwrap();

    let result = w.defender_exclusion(ScanExclusionOutcome::Applied).change(suggestion.request, gui()).await.unwrap();
    assert_eq!((result.change, result.outcome), (ScanExclusionChangeDto::Remove, ScanExclusionOutcomeDto::Applied));
    assert_eq!(result.processes, display(&[entry(&php, EntryRole::FastCgi)]));
    assert_eq!(w.exclusion_record.get().excluded, still_installed, "the installed ones stay excluded");
    assert!(w.defender_exclusion_advice().suggestions().await.is_empty());
}

#[tokio::test]
async fn the_proposed_addition_covers_the_new_php() {
    let w = covered();
    install_php_85(&w);
    let [suggestion] = <[_; 1]>::try_from(w.defender_exclusion_advice().suggestions().await).unwrap();
    w.defender_exclusion(ScanExclusionOutcome::Applied).change(suggestion.request, CallContext::cli(true)).await.unwrap();
    assert!(w.defender_exclusion_advice().suggestions().await.is_empty());
}

#[tokio::test]
async fn uninstalled_without_anything_to_remove_never_prompts() {
    let w = covered();
    let err = w.defender_exclusion(ScanExclusionOutcome::Applied).change(DefenderExclusionRequestDto::remove_uninstalled(), gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation, "{err}");
    assert_eq!(w.exclusion_record.get().last_request.map(|r| r.outcome), Some(ScanExclusionOutcome::Applied), "nothing requested");
}

#[tokio::test]
async fn uninstalled_executables_are_never_added() {
    let w = covered();
    let request = DefenderExclusionRequestDto { uninstalled: true, ..DefenderExclusionRequestDto::new(ScanExclusionChangeDto::Add, true, false) };
    let err = w.defender_exclusion(ScanExclusionOutcome::Applied).change(request.clone(), gui()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation, "{err}");
    let err = w.defender_exclusion(ScanExclusionOutcome::Applied).commands(request).unwrap_err();
    assert_eq!(err.code, ErrorCode::Validation, "{err}");
}

#[tokio::test]
async fn uninstalled_needs_a_prompt_from_the_cli_too() {
    let w = covered();
    let php = package(&w, PackageKind::PHP, "8.3.14");
    w.installed.remove(&php.id).await.unwrap();
    let err = w.defender_exclusion(ScanExclusionOutcome::Applied).change(DefenderExclusionRequestDto::remove_uninstalled(), cli()).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::ElevationRequired, "{err}");
}

#[test]
fn uninstalled_is_optional_on_the_wire() {
    let request: DefenderExclusionRequestDto = serde_json::from_value(serde_json::json!({ "change": "remove", "php": false, "mysql": false, "uninstalled": true })).unwrap();
    assert_eq!(request, DefenderExclusionRequestDto::remove_uninstalled());
    let suggestion = serde_json::to_value(remove_suggestion(&[PathBuf::from(r"C:\php-cgi.exe")])).unwrap();
    assert_eq!(suggestion["request"]["uninstalled"], true);
    assert_eq!(suggestion["command"], REMOVE_COMMAND);
}
