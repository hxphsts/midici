//! Property Exchange status codes: Reply Header `status` (M2-103 §5.4.1)
//! and Notify `status` (M2-103 §10.1.1).

/// A PE status code. Open-ended: unknown codes are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PeStatus(u16);

impl PeStatus {
    /// A status code (any JSON integer that fits `u16`).
    #[must_use]
    pub const fn new(code: u16) -> Self {
        Self(code)
    }

    /// The code.
    #[must_use]
    pub const fn value(self) -> u16 {
        self.0
    }

    /// Notify: Timeout Wait (§10.3.1).
    pub const TIMEOUT_WAIT: Self = Self(100);
    /// Notify: Terminate Inquiry (§10.2).
    pub const TERMINATE: Self = Self(144);
    /// Success/OK.
    pub const OK: Self = Self(200);
    /// Accepted, results not guaranteed.
    pub const ACCEPTED: Self = Self(202);
    /// Resource currently unavailable or an error occurred (retry).
    pub const UNAVAILABLE: Self = Self(341);
    /// Bad data / unexpected end of data set (retry).
    pub const BAD_DATA: Self = Self(342);
    /// Too many requests (retry).
    pub const TOO_MANY_REQUESTS: Self = Self(343);
    /// Bad request.
    pub const BAD_REQUEST: Self = Self(400);
    /// Reply not available based on authorisation.
    pub const FORBIDDEN: Self = Self(403);
    /// Resource not supported/found.
    pub const NOT_FOUND: Self = Self(404);
    /// Resource not allowed at this time.
    pub const NOT_ALLOWED: Self = Self(405);
    /// Notify: Timeout has occurred (§10.3.2).
    pub const TIMEOUT: Self = Self(408);
    /// Payload too large.
    pub const PAYLOAD_TOO_LARGE: Self = Self(413);
    /// Unsupported media type or encoding.
    pub const UNSUPPORTED_MEDIA_TYPE: Self = Self(415);
    /// Invalid version of data.
    pub const INVALID_VERSION: Self = Self(445);
    /// Internal device error.
    pub const INTERNAL_ERROR: Self = Self(500);

    /// `200..=299`: success, do not retry.
    #[must_use]
    pub const fn is_success(self) -> bool {
        self.0 >= 200 && self.0 <= 299
    }

    /// `300..=399`: should retry.
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        self.0 >= 300 && self.0 <= 399
    }

    /// Whether a Reply may use this code (M2-103 §5.4.1 lists the only
    /// allowed codes).
    #[must_use]
    pub const fn is_reply_code(self) -> bool {
        matches!(
            self.0,
            200 | 202 | 341 | 342 | 343 | 400 | 403 | 404 | 405 | 413 | 415 | 445 | 500
        )
    }

    /// Whether a Notify may use this code (M2-103 §10.1.1).
    #[must_use]
    pub const fn is_notify_code(self) -> bool {
        matches!(self.0, 100 | 144 | 408)
    }
}
