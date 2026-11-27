use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;
pub type DbResult<T> = Result<T, DbError>;

fn format_coded(key: &str, args: &[String]) -> String {
    if args.is_empty() {
        key.to_string()
    } else {
        format!("{}|{}", key, args.join("|"))
    }
}

#[derive(Debug, Error)]
pub enum DbError {
    #[error("err.db_connect|{0}")]
    Connect(#[from] r2d2::Error),
    #[error("err.db_lock|{0}")]
    Lock(String),
    #[error("{0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("{}", format_coded(.key, .args))]
    Validation {
        key: &'static str,
        args: Vec<String>,
    },
    #[error("err.query_failed|{0}")]
    Query(String),
    #[error("err.task_failed|{0}")]
    TaskJoin(String),
    #[error("{0}")]
    Other(String),
}

impl DbError {
    pub fn validation_args(
        key: &'static str,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::Validation {
            key,
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    pub fn to_ipc_string(&self) -> String {
        self.to_string()
    }
}

impl From<DbError> for String {
    fn from(e: DbError) -> Self {
        e.to_ipc_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceErrorKind {
    RateLimit,
    RateLimitReset { wait_secs: i64 },
    CredentialInvalid,
    YoutubeApiQuota { detail: Option<String> },
    YoutubeApiKeyInvalid { detail: Option<String> },
    BiliRisk { detail: Option<String> },
    BiliRateLimit,
    SaveTimeout { secs: u64 },
    SourceTimeout { secs: u64 },
    UnsupportedSource { source_type: String },
    InvalidUrl,
    ApiError { status: u16, reason: String },
    Coded { key: &'static str, args: Vec<String> },
    Raw(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{}", self.to_ipc_string())]
pub struct SourceError {
    pub status: u16,
    pub kind: SourceErrorKind,
}

impl SourceError {
    pub fn new(status: u16, kind: SourceErrorKind) -> Self {
        Self { status, kind }
    }

    pub fn coded(status: u16, key: &'static str, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            status,
            kind: SourceErrorKind::Coded {
                key,
                args: args.into_iter().map(Into::into).collect(),
            },
        }
    }

    pub fn key(status: u16, key: &'static str) -> Self {
        Self {
            status,
            kind: SourceErrorKind::Coded {
                key,
                args: Vec::new(),
            },
        }
    }

    pub fn raw(status: u16, msg: impl Into<String>) -> Self {
        Self {
            status,
            kind: SourceErrorKind::Raw(msg.into()),
        }
    }

    pub fn api_error(status: u16, reason: impl Into<String>) -> Self {
        Self {
            status,
            kind: SourceErrorKind::ApiError {
                status,
                reason: reason.into(),
            },
        }
    }

    pub fn rate_limit(status: u16) -> Self {
        Self::new(status, SourceErrorKind::RateLimit)
    }

    pub fn rate_limit_reset(status: u16, wait_secs: i64) -> Self {
        Self::new(status, SourceErrorKind::RateLimitReset { wait_secs })
    }

    pub fn credential_invalid(status: u16) -> Self {
        Self::new(status, SourceErrorKind::CredentialInvalid)
    }

    pub fn invalid_url() -> Self {
        Self::new(0, SourceErrorKind::InvalidUrl)
    }

    pub fn save_timeout(secs: u64) -> Self {
        Self::new(0, SourceErrorKind::SaveTimeout { secs })
    }

    pub fn source_timeout(secs: u64) -> Self {
        Self::new(0, SourceErrorKind::SourceTimeout { secs })
    }

    pub fn unsupported_source(source_type: impl Into<String>) -> Self {
        Self::new(
            0,
            SourceErrorKind::UnsupportedSource {
                source_type: source_type.into(),
            },
        )
    }

    pub fn status(&self) -> u16 {
        self.status
    }

    pub fn is_invalid_url(&self) -> bool {
        matches!(self.kind, SourceErrorKind::InvalidUrl)
    }

    /// 上游配额、限流、全局凭据或保存超时等环境故障不计入单源断路器，避免误伤整批同源。
    pub fn is_uncounted_failure(&self) -> bool {
        matches!(
            self.kind,
            SourceErrorKind::YoutubeApiQuota { .. }
                | SourceErrorKind::YoutubeApiKeyInvalid { .. }
                | SourceErrorKind::BiliRisk { .. }
                | SourceErrorKind::BiliRateLimit
                | SourceErrorKind::RateLimit
                | SourceErrorKind::RateLimitReset { .. }
                | SourceErrorKind::CredentialInvalid
                | SourceErrorKind::SaveTimeout { .. }
        )
    }

    pub fn counts_against_circuit(&self) -> bool {
        !self.is_uncounted_failure()
    }

    pub fn should_retry_http(&self) -> bool {
        self.status != 403
            && !matches!(
                self.kind,
                SourceErrorKind::RateLimitReset { .. } | SourceErrorKind::CredentialInvalid
            )
    }

    pub fn to_ipc_string(&self) -> String {
        match &self.kind {
            SourceErrorKind::RateLimit => "err.rate_limit".to_string(),
            SourceErrorKind::RateLimitReset { wait_secs } => {
                format!("err.rate_limit_reset|{}", wait_secs)
            }
            SourceErrorKind::CredentialInvalid => "err.credential_invalid".to_string(),
            SourceErrorKind::YoutubeApiQuota { detail: Some(d) } => {
                format!("err.youtube_api_quota|{}", d)
            }
            SourceErrorKind::YoutubeApiQuota { detail: None } => {
                "err.youtube_api_quota".to_string()
            }
            SourceErrorKind::YoutubeApiKeyInvalid { detail: Some(d) } => {
                format!("err.youtube_api_key_invalid|{}", d)
            }
            SourceErrorKind::YoutubeApiKeyInvalid { detail: None } => {
                "err.youtube_api_key_invalid".to_string()
            }
            SourceErrorKind::BiliRisk { detail: Some(d) } => format!("err.bili_risk|{}", d),
            SourceErrorKind::BiliRisk { detail: None } => "err.bili_risk".to_string(),
            SourceErrorKind::BiliRateLimit => "err.bili_rate_limit".to_string(),
            SourceErrorKind::SaveTimeout { secs } => format!("err.save_timeout|{}", secs),
            SourceErrorKind::SourceTimeout { secs } => format!("err.source_timeout|{}", secs),
            SourceErrorKind::UnsupportedSource { source_type } => {
                format!("err.unsupported_source|{}", source_type)
            }
            SourceErrorKind::InvalidUrl => "err.invalid_url".to_string(),
            SourceErrorKind::ApiError { status, reason } => {
                format!("err.api_error|{}|{}", status, reason)
            }
            SourceErrorKind::Coded { key, args } => format_coded(key, args),
            SourceErrorKind::Raw(s) => s.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{}", self.to_ipc_string())]
pub enum AgentRunError {
    Timeout {
        timeout_secs: Option<u64>,
        partial_stdout: String,
    },
    EndLost {
        partial_stdout: String,
    },
    Aborted {
        partial_stdout: String,
    },
    RunNotFound,
    Coded {
        key: &'static str,
        args: Vec<String>,
        partial_stdout: String,
    },
    Failed {
        code: String,
        partial_stdout: String,
    },
}

impl AgentRunError {
    pub fn timeout(partial_stdout: String) -> Self {
        Self::Timeout {
            timeout_secs: None,
            partial_stdout,
        }
    }

    pub fn timeout_secs(secs: u64, partial_stdout: String) -> Self {
        Self::Timeout {
            timeout_secs: Some(secs),
            partial_stdout,
        }
    }

    pub fn end_lost(partial_stdout: String) -> Self {
        Self::EndLost { partial_stdout }
    }

    pub fn aborted(partial_stdout: String) -> Self {
        Self::Aborted { partial_stdout }
    }

    pub fn coded(key: &'static str, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::Coded {
            key,
            args: args.into_iter().map(Into::into).collect(),
            partial_stdout: String::new(),
        }
    }

    pub fn key_with_output(key: &'static str, partial_stdout: String) -> Self {
        Self::Coded {
            key,
            args: Vec::new(),
            partial_stdout,
        }
    }

    pub fn key(key: &'static str) -> Self {
        Self::Coded {
            key,
            args: Vec::new(),
            partial_stdout: String::new(),
        }
    }

    pub fn failed(code: impl Into<String>, partial_stdout: String) -> Self {
        Self::Failed {
            code: code.into(),
            partial_stdout,
        }
    }

    pub fn is_run_not_found(&self) -> bool {
        matches!(self, Self::RunNotFound)
    }

    pub fn partial_stdout(&self) -> &str {
        match self {
            Self::Timeout { partial_stdout, .. }
            | Self::EndLost { partial_stdout }
            | Self::Aborted { partial_stdout }
            | Self::Coded { partial_stdout, .. }
            | Self::Failed { partial_stdout, .. } => partial_stdout,
            Self::RunNotFound => "",
        }
    }

    pub fn into_partial_stdout(self) -> String {
        match self {
            Self::Timeout { partial_stdout, .. }
            | Self::EndLost { partial_stdout }
            | Self::Aborted { partial_stdout }
            | Self::Coded { partial_stdout, .. }
            | Self::Failed { partial_stdout, .. } => partial_stdout,
            Self::RunNotFound => String::new(),
        }
    }

    pub fn terminal_status(&self) -> &'static str {
        match self {
            Self::Timeout { .. } => crate::agent::STATUS_TIMEOUT,
            Self::EndLost { .. } => crate::agent::STATUS_UNKNOWN,
            Self::Aborted { .. } => crate::agent::STATUS_CANCELLED,
            Self::RunNotFound | Self::Coded { .. } | Self::Failed { .. } => {
                crate::agent::STATUS_FAILED
            }
        }
    }

    pub fn to_ipc_string(&self) -> String {
        match self {
            Self::Timeout {
                timeout_secs: Some(s),
                ..
            } => format!("err.agent.timeout|{}", s),
            Self::Timeout {
                timeout_secs: None, ..
            } => "err.agent.timeout".to_string(),
            Self::EndLost { .. } => "err.agent.end_lost".to_string(),
            Self::Aborted { .. } => "err.agent.aborted".to_string(),
            Self::RunNotFound => "err.agent.run_not_found".to_string(),
            Self::Coded { key, args, .. } => format_coded(key, args),
            Self::Failed { code, .. } => code.clone(),
        }
    }
}

impl From<String> for AgentRunError {
    fn from(code: String) -> Self {
        Self::failed(code, String::new())
    }
}

impl From<&str> for AgentRunError {
    fn from(code: &str) -> Self {
        Self::failed(code, String::new())
    }
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    Db(#[from] DbError),
    #[error("{0}")]
    Source(#[from] SourceError),
    #[error("{0}")]
    Agent(#[from] AgentRunError),
    #[error("{}", format_coded(.key, .args))]
    Validation {
        key: &'static str,
        args: Vec<String>,
    },
    #[error("{key}")]
    Cancelled { key: &'static str },
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Raw(String),
}

impl AppError {
    pub fn validation(key: &'static str) -> Self {
        Self::Validation {
            key,
            args: Vec::new(),
        }
    }

    pub fn validation_args(
        key: &'static str,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::Validation {
            key,
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    pub fn cancelled(key: &'static str) -> Self {
        Self::Cancelled { key }
    }

    pub fn raw(msg: impl Into<String>) -> Self {
        Self::Raw(msg.into())
    }

    pub fn to_ipc_string(&self) -> String {
        match self {
            Self::Db(e) => e.to_ipc_string(),
            Self::Source(e) => e.to_ipc_string(),
            Self::Agent(e) => e.to_ipc_string(),
            Self::Validation { key, args } => format_coded(key, args),
            Self::Cancelled { key } => (*key).to_string(),
            Self::Io(e) => e.to_string(),
            Self::Raw(s) => s.clone(),
        }
    }

    pub fn is_run_not_found(&self) -> bool {
        matches!(self, Self::Agent(e) if e.is_run_not_found())
    }

    pub fn is_invalid_url(&self) -> bool {
        matches!(self, Self::Source(e) if e.is_invalid_url())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(DbError::Sqlite(e))
    }
}

impl From<r2d2::Error> for AppError {
    fn from(e: r2d2::Error) -> Self {
        Self::Db(DbError::Connect(e))
    }
}

impl From<String> for AppError {
    fn from(s: String) -> Self {
        Self::Raw(s)
    }
}

impl From<AppError> for String {
    fn from(e: AppError) -> Self {
        e.to_ipc_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_error_circuit_breaker_classification() {
        let uncounted = [
            SourceError::new(403, SourceErrorKind::YoutubeApiQuota { detail: Some("quotaExceeded".into()) }),
            SourceError::new(400, SourceErrorKind::YoutubeApiKeyInvalid { detail: None }),
            SourceError::new(412, SourceErrorKind::BiliRisk { detail: Some("HTTP 412".into()) }),
            SourceError::new(429, SourceErrorKind::BiliRateLimit),
            SourceError::rate_limit(429),
            SourceError::rate_limit_reset(403, 120),
            SourceError::credential_invalid(401),
            SourceError::save_timeout(300),
        ];
        for err in &uncounted {
            assert!(err.is_uncounted_failure(), "expected uncounted: {err:?}");
            assert!(!err.counts_against_circuit());
        }

        let counted = [
            SourceError::api_error(404, "Not Found"),
            SourceError::source_timeout(300),
            SourceError::coded(500, "err.repo_api_error", ["500"]),
            SourceError::invalid_url(),
        ];
        for err in &counted {
            assert!(!err.is_uncounted_failure(), "expected counted: {err:?}");
            assert!(err.counts_against_circuit());
        }
    }

    #[test]
    fn source_error_http_retry_policy() {
        assert!(!SourceError::api_error(403, "Forbidden").should_retry_http());
        assert!(!SourceError::rate_limit_reset(429, 60).should_retry_http());
        assert!(!SourceError::credential_invalid(401).should_retry_http());
        assert!(SourceError::rate_limit(429).should_retry_http());
        assert!(SourceError::api_error(502, "Bad Gateway").should_retry_http());
    }

    #[test]
    fn source_error_ipc_serialization() {
        assert_eq!(SourceError::rate_limit(429).to_ipc_string(), "err.rate_limit");
        assert_eq!(
            SourceError::rate_limit_reset(403, 90).to_ipc_string(),
            "err.rate_limit_reset|90"
        );
        assert_eq!(
            SourceError::api_error(404, "Not Found").to_ipc_string(),
            "err.api_error|404|Not Found"
        );
        assert_eq!(
            SourceError::unsupported_source("gitlab").to_ipc_string(),
            "err.unsupported_source|gitlab"
        );
        assert_eq!(
            SourceError::coded(0, "err.request_timeout", ["boom"]).to_ipc_string(),
            "err.request_timeout|boom"
        );
    }

    #[test]
    fn agent_run_error_terminal_status_without_prefix_matching() {
        assert_eq!(
            AgentRunError::timeout("partial".into()).terminal_status(),
            crate::agent::STATUS_TIMEOUT
        );
        assert_eq!(
            AgentRunError::end_lost("partial".into()).terminal_status(),
            crate::agent::STATUS_UNKNOWN
        );
        assert_eq!(
            AgentRunError::aborted(String::new()).terminal_status(),
            crate::agent::STATUS_CANCELLED
        );
        assert_eq!(
            AgentRunError::failed("err.agent.rpc_exited|1", String::new()).terminal_status(),
            crate::agent::STATUS_FAILED
        );
        assert!(AgentRunError::RunNotFound.is_run_not_found());
        assert_eq!(AgentRunError::RunNotFound.to_ipc_string(), "err.agent.run_not_found");
    }

    #[test]
    fn app_error_ipc_roundtrip() {
        assert_eq!(
            AppError::validation("err.empty_body").to_ipc_string(),
            "err.empty_body"
        );
        assert_eq!(
            AppError::validation_args("err.agent.ws_width_range", ["300", "900"]).to_ipc_string(),
            "err.agent.ws_width_range|300|900"
        );
        assert_eq!(
            AppError::cancelled("err.backup_cancelled_export").to_ipc_string(),
            "err.backup_cancelled_export"
        );
    }
}
