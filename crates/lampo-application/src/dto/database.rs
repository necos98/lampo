use serde::{Deserialize, Serialize};

/// Connection details shown to the user/agent. Lampo never writes them to `.env`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseConnectionDto {
    pub mysql_version: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
}

/// `lampo db list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseListDto {
    pub mysql_version: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub databases: Vec<String>,
    /// The database the resolved site uses.
    pub site_database: Option<String>,
}

/// Targets the MySQL instance of `site` (`shop`, `shop@fix-1`; `null` = resolve from cwd on
/// the CLI side before sending).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseListRequest {
    pub site: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseRequest {
    pub site: String,
    /// Default: the site's database.
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseFileRequest {
    pub site: String,
    pub name: Option<String>,
    /// SQL file to read (import) or write (export).
    pub file: String,
}

/// What happened to a new project's database (`CreateProjectResultDto::database_setup`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DatabaseSetupStatusDto {
    Created,
    /// It existed already (e.g. a folder reusing its database): left untouched.
    AlreadyExists,
    /// The MySQL package of the project's series is not installed: nothing was attempted.
    PackageMissing,
    /// MySQL did not start or refused: see `message`. The project itself was created.
    Failed,
}

impl DatabaseSetupStatusDto {
    /// `true` when front-ends should warn (the database does not exist).
    pub fn is_problem(self) -> bool {
        matches!(self, Self::PackageMissing | Self::Failed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseSetupDto {
    /// The project's database name.
    pub name: String,
    pub status: DatabaseSetupStatusDto,
    /// Why, for `package-missing` and `failed`.
    pub message: Option<String>,
}

/// Where a site's mail goes (the shared Mailpit), shown like the database connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailConnectionDto {
    pub host: String,
    /// Plain SMTP: no authentication needed (any credentials are accepted), no encryption.
    pub smtp_port: u16,
    /// The web UI showing the caught messages (`http://127.0.0.1:8025`).
    pub web_url: String,
}
