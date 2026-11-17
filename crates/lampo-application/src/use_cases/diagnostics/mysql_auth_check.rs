//! `mysql-auth`: projects whose PHP cannot use their MySQL series' default authentication (PHP
//! before 7.4 on MySQL 8.4, see [`lampo_domain::mysql::auth`]). Lampo moves their account to
//! `mysql_native_password`, so this is a `Warning`, with MySQL 5.7 as the alternative.

use lampo_domain::mysql::MysqlSeries;
use lampo_domain::mysql::auth::legacy_php_auth_series;

use super::{DoctorCheck, DoctorFacts, check};
use crate::dto::{CheckStatus, DoctorCheckDto, DoctorReportDto};

const ID: &str = "mysql-auth";

pub struct MysqlAuthCheck;

impl DoctorCheck for MysqlAuthCheck {
    fn run(&self, facts: &DoctorFacts, _report: &mut DoctorReportDto) -> DoctorCheckDto {
        let affected: Vec<String> = facts
            .project_files
            .projects
            .iter()
            .filter_map(|p| legacy_php_auth_series(&p.environment).map(|series| format!("{} (PHP {}, MySQL {series})", p.id, p.environment.php)))
            .collect();
        if affected.is_empty() {
            return check(ID, CheckStatus::Ok, "every project's PHP can authenticate to its MySQL", None);
        }
        let legacy = MysqlSeries::LEGACY_5_7;
        check(
            ID,
            CheckStatus::Warning,
            format!(
                "PHP before 7.4 cannot use MySQL 8's default authentication (caching_sha2_password); Lampo moves the MySQL account of these projects to mysql_native_password: {}",
                affected.join(", ")
            ),
            Some(format!("if a connection still fails (error 2054), move the project to MySQL {legacy}: lampo project set <project> --service mysql@{legacy}")),
        )
    }
}
