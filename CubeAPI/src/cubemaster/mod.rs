// Copyright (c) 2024 Tencent Inc.
// SPDX-License-Identifier: Apache-2.0
//

/// cubemaster — thin HTTP client wrapping CubeMaster REST API.
///
/// Existing APIs (✅ implemented on CubeMaster):
///   - POST   /cube/sandbox          create sandbox
///   - DELETE /cube/sandbox          delete sandbox
///   - POST   /cube/sandbox/list     list sandboxes
///
/// Implemented on CubeMaster (see pkg/service/sandbox/types):
///   - GET    /cube/sandbox/info       get single sandbox detail (query: sandbox_id, instance_type)
///   - POST   /cube/sandbox/update     update sandbox (action: "pause" | "resume")
/// New APIs required (❌ not yet on CubeMaster — see docs/cubemaster-api-requirements.md):
///   - POST   /cube/sandbox/timeout    set absolute TTL
///   - POST   /cube/sandbox/refresh    extend TTL by delta
///   - POST   /cube/sandbox/logs       fetch sandbox logs
///   - POST   /cube/sandbox/snapshot   create sandbox snapshot
///   - POST   /cube/sandbox/commit     commit sandbox → template image
///   - GET    /cube/template/build/{id}/status  build status poll
///   - DELETE /cube/template           delete template
///   - POST   /cube/template/list      list templates
///   - GET    /cube/template/{id}      get single template
use chrono::{DateTime, Utc};
use serde::de::Deserializer;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Client ────────────────────────────────────────────────────────────────

/// Lightweight wrapper around a shared `reqwest::Client`.
/// Clone is O(1) — the inner client holds an `Arc` to the connection pool.
#[derive(Clone)]
pub struct CubeMasterClient {
    inner: reqwest::Client,
    base_url: String,
}

impl CubeMasterClient {
    /// Create a client pointing at `base_url` (e.g. `"http://10.0.0.1:8080"`).
    pub fn new(base_url: impl Into<String>, http_client: reqwest::Client) -> Self {
        Self {
            inner: http_client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        }
    }

    // ── Sandbox: existing APIs ─────────────────────────────────────────────

    /// POST /cube/sandbox — create a new sandbox.
    pub async fn create_sandbox(
        &self,
        req: &CreateSandboxRequest,
    ) -> Result<CreateSandboxResponse, CubeMasterError> {
        let url = format!("{}/cube/sandbox", self.base_url);
        let resp = self
            .inner
            .post(&url)
            .json(req)
            .send()
            .await
            .map_err(CubeMasterError::Http)?;
        parse_response(resp).await
    }

    /// DELETE /cube/sandbox — destroy a sandbox.
    pub async fn delete_sandbox(
        &self,
        req: &DeleteSandboxRequest,
    ) -> Result<DeleteSandboxResponse, CubeMasterError> {
        let url = format!("{}/cube/sandbox", self.base_url);
        let resp = self
            .inner
            .delete(&url)
            .json(req)
            .send()
            .await
            .map_err(CubeMasterError::Http)?;
        parse_response(resp).await
    }

    /// POST /cube/sandbox/list — list sandboxes (paginated by host).
    pub async fn list_sandboxes(
        &self,
        req: &ListSandboxRequest,
    ) -> Result<ListSandboxResponse, CubeMasterError> {
        let url = format!("{}/cube/sandbox/list", self.base_url);
        let resp = self
            .inner
            .post(&url)
            .json(req)
            .send()
            .await
            .map_err(CubeMasterError::Http)?;
        parse_response(resp).await
    }

    // ── Sandbox: new APIs (require CubeMaster implementation) ─────────────
    // See docs/cubemaster-api-requirements.md §1 for full specs.

    /// GET /cube/sandbox/info — fetch a single sandbox's real-time status.
    /// Query: sandbox_id, instance_type. See CubeMaster pkg/service/sandbox/types.
    pub async fn get_sandbox(
        &self,
        sandbox_id: &str,
        instance_type: &str,
    ) -> Result<GetSandboxResponse, CubeMasterError> {
        let url = format!("{}/cube/sandbox/info", self.base_url);
        let resp = self
            .inner
            .get(&url)
            .query(&[("sandbox_id", sandbox_id), ("instance_type", instance_type)])
            .send()
            .await
            .map_err(CubeMasterError::Http)?;
        parse_response(resp).await
    }

    /// POST /cube/sandbox/update — pause or resume a sandbox (action: "pause" | "resume").
    pub async fn update_sandbox(
        &self,
        req: &SandboxUpdateRequest,
    ) -> Result<SandboxUpdateResponse, CubeMasterError> {
        let url = format!("{}/cube/sandbox/update", self.base_url);
        let resp = self
            .inner
            .post(&url)
            .json(req)
            .send()
            .await
            .map_err(CubeMasterError::Http)?;
        parse_response(resp).await
    }

    /// POST /cube/sandbox/timeout — set absolute TTL for a sandbox.
    /// ❌ New API required on CubeMaster.
    pub async fn set_sandbox_timeout(
        &self,
        req: &SandboxTimeoutRequest,
    ) -> Result<SandboxTimeoutResponse, CubeMasterError> {
        let url = format!("{}/cube/sandbox/timeout", self.base_url);
        let resp = self
            .inner
            .post(&url)
            .json(req)
            .send()
            .await
            .map_err(CubeMasterError::Http)?;
        parse_response(resp).await
    }

    /// POST /cube/sandbox/refresh — extend TTL by a delta (seconds).
    /// ❌ New API required on CubeMaster.
    pub async fn refresh_sandbox(
        &self,
        req: &SandboxRefreshRequest,
    ) -> Result<SandboxRefreshResponse, CubeMasterError> {
        let url = format!("{}/cube/sandbox/refresh", self.base_url);
        let resp = self
            .inner
            .post(&url)
            .json(req)
            .send()
            .await
            .map_err(CubeMasterError::Http)?;
        parse_response(resp).await
    }

    /// POST /cube/sandbox/logs — fetch sandbox stdout/stderr logs.
    /// ❌ New API required on CubeMaster.
    pub async fn get_sandbox_logs(
        &self,
        req: &SandboxLogsRequest,
    ) -> Result<SandboxLogsResponse, CubeMasterError> {
        let url = format!("{}/cube/sandbox/logs", self.base_url);
        let resp = self
            .inner
            .post(&url)
            .json(req)
            .send()
            .await
            .map_err(CubeMasterError::Http)?;
        parse_response(resp).await
    }

    /// POST /cube/sandbox/snapshot — create a named runtime snapshot.
    pub async fn create_sandbox_snapshot(
        &self,
        req: &SandboxSnapshotRequest,
    ) -> Result<SandboxSnapshotResponse, CubeMasterError> {
        let url = format!("{}/cube/sandbox/snapshot", self.base_url);
        let resp = self
            .inner
            .post(&url)
            .json(req)
            .send()
            .await
            .map_err(CubeMasterError::Http)?;
        parse_response(resp).await
    }

    /// POST /cube/sandbox/snapshot/delete — drop a runtime snapshot
    /// blob from the host that owns it.
    pub async fn delete_sandbox_snapshot(
        &self,
        req: &SandboxSnapshotDeleteRequest,
    ) -> Result<SandboxSnapshotDeleteResponse, CubeMasterError> {
        let url = format!("{}/cube/sandbox/snapshot/delete", self.base_url);
        let resp = self
            .inner
            .post(&url)
            .json(req)
            .send()
            .await
            .map_err(CubeMasterError::Http)?;
        parse_response(resp).await
    }
    }

// ─── Error ─────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum CubeMasterError {
    #[error("HTTP transport error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("CubeMaster returned error code {ret_code}: {ret_msg}")]
    Api { ret_code: i32, ret_msg: String },

    #[error("failed to deserialise CubeMaster response: {0}")]
    Deserialize(String),
}

impl CubeMasterError {
    /// True when CubeMaster returned 404 / 130404 (not found).
    pub fn is_not_found(&self) -> bool {
        match self {
            Self::Api { ret_code, .. } => *ret_code == 130404,
            _ => false,
        }
    }

    /// True when CubeMaster returned 130409 (conflict / wrong state).
    pub fn is_conflict(&self) -> bool {
        match self {
            Self::Api { ret_code, .. } => *ret_code == 130409,
            _ => false,
        }
    }

    /// True when CubeMaster doesn't have the endpoint yet (HTTP 404 on the path).
    pub fn is_endpoint_missing(&self) -> bool {
        match self {
            Self::Api { ret_code, ret_msg } => {
                *ret_code == 404 || ret_msg.to_lowercase().contains("not found")
            }
            Self::Http(e) => e.status().map_or(false, |s| s == 404),
            _ => false,
        }
    }
}

// ─── Common response envelope ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RetCode {
    pub ret_code: i32,
    pub ret_msg: String,
}

impl RetCode {
    pub fn into_result(self) -> Result<(), CubeMasterError> {
        if self.ret_code == 0 || self.ret_code == 200 {
            Ok(())
        } else {
            Err(CubeMasterError::Api {
                ret_code: self.ret_code,
                ret_msg: self.ret_msg,
            })
        }
    }
}

// ─── Sandbox status enum ───────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SandboxStatus {
    Running,
    Paused,
    Pausing,
    Stopped,
    Error,
    #[serde(other)]
    Unknown,
}

// ─── Create sandbox ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct CreateSandboxRequest {
    #[serde(rename = "RequestID", alias = "requestID")]
    pub request_id: String,

    pub instance_type: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i32>,

    pub containers: Vec<ContainerSpec>,
    pub annotations: HashMap<String, String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<HashMap<String, String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub volumes: Option<Vec<VolumeSpec>>,

    /// Port numbers to expose from the sandbox (e.g. [8888, 49999]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exposed_ports: Vec<u16>,

    /// Network mode: "tap" | "bridge" | ""
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_type: Option<String>,

    /// CubeVS network policy (egress control).
    #[serde(rename = "cubevs_context", skip_serializing_if = "Option::is_none")]
    pub cubevs_context: Option<CubeVSContext>,
}

/// CubeVS network egress control, maps to CubeMaster's CubeVSContext.
#[derive(Debug, Serialize, Clone, Default)]
pub struct CubeVSContext {
    /// Allow internet (public) access. Maps to CubeMaster allowInternetAccess.
    #[serde(rename = "allowInternetAccess", skip_serializing_if = "Option::is_none")]
    pub allow_internet_access: Option<bool>,

    /// Allowed outbound CIDRs whitelist.
    #[serde(rename = "allowOut", skip_serializing_if = "Vec::is_empty")]
    pub allow_out: Vec<String>,

    /// Denied outbound CIDRs blacklist.
    #[serde(rename = "denyOut", skip_serializing_if = "Vec::is_empty")]
    pub deny_out: Vec<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ContainerSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub image: ImageSpec,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<ResourceSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envs: Option<Vec<EnvVar>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_mounts: Option<Vec<VolumeMount>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns_config: Option<DnsConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r_limit: Option<RLimit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_context: Option<SecurityContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe: Option<Probe>,
    /// Per-container annotations (separate from top-level sandbox annotations).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ImageSpec {
    pub image: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_media: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ResourceSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem: Option<String>,
}

/// EnvVar uses `key` (not `name`) per the CubeMaster JSON schema.
#[derive(Debug, Serialize, Clone)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct VolumeMount {
    pub name: String,
    pub container_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readonly: Option<bool>,
}

#[derive(Debug, Serialize, Clone)]
pub struct VolumeSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_source: Option<VolumeSource>,
}

#[derive(Debug, Serialize, Clone)]
pub struct VolumeSource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub empty_dir: Option<EmptyDir>,
}

#[derive(Debug, Serialize, Clone)]
pub struct EmptyDir {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_limit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub medium: Option<i32>,
}

/// DNS configuration injected into the container's resolv.conf.
#[derive(Debug, Serialize, Clone)]
pub struct DnsConfig {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub servers: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub searches: Vec<String>,
}

/// Resource limit overrides (ulimit).
#[derive(Debug, Serialize, Clone)]
pub struct RLimit {
    /// RLIMIT_NOFILE — max open file descriptors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_file: Option<u64>,
    /// RLIMIT_NPROC — max child processes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_proc: Option<u64>,
}

/// Container security context.
#[derive(Debug, Serialize, Clone)]
pub struct SecurityContext {
    /// Run container as root with full privileges.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privileged: Option<bool>,
    /// UID to run the container process as.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_as_user: Option<i64>,
}

/// Readiness / liveness probe configuration.
#[derive(Debug, Serialize, Clone)]
pub struct Probe {
    pub probe_handler: ProbeHandler,
    /// Probe timeout in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// How often to probe (ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_ms: Option<u64>,
    /// Min consecutive successes to be considered healthy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success_threshold: Option<u32>,
    /// Max failures before the sandbox is considered unhealthy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_threshold: Option<u32>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ProbeHandler {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_get: Option<HttpGetAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exec: Option<ExecAction>,
}

#[derive(Debug, Serialize, Clone)]
pub struct HttpGetAction {
    pub path: String,
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct ExecAction {
    pub command: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSandboxResponse {
    #[serde(rename = "RequestID", alias = "requestID")]
    pub request_id: String,
    #[serde(default)]
    pub sandbox_id: String,
    pub ret: RetCode,
}

// ─── Delete sandbox ────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct DeleteSandboxRequest {
    #[serde(rename = "RequestID", alias = "requestID")]
    pub request_id: String,
    pub sandbox_id: String,
    pub instance_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<DeleteFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub struct DeleteFilter {
    pub label_selector: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct DeleteSandboxResponse {
    #[serde(rename = "RequestID", alias = "requestID")]
    pub request_id: String,
    #[serde(default)]
    pub sandbox_id: String,
    pub ret: RetCode,
}

// ─── List sandboxes ────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct ListSandboxRequest {
    #[serde(rename = "RequestID", alias = "requestID")]
    pub request_id: String,
    pub instance_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_idx: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<ListFilter>,
}

#[derive(Debug, Serialize)]
pub struct ListFilter {
    pub label_selector: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ListSandboxResponse {
    #[serde(rename = "RequestID", alias = "requestID")]
    pub request_id: String,
    #[serde(default, alias = "data")]
    pub sandboxes: Vec<SandboxInfo>,
    pub ret: RetCode,
}

/// One sandbox entry as returned by /cube/sandbox/list.
#[derive(Debug, Deserialize)]
pub struct SandboxInfo {
    pub sandbox_id: String,
    #[serde(default)]
    pub host_id: String,
    #[serde(default, deserialize_with = "deserialize_sandbox_status")]
    pub status: String,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub end_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub cpu_count: i32,
    #[serde(default)]
    pub memory_mb: i32,
    #[serde(default)]
    pub template_id: String,
    #[serde(default)]
    pub annotations: HashMap<String, String>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

// ─── Get single sandbox ────────────────────────────────────────────────────
// CubeMaster GET /cube/sandbox/info returns: { requestID, ret, data: [{ sandbox_id, status, containers, namespace }] }

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GetSandboxResponse {
    #[serde(rename = "RequestID", alias = "requestID")]
    pub request_id: String,
    #[serde(default)]
    pub data: Vec<GetSandboxDataItem>,
    pub ret: RetCode,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GetSandboxDataItem {
    #[serde(default)]
    pub sandbox_id: String,
    #[serde(default)]
    pub status: i32,
    #[serde(default)]
    pub template_id: String,
    #[serde(default)]
    pub annotations: HashMap<String, String>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub containers: Vec<GetSandboxContainerItem>,
    #[serde(default)]
    pub namespace: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct GetSandboxContainerItem {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub container_id: String,
    #[serde(default)]
    pub image: String,
    #[serde(default)]
    pub cpu: String,
    #[serde(default)]
    pub mem: String,
}

/// Normalized sandbox detail used by handlers (built from GetSandboxDataItem).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SandboxDetail {
    pub sandbox_id: String,
    pub host_id: String,
    pub instance_type: String,
    pub status: SandboxStatus,
    pub template_id: String,
    pub started_at: Option<DateTime<Utc>>,
    pub end_at: Option<DateTime<Utc>>,
    pub cpu_count: i32,
    pub memory_mb: i32,
    pub disk_size_mb: i32,
    pub annotations: HashMap<String, String>,
    pub labels: HashMap<String, String>,
}

fn parse_cpu_millicores(s: &str) -> i32 {
    let s = s.trim().trim_end_matches('m');
    s.parse::<i32>().unwrap_or(0) / 1000
}

fn parse_mem_mb(s: &str) -> i32 {
    let s = s
        .trim()
        .trim_end_matches("Mi")
        .trim_end_matches("MB")
        .trim_end_matches('M');
    s.parse::<i32>().unwrap_or(0)
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SandboxStatusValue {
    Text(String),
    Number(i32),
}

fn deserialize_sandbox_status<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<SandboxStatusValue>::deserialize(deserializer)?;
    Ok(match value {
        Some(SandboxStatusValue::Text(text)) => normalize_sandbox_status_text(&text),
        Some(SandboxStatusValue::Number(number)) => match number {
            1 => "running".to_string(),
            2 => "paused".to_string(),
            3 => "stopped".to_string(),
            4 => "error".to_string(),
            _ => "unknown".to_string(),
        },
        None => String::new(),
    })
}

fn normalize_sandbox_status_text(raw: &str) -> String {
    match raw.trim().to_lowercase().as_str() {
        "1" | "running" => "running".to_string(),
        "2" | "paused" => "paused".to_string(),
        "3" | "stopped" => "stopped".to_string(),
        "4" | "error" => "error".to_string(),
        other => other.to_string(),
    }
}

fn extract_template_id(
    explicit_template_id: &str,
    annotations: &HashMap<String, String>,
    labels: &HashMap<String, String>,
) -> String {
    if !explicit_template_id.trim().is_empty() {
        return explicit_template_id.to_string();
    }
    annotations
        .get("cube.master.appsnapshot.template.id")
        .cloned()
        .or_else(|| labels.get("cube.master.appsnapshot.template.id").cloned())
        .unwrap_or_default()
}

impl GetSandboxResponse {
    /// Take the first item from `data` and convert to SandboxDetail. Returns None if data is empty.
    pub fn into_first_sandbox(self, instance_type: &str) -> Option<SandboxDetail> {
        let item = self.data.into_iter().next()?;
        let (cpu_count, memory_mb) = item
            .containers
            .first()
            .map(|c| (parse_cpu_millicores(&c.cpu), parse_mem_mb(&c.mem)))
            .unwrap_or((0, 0));
        let status = match item.status {
            0 => SandboxStatus::Unknown,   // CONTAINER_CREATED
            1 => SandboxStatus::Running,   // CONTAINER_RUNNING
            2 => SandboxStatus::Stopped,   // CONTAINER_EXITED
            3 => SandboxStatus::Unknown,   // CONTAINER_UNKNOWN
            4 => SandboxStatus::Pausing,   // CONTAINER_PAUSING
            5 => SandboxStatus::Paused,    // CONTAINER_PAUSED
            _ => SandboxStatus::Unknown,
        };
        let template_id = extract_template_id(&item.template_id, &item.annotations, &item.labels);
        let sid = item.sandbox_id;
        Some(SandboxDetail {
            sandbox_id: sid.clone(),
            host_id: sid,
            instance_type: instance_type.to_string(),
            status,
            template_id,
            started_at: None,
            end_at: None,
            cpu_count,
            memory_mb,
            disk_size_mb: 0,
            annotations: item.annotations,
            labels: item.labels,
        })
    }
}

impl Default for SandboxStatus {
    fn default() -> Self {
        SandboxStatus::Unknown
    }
}

// ─── Update sandbox (pause / resume) ───────────────────────────────────────
// CubeMaster POST /cube/sandbox/update, action: "pause" | "resume"

#[derive(Debug, Serialize)]
pub struct SandboxUpdateRequest {
    #[serde(rename = "requestID")]
    pub request_id: String,
    #[serde(rename = "sandbox_id")]
    pub sandbox_id: String,
    #[serde(rename = "instance_type")]
    pub instance_type: String,
    /// "pause" | "resume"
    #[serde(rename = "action")]
    pub action: String,
    /// TTL in seconds (for resume; 0 = keep original). Optional for pause.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct SandboxUpdateResponse {
    pub ret: RetCode,
}

// ─── Set sandbox timeout (absolute) ───────────────────────────────────────
// ❌ New API — see docs/cubemaster-api-requirements.md §1.4

#[derive(Debug, Serialize)]
pub struct SandboxTimeoutRequest {
    #[serde(rename = "RequestID", alias = "requestID")]
    pub request_id: String,
    #[serde(rename = "sandboxID")]
    pub sandbox_id: String,
    #[serde(rename = "instanceType")]
    pub instance_type: String,
    /// New TTL in seconds from now.
    pub timeout: i32,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SandboxTimeoutResponse {
    #[serde(rename = "RequestID", alias = "requestID")]
    pub request_id: String,
    #[serde(rename = "sandboxID", default)]
    pub sandbox_id: String,
    pub end_at: Option<DateTime<Utc>>,
    pub ret: RetCode,
}

// ─── Refresh sandbox TTL (relative extend) ────────────────────────────────
// ❌ New API — see docs/cubemaster-api-requirements.md §1.5

#[derive(Debug, Serialize)]
pub struct SandboxRefreshRequest {
    #[serde(rename = "RequestID", alias = "requestID")]
    pub request_id: String,
    #[serde(rename = "sandboxID")]
    pub sandbox_id: String,
    #[serde(rename = "instanceType")]
    pub instance_type: String,
    /// Seconds to add onto the current endAt.
    pub duration: i32,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SandboxRefreshResponse {
    #[serde(rename = "RequestID", alias = "requestID")]
    pub request_id: String,
    #[serde(rename = "sandboxID", default)]
    pub sandbox_id: String,
    pub end_at: Option<DateTime<Utc>>,
    pub ret: RetCode,
}

// ─── Sandbox logs ──────────────────────────────────────────────────────────
// ❌ New API — see docs/cubemaster-api-requirements.md §1.6

#[derive(Debug, Serialize)]
pub struct SandboxLogsRequest {
    #[serde(rename = "RequestID", alias = "requestID")]
    pub request_id: String,
    #[serde(rename = "sandboxID")]
    pub sandbox_id: String,
    #[serde(rename = "instanceType")]
    pub instance_type: String,
    /// Unix-millisecond cursor — only return logs after this timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<i64>,
    /// Max log lines to return (default 1000, max 5000).
    pub limit: i32,
    /// "stdout" | "stderr" | "all"
    pub source: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SandboxLogsResponse {
    #[serde(rename = "RequestID", alias = "requestID")]
    pub request_id: String,
    #[serde(default)]
    pub logs: Vec<SandboxLogLine>,
    pub next_cursor: Option<i64>,
    #[serde(default)]
    pub has_more: bool,
    pub ret: RetCode,
}

#[derive(Debug, Deserialize)]
pub struct SandboxLogLine {
    pub timestamp: DateTime<Utc>,
    pub line: String,
    #[serde(default)]
    pub source: String, // "stdout" | "stderr"
}

// ─── Sandbox snapshot ──────────────────────────────────────────────────────
// ❌ New API — see docs/cubemaster-api-requirements.md §1.7

#[derive(Debug, Serialize)]
pub struct SandboxSnapshotRequest {
    #[serde(rename = "RequestID", alias = "requestID")]
    pub request_id: String,
    #[serde(rename = "sandboxID")]
    pub sandbox_id: String,
    #[serde(rename = "instanceType")]
    pub instance_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
    pub sync: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SandboxSnapshotResponse {
    #[serde(rename = "RequestID", alias = "requestID", default)]
    pub request_id: String,
    #[serde(rename = "sandboxID", default)]
    pub sandbox_id: String,
    #[serde(default)]
    pub snapshot_id: String,
    #[serde(default)]
    pub names: Vec<String>,
    /// On-disk directory holding the snapshot bundle. Set by CubeMaster.
    #[serde(default)]
    pub path: String,
    /// Host that owns the blob. Returned so callers can plumb it back
    /// for delete.
    #[serde(default, rename = "host_ip", alias = "hostIP")]
    pub host_ip: String,
    pub ret: RetCode,
}

// ─── Sandbox snapshot delete ──────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct SandboxSnapshotDeleteRequest {
    #[serde(rename = "requestID")]
    pub request_id: String,
    pub snapshot_id: String,
    pub host_ip: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SandboxSnapshotDeleteResponse {
    #[serde(rename = "RequestID", alias = "requestID", default)]
    pub request_id: String,
    #[serde(default)]
    pub snapshot_id: String,
    pub ret: RetCode,
}

// ─── Helper: parse HTTP response ───────────────────────────────────────────

async fn parse_response<T: for<'de> Deserialize<'de>>(
    resp: reqwest::Response,
) -> Result<T, CubeMasterError> {
    let status = resp.status();
    let body = resp.text().await.map_err(CubeMasterError::Http)?;

    // Try to parse the envelope first — CubeMaster may return HTTP 200 even on
    // logical failures, with ret.ret_code != 0.
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body) {
        let ret_code = parsed
            .get("ret")
            .and_then(|r| r.get("ret_code"))
            .and_then(|c| c.as_i64())
            .map(|v| v as i32);

        if let Some(code) = ret_code {
            if code != 0 && code != 200 {
                let msg = parsed
                    .get("ret")
                    .and_then(|r| r.get("ret_msg"))
                    .and_then(|m| m.as_str())
                    .unwrap_or(&body)
                    .to_string();
                return Err(CubeMasterError::Api {
                    ret_code: code,
                    ret_msg: msg,
                });
            }
        } else if !status.is_success() {
            // No ret envelope, but HTTP error
            let code = status.as_u16() as i32;
            return Err(CubeMasterError::Api {
                ret_code: code,
                ret_msg: body,
            });
        }
    } else if !status.is_success() {
        return Err(CubeMasterError::Api {
            ret_code: status.as_u16() as i32,
            ret_msg: body,
        });
    }

    serde_json::from_str::<T>(&body)
        .map_err(|e| CubeMasterError::Deserialize(format!("{e}: body={body}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression for 782e721: CubeMaster's success-path snapshot
    /// payload does not include a `requestID` (the Go server's `Res`
    /// embed has `omitempty` and the value is empty unless the caller
    /// supplied one).  Earlier the Rust struct made `request_id`
    /// required, so this body would fail with `missing field
    /// 'RequestID'` and CubeAPI would return 500 to dyson-swarm even
    /// though the snapshot succeeded — leaving rotation pipelines
    /// hung waiting on a snapshot that was already on disk.
    #[test]
    fn snapshot_response_deserialises_without_request_id() {
        let body = r#"{
            "ret": {"ret_code": 200, "ret_msg": ""},
            "sandboxID": "0bedf0a224e649dc8c682c24fdc1d995",
            "snapshot_id": "smoke-3",
            "names": ["smoke-3"]
        }"#;
        let resp: SandboxSnapshotResponse =
            serde_json::from_str(body).expect("snapshot success body must deserialise without requestID");
        assert_eq!(resp.snapshot_id, "smoke-3");
        assert_eq!(resp.sandbox_id, "0bedf0a224e649dc8c682c24fdc1d995");
        assert_eq!(resp.request_id, "");
        assert_eq!(resp.ret.ret_code, 200);
    }

    /// Error-path payloads can carry neither `requestID` nor
    /// `snapshot_id` (e.g. the early-return blocks in
    /// CubeMaster/pkg/service/httpservice/cube/sandbox_snapshot.go
    /// that fire on missing sandboxID or upstream commit failures).
    /// CubeAPI must still parse those so the upper layer sees the
    /// real ret_code instead of a deserialiser error.
    #[test]
    fn snapshot_response_deserialises_error_envelope() {
        let body = r#"{"ret": {"ret_code": 130400, "ret_msg": "sandboxID is required"}}"#;
        let resp: SandboxSnapshotResponse =
            serde_json::from_str(body).expect("error envelope must deserialise even with no fields beyond ret");
        assert_eq!(resp.request_id, "");
        assert_eq!(resp.snapshot_id, "");
        assert_eq!(resp.ret.ret_code, 130400);
    }

    /// Belt-and-braces: when CubeMaster does include `requestID`
    /// (either capitalisation), it round-trips correctly.  Guards
    /// against a careless edit dropping the alias and silently
    /// throwing the field away on real production payloads.
    #[test]
    fn snapshot_response_accepts_both_request_id_casings() {
        for body in [
            r#"{"ret":{"ret_code":200,"ret_msg":""},"RequestID":"abc-123","snapshot_id":"s"}"#,
            r#"{"ret":{"ret_code":200,"ret_msg":""},"requestID":"abc-123","snapshot_id":"s"}"#,
        ] {
            let resp: SandboxSnapshotResponse =
                serde_json::from_str(body).expect("both casings must parse");
            assert_eq!(resp.request_id, "abc-123", "body: {body}");
        }
    }

    /// Same shape on the delete response — error-path delete bodies
    /// drop snapshot_id, and the success-path body lacks requestID
    /// when the caller didn't supply one.
    #[test]
    fn snapshot_delete_response_deserialises_minimal() {
        let body = r#"{"ret":{"ret_code":200,"ret_msg":""}}"#;
        let resp: SandboxSnapshotDeleteResponse =
            serde_json::from_str(body).expect("delete envelope-only body must deserialise");
        assert_eq!(resp.snapshot_id, "");
        assert_eq!(resp.request_id, "");
        assert_eq!(resp.ret.ret_code, 200);
    }
}
