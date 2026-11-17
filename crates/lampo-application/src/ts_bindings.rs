//! TypeScript declarations of the wire types, generated from the Rust DTOs into
//! [`BINDINGS_FILE`] (imported by the GUI through `src/lib/api/types.ts`).
//!
//! `bindings_are_up_to_date` fails when the committed file differs from what the DTOs produce.
//! Regenerate it after changing a DTO:
//!
//! ```text
//! cargo test -p lampo-application --lib ts_bindings -- --ignored
//! ```
//!
//! Why `ts-rs` and not `specta`/`tauri-specta`: it needs only derives and attributes on the DTOs
//! (`cfg_attr(test)`, so the release binaries never link it), while `tauri-specta` would
//! rewrite every Tauri command signature and their registration. The command wrappers stay
//! hand-written in `src/lib/api/commands.ts`, typed with these declarations.
//!
//! A type is exported when it is listed in [`declarations`]; every type it refers to must be
//! listed too (`every_dependency_is_declared`). Large integers (`u64`, `usize`) are `number`:
//! that is how `serde_json` sends them.
//!
//! Attributes, all `cfg_attr(test, ...)`: `ts(rename = "Project")` gives the GUI name (no `Dto`
//! suffix); `ts(optional = nullable)` marks a request field the backend defaults when it is
//! omitted (`field?: T`; an `Option` stays `T | null`). Fields with `serde(default,
//! skip_serializing_if = ...)` are optional without it.

use std::any::TypeId;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ts_rs::{Config, TS, TypeVisitor};

use crate::dto::*;
use crate::error::{AppError, ErrorCode};
use crate::events::*;

/// Relative to the workspace root.
const BINDINGS_FILE: &str = "apps/desktop/src/lib/api/bindings.ts";

/// The command that rewrites [`BINDINGS_FILE`].
const REGENERATE: &str = "cargo test -p lampo-application --lib ts_bindings -- --ignored";

/// TypeScript type for `u64`/`usize`: JSON numbers on the wire, never `bigint`.
const LARGE_INT_TYPE: &str = "number";

/// One exported declaration: its TypeScript name, the types it refers to and its source.
struct Declaration {
    name: String,
    dependencies: Vec<TypeId>,
    type_id: TypeId,
    source: String,
}

fn config() -> Config {
    Config::new().with_large_int(LARGE_INT_TYPE)
}

fn declare<T: TS + 'static>(cfg: &Config) -> Declaration {
    let mut dependencies = DependencyIds::default();
    T::visit_dependencies(&mut dependencies);
    let docs = T::docs().unwrap_or_default();
    Declaration { name: T::ident(cfg), dependencies: dependencies.0, type_id: TypeId::of::<T>(), source: format!("{docs}export {}", T::decl(cfg)) }
}

/// The declared types a type refers to (built-ins such as `string` or `Array` have no
/// declaration, hence no output path).
#[derive(Default)]
struct DependencyIds(Vec<TypeId>);

impl TypeVisitor for DependencyIds {
    fn visit<T: TS + 'static + ?Sized>(&mut self) {
        if T::output_path().is_some() {
            self.0.push(TypeId::of::<T>());
        }
    }
}

/// Exported types, one section per area, in the order they appear in the file.
fn declarations(cfg: &Config) -> Vec<(&'static str, Vec<Declaration>)> {
    macro_rules! section {
        ($title:literal: $($ty:ty),+ $(,)?) => {
            ($title, vec![$(declare::<$ty>(cfg)),+])
        };
    }
    vec![
        section!("Errors": ErrorCode, AppError),
        section!("Projects":
            SiteAddressDto, AddressingDto, TlsModeDto, MysqlCredentialsDto, ProjectEnvironmentDto, ServiceUseDto, AddressSuggestionsDto,
            DatabaseConnectionDto, MailConnectionDto, ProjectWarningCodeDto, ProjectWarningDto, ProjectDto, UnreadableProjectDto, ProjectListDto, ProjectSummaryDto,
            CreateProjectRequest, DatabaseSetupStatusDto, DatabaseSetupDto, CreateProjectResultDto, DeleteProjectResultDto,
            UpdateProjectRequest, SetProjectActiveRequest, SetProjectPhpRequest, RelinkProjectRequest, ExportProjectRequest,
            ImportProjectRequest, ImportProjectResultDto, DetectionDto, DetectedAppDto,
        ),
        section!("New project setup": StarterKitDto, TestingFrameworkDto, ScaffoldSourceDto, ScaffoldProjectRequest, ScaffoldProjectResultDto),
        section!("Presets": PresetDto, SavePresetRequest),
        section!("Packages and tasks":
            PackageIdDto, CatalogChannelDto, CatalogEntryDto, InstalledPackageDto, InstallPackageRequest, UpdateAvailableDto,
            TaskStartedDto, PhpExtensionDto, TaskOperationDto, RunningTaskDto, FirstRunItemDto, FirstRunStatusDto,
        ),
        section!("Services": ServiceStateDto, ServiceStatusDto, ServicesRequest, SyncReportDto, StatusDto),
        section!("Worktree environments":
            DatabaseModeDto, WorktreeEnvironmentDto, WorktreeUpRequest, WorktreeUpResultDto, WorktreeDownRequest,
            WorktreeDownResultDto, PruneWorktreesRequest, PruneReportDto, PruneFailureDto,
        ),
        section!("Logs": LogSourceDto, LogTailRequest, LogLineDto, LogLinesDto, LogExportRequest),
        section!("Settings": ServiceInstanceDto, ServiceKindDto, PortRangeDto, SettingsDto),
        section!("Diagnostics and shims":
            CheckStatus, DoctorCheckDto, VcRuntimeIssueDto, PortConflictDto, ShadowedToolDto, DoctorReportDto, ShimStatusDto,
        ),
        section!("Integrations": ClaudeCodeStateDto, ClaudeCodeStatusDto, StepOutcomeDto, ClaudeCodeStepDto, ClaudeCodeActionDto),
        section!(
            "Defender exclusion": ScanExclusionChangeDto, ScanExclusionOutcomeDto, ScanExclusionResultDto, ScanExclusionInfoDto, DefenderExclusionRequestDto,
            DefenderExclusionResultDto, DefenderExclusionInfoDto
        ),
        section!("App": AppInfoDto),
        section!("Event payloads":
            ProgressStageDto, ProgressDto, TaskProgressPayload, TaskOutcomeDto, TaskFinishedPayload, ServiceStatusPayload,
            LogStreamDto, ServiceLogPayload, ProjectsChangedPayload, WorktreesChangedPayload, AutoRestartStepDto, ServiceAutoRestartPayload,
        ),
    ]
}

/// The whole file: header, then each section's declarations separated by a blank line.
fn render(sections: &[(&str, Vec<Declaration>)]) -> String {
    let mut out = format!(
        "// GENERATED from the Rust DTOs by `{REGENERATE}`\n\
         // (crates/lampo-application/src/ts_bindings.rs). Do not edit: change the DTOs and regenerate.\n"
    );
    for (title, declarations) in sections {
        out.push_str(&format!("\n// --- {title} ---\n"));
        for declaration in declarations {
            out.push('\n');
            out.push_str(&declaration.source);
            out.push('\n');
        }
    }
    out
}

fn bindings_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(BINDINGS_FILE)
}

/// Line endings normalised: a checkout may turn LF into CRLF.
fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n")
}

#[test]
fn bindings_are_up_to_date() {
    let expected = render(&declarations(&config()));
    let path = bindings_path();
    let actual = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {BINDINGS_FILE}: {e}"));
    assert!(normalize(&actual) == expected, "{BINDINGS_FILE} is out of date with the Rust DTOs: run `{REGENERATE}` and commit the result");
}

#[test]
fn every_dependency_is_declared() {
    let sections = declarations(&config());
    let all: Vec<&Declaration> = sections.iter().flat_map(|(_, d)| d).collect();
    let declared: BTreeSet<TypeId> = all.iter().map(|d| d.type_id).collect();
    for declaration in &all {
        for dependency in &declaration.dependencies {
            assert!(declared.contains(dependency), "`{}` refers to a type missing from `declarations`", declaration.name);
        }
    }
}

#[test]
fn names_are_unique() {
    let sections = declarations(&config());
    let mut seen = BTreeSet::new();
    for declaration in sections.iter().flat_map(|(_, d)| d) {
        assert!(seen.insert(declaration.name.clone()), "`{}` is declared twice", declaration.name);
    }
}

/// Writes [`BINDINGS_FILE`]; run on purpose (`-- --ignored`), never by a plain `cargo test`.
#[test]
#[ignore = "regenerates the TypeScript bindings: run with `-- --ignored`"]
fn write_bindings() {
    let path = bindings_path();
    std::fs::write(&path, render(&declarations(&config()))).unwrap_or_else(|e| panic!("cannot write {BINDINGS_FILE}: {e}"));
}
