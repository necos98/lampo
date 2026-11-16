use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "AppInfo"))]
#[serde(rename_all = "camelCase")]
pub struct AppInfoDto {
    pub version: String,
    pub config_dir: String,
    pub data_dir: String,
    pub shim_dir: String,
    pub control_protocol_version: u32,
}
