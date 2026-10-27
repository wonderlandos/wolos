//! Stable error codes shared by CLI exit status, wire replies and logs. Values and exit codes are ABI.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ErrorCode {
    Ok = 0,
    Failed = 1,
    Usage = 2,
    NotFound = 3,
    Denied = 4,
    Unavailable = 5,
    Invalid = 6,
    Unsupported = 7,
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 8] = [
        ErrorCode::Ok,
        ErrorCode::Failed,
        ErrorCode::Usage,
        ErrorCode::NotFound,
        ErrorCode::Denied,
        ErrorCode::Unavailable,
        ErrorCode::Invalid,
        ErrorCode::Unsupported,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ErrorCode::Ok => "ok",
            ErrorCode::Failed => "failed",
            ErrorCode::Usage => "usage",
            ErrorCode::NotFound => "not-found",
            ErrorCode::Denied => "denied",
            ErrorCode::Unavailable => "unavailable",
            ErrorCode::Invalid => "invalid",
            ErrorCode::Unsupported => "unsupported",
        }
    }

    /// Process exit status (sysexits, IaraOS CLI invariant: 0, 1, 64, 65, 66, 69, 77).
    pub fn exit(self) -> i32 {
        match self {
            ErrorCode::Ok => 0,
            ErrorCode::Failed => 1,
            ErrorCode::Usage => 64,
            ErrorCode::Invalid => 65,
            ErrorCode::NotFound => 66,
            ErrorCode::Unavailable | ErrorCode::Unsupported => 69,
            ErrorCode::Denied => 77,
        }
    }
}

/// Error with a stable code and human message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WolError {
    pub code: ErrorCode,
    pub message: String,
}

impl WolError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for WolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.name(), self.message)
    }
}

impl std::error::Error for WolError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_unique() {
        let exits = [0, 1, 64, 66, 77, 69, 65, 69];
        for (c, e) in ErrorCode::ALL.iter().zip(exits) {
            assert_eq!(c.exit(), e, "{c:?}");
        }
        let mut names: Vec<_> = ErrorCode::ALL.iter().map(|c| c.name()).collect();
        names.dedup();
        assert_eq!(names.len(), 8);
        assert_eq!(
            WolError::new(ErrorCode::Denied, "x").to_string(),
            "denied: x"
        );
    }
}
