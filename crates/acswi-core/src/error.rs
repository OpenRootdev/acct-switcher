use serde::Serialize;
use serde_json::Value;

/// Stable, machine-readable error codes. Agents branch on these, so never rename one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    UnknownApp,
    UnknownProfile,
    ProfileExists,
    InvalidName,
    InvalidDefinition,
    NotInstalled,
    AppRunning,
    UnsavedSession,
    IdentityMismatch,
    NeedsHuman,
    AddPending,
    CommandFailed,
    Busy,
    Recovery,
    Unsupported,
    Io,
    Crypto,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::UnknownApp => "unknown_app",
            Code::UnknownProfile => "unknown_profile",
            Code::ProfileExists => "profile_exists",
            Code::InvalidName => "invalid_name",
            Code::InvalidDefinition => "invalid_definition",
            Code::NotInstalled => "not_installed",
            Code::AppRunning => "app_running",
            Code::UnsavedSession => "unsaved_session",
            Code::IdentityMismatch => "identity_mismatch",
            Code::NeedsHuman => "needs_human",
            Code::AddPending => "add_pending",
            Code::CommandFailed => "command_failed",
            Code::Busy => "busy",
            Code::Recovery => "recovery",
            Code::Unsupported => "unsupported",
            Code::Io => "io",
            Code::Crypto => "crypto",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Error {
    pub code: Code,
    pub message: String,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub details: Value,
}

impl Error {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Error { code, message: message.into(), details: Value::Null }
    }

    pub fn with(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    pub fn io(context: impl std::fmt::Display, err: std::io::Error) -> Self {
        Error::new(Code::Io, format!("{context}: {err}"))
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Shorthand for `Err(Error::new(..))`.
macro_rules! bail {
    ($code:ident, $($arg:tt)*) => {
        return Err($crate::error::Error::new($crate::error::Code::$code, format!($($arg)*)))
    };
}
pub(crate) use bail;
