// SPDX-License-Identifier: BSD-2-Clause
//! Status codes (`wolkit_status`) and the error value every fallible engine call returns. A message
//! is one sentence for a person: what happened, and what to do.

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok = 0,
    InvalidArgument = 1,
    PermissionDenied = 2,
    Unavailable = 3,
    Internal = 4,
    /// A documented limit was reached (plan section 2.8).
    Limit = 5,
    /// The GPU went away; the engine falls back to the CPU rasteriser and tells the app.
    DeviceLost = 6,
}

impl Status {
    pub const ALL: &'static [(&'static str, Status)] = &[
        ("OK", Status::Ok),
        ("ERR_INVALID_ARGUMENT", Status::InvalidArgument),
        ("ERR_PERMISSION_DENIED", Status::PermissionDenied),
        ("ERR_UNAVAILABLE", Status::Unavailable),
        ("ERR_INTERNAL", Status::Internal),
        ("ERR_LIMIT", Status::Limit),
        ("ERR_DEVICE_LOST", Status::DeviceLost),
    ];
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub status: Status,
    pub message: String,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(status: Status, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(Status::InvalidArgument, message)
    }

    pub fn limit(message: impl Into<String>) -> Self {
        Self::new(Status::Limit, message)
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(Status::Unavailable, message)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}
