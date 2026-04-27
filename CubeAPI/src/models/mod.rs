// Copyright (c) 2024 Tencent Inc.
// SPDX-License-Identifier: Apache-2.0
//

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use validator::Validate;

// ─── Common ────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub code: i32,
    pub message: String,
}

impl ApiError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

// ─── Sandbox shared types ──────────────────────────────────────────────────

pub type SandboxMetadata = HashMap<String, String>;
pub type EnvVars = HashMap<String, String>;

/// State of the sandbox (running | paused)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SandboxState {
    Running,
    Paused,
}

/// Network configuration for sandbox egress/ingress control.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SandboxNetworkConfig {
    #[serde(rename = "allowPublicTraffic", skip_serializing_if = "Option::is_none")]
    pub allow_public_traffic: Option<bool>,
    #[serde(rename = "allowOut", skip_serializing_if = "Option::is_none")]
    pub allow_out: Option<Vec<String>>,
    #[serde(rename = "denyOut", skip_serializing_if = "Option::is_none")]
    pub deny_out: Option<Vec<String>>,
    #[serde(rename = "maskRequestHost", skip_serializing_if = "Option::is_none")]
    pub mask_request_host: Option<String>,
}

/// Auto-resume configuration for paused sandboxes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxAutoResumeConfig {
    pub enabled: bool,
}

/// Volume mount inside the sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxVolumeMount {
    pub name: String,
    pub path: String,
}

// ─── Sandbox — create request ──────────────────────────────────────────────

/// Request body for POST /sandboxes
/// Field names match exactly what the E2B SDK sends.
/// Rule: ID abbreviations → uppercase (templateID, sandboxID, envVars, autoPause);
///       allow_internet_access is a known SDK snake_case quirk.
#[derive(Debug, Deserialize, Validate)]
#[allow(dead_code)]
pub struct NewSandbox {
    #[serde(rename = "templateID")]
    pub template_id: String,

    #[validate(range(min = 0))]
    #[serde(default = "default_timeout")]
    pub timeout: i32,

    #[serde(rename = "autoPause", default)]
    pub auto_pause: bool,

    #[serde(rename = "autoResume", skip_serializing_if = "Option::is_none")]
    pub auto_resume: Option<SandboxAutoResumeConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub secure: Option<bool>,

    /// SDK sends this as snake_case (known quirk).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_internet_access: Option<bool>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<SandboxNetworkConfig>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SandboxMetadata>,

    #[serde(rename = "envVars", skip_serializing_if = "Option::is_none")]
    pub env_vars: Option<EnvVars>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp: Option<serde_json::Value>,

    #[serde(rename = "volumeMounts", skip_serializing_if = "Option::is_none")]
    pub volume_mounts: Option<Vec<SandboxVolumeMount>>,

    /// Fork-local extension: when present, the new sandbox is restored
    /// from the supplied on-disk snapshot bundle instead of cold-booted
    /// from the template. The orchestrator persists snapshot metadata
    /// (path, host_ip) returned by `POST /sandboxes/{id}/snapshots` and
    /// supplies it back here.
    #[serde(rename = "fromSnapshot", skip_serializing_if = "Option::is_none")]
    pub from_snapshot: Option<FromSnapshot>,
}

/// Snapshot reference for restore-on-create.
#[derive(Debug, Deserialize, Clone)]
pub struct FromSnapshot {
    pub path: String,
}

/// Query string for `DELETE /sandboxes/snapshots/{snapshotID}`. CubeMaster
/// keeps no snapshot index, so the orchestrator must hand back the host
/// that owns the blob (returned in the create response as `hostIP`).
#[derive(Debug, Deserialize)]
pub struct DeleteSnapshotQuery {
    #[serde(rename = "hostIP", alias = "host_ip")]
    pub host_ip: String,
}

fn default_timeout() -> i32 {
    15
}

// ─── Sandbox — create / connect response ──────────────────────────────────

/// Response for POST /sandboxes and POST /sandboxes/{id}/connect.
/// All ID abbreviations uppercase per E2B OpenAPI spec.
#[derive(Debug, Serialize, Deserialize)]
pub struct Sandbox {
    #[serde(rename = "templateID")]
    pub template_id: String,

    #[serde(rename = "sandboxID")]
    pub sandbox_id: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,

    #[serde(rename = "clientID")]
    pub client_id: String,

    #[serde(rename = "envdVersion")]
    pub envd_version: String,

    #[serde(rename = "envdAccessToken", skip_serializing_if = "Option::is_none")]
    pub envd_access_token: Option<String>,

    #[serde(rename = "trafficAccessToken", skip_serializing_if = "Option::is_none")]
    pub traffic_access_token: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

// ─── Sandbox — list / detail responses ────────────────────────────────────

/// One entry in GET /sandboxes (RunningSandbox in OpenAPI spec).
#[derive(Debug, Serialize, Deserialize)]
pub struct ListedSandbox {
    #[serde(rename = "templateID")]
    pub template_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(rename = "sandboxID")]
    pub sandbox_id: String,
    #[serde(rename = "clientID")]
    pub client_id: String,
    #[serde(rename = "startedAt")]
    pub started_at: DateTime<Utc>,
    #[serde(rename = "endAt")]
    pub end_at: DateTime<Utc>,
    #[serde(rename = "cpuCount")]
    pub cpu_count: i32,
    #[serde(rename = "memoryMB")]
    pub memory_mb: i32,
    #[serde(rename = "diskSizeMB", skip_serializing_if = "Option::is_none")]
    pub disk_size_mb: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SandboxMetadata>,
    pub state: SandboxState,
    #[serde(rename = "envdVersion")]
    pub envd_version: String,
    #[serde(rename = "volumeMounts", skip_serializing_if = "Option::is_none")]
    pub volume_mounts: Option<Vec<SandboxVolumeMount>>,
}

/// Detailed sandbox info returned by GET /sandboxes/{sandboxID}.
#[derive(Debug, Serialize, Deserialize)]
pub struct SandboxDetail {
    #[serde(rename = "templateID")]
    pub template_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(rename = "sandboxID")]
    pub sandbox_id: String,
    #[serde(rename = "clientID")]
    pub client_id: String,
    #[serde(rename = "startedAt")]
    pub started_at: DateTime<Utc>,
    #[serde(rename = "endAt")]
    pub end_at: DateTime<Utc>,
    #[serde(rename = "envdVersion")]
    pub envd_version: String,
    #[serde(rename = "envdAccessToken", skip_serializing_if = "Option::is_none")]
    pub envd_access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(rename = "cpuCount")]
    pub cpu_count: i32,
    #[serde(rename = "memoryMB")]
    pub memory_mb: i32,
    #[serde(rename = "diskSizeMB", skip_serializing_if = "Option::is_none")]
    pub disk_size_mb: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SandboxMetadata>,
    pub state: SandboxState,
    #[serde(rename = "volumeMounts", skip_serializing_if = "Option::is_none")]
    pub volume_mounts: Option<Vec<SandboxVolumeMount>>,
}

// ─── Sandbox — pause/resume/connect/snapshot ──────────────────────────────

/// Request body for POST /sandboxes/{id}/resume (deprecated).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ResumedSandbox {
    #[serde(default = "default_timeout")]
    pub timeout: i32,
    #[serde(rename = "autoPause", default)]
    pub auto_pause: bool,
}

/// Request body for POST /sandboxes/{id}/connect.
#[derive(Debug, Deserialize, Validate)]
pub struct ConnectSandbox {
    #[validate(range(min = 0))]
    pub timeout: i32,
}

/// Request body for POST /sandboxes/{id}/snapshots.
#[derive(Debug, Deserialize)]
pub struct CreateSnapshotRequest {
    pub name: Option<String>,
}

/// Response for POST /sandboxes/{id}/snapshots.
#[derive(Debug, Serialize)]
pub struct SnapshotInfo {
    #[serde(rename = "snapshotID")]
    pub snapshot_id: String,
    pub names: Vec<String>,
    /// On-disk directory holding the snapshot bundle. Returned so the
    /// orchestrator can persist its own (snapshot_id -> path) mapping
    /// without a follow-up query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// IP of the node that owns the blob. Required when the
    /// orchestrator later asks to delete the snapshot.
    #[serde(rename = "hostIP", skip_serializing_if = "Option::is_none")]
    pub host_ip: Option<String>,
}

// ─── Sandbox — logs ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// Single raw log line — matches E2B SandboxLog schema (timestamp + line).
#[derive(Debug, Serialize, Deserialize)]
pub struct SandboxLog {
    pub timestamp: DateTime<Utc>,
    pub line: String,
}

/// Structured log entry (v2 logs).
#[derive(Debug, Serialize, Deserialize)]
pub struct SandboxLogEntry {
    pub timestamp: DateTime<Utc>,
    pub message: String,
    pub level: LogLevel,
    pub fields: HashMap<String, String>,
}

/// Legacy log response — matches E2B SandboxLogs schema.
#[derive(Debug, Serialize, Deserialize)]
pub struct SandboxLogs {
    pub logs: Vec<SandboxLog>,
    #[serde(rename = "logEntries")]
    pub log_entries: Vec<SandboxLogEntry>,
}

/// v2 log response.
#[derive(Debug, Serialize, Deserialize)]
pub struct SandboxLogsV2Response {
    pub logs: Vec<SandboxLogEntry>,
}

/// Query params for v1 sandbox logs.
#[derive(Debug, Deserialize)]
pub struct SandboxLogsQuery {
    pub start: Option<i64>,
    #[serde(default = "default_log_limit")]
    pub limit: i32,
}

/// Query params for v2 sandbox logs.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SandboxLogsV2Query {
    pub cursor: Option<i64>,
    #[serde(default = "default_log_limit")]
    pub limit: i32,
    pub direction: Option<String>,
}

fn default_log_limit() -> i32 {
    1000
}

// ─── Sandbox — timeout / refresh ──────────────────────────────────────────

/// Request body for POST /sandboxes/{id}/timeout
#[derive(Debug, Deserialize, Validate)]
pub struct SetTimeoutRequest {
    #[validate(range(min = 0))]
    pub timeout: i32,
}

/// Request body for POST /sandboxes/{id}/refreshes
#[derive(Debug, Deserialize, Validate)]
pub struct RefreshRequest {
    #[validate(range(min = 0, max = 3600))]
    pub duration: Option<i32>,
}

// ─── Sandbox — list query ──────────────────────────────────────────────────

/// Query params for GET /sandboxes.
#[derive(Debug, Deserialize)]
pub struct ListSandboxesQuery {
    pub metadata: Option<String>,
}

/// Query params for GET /v2/sandboxes.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ListSandboxesV2Query {
    pub metadata: Option<String>,
    pub state: Option<String>,
    #[serde(rename = "nextToken")]
    pub next_token: Option<String>,
    #[serde(default = "default_page_limit")]
    pub limit: i32,
}

fn default_page_limit() -> i32 {
    100
}
