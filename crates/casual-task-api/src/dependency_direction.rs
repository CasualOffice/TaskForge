/// Which way the edge points, once the request is known to be coherent.
#[derive(Debug, Clone, Copy)]
enum Direction {
    /// This task blocks `other`.
    Blocks { this: Uuid, other: Uuid },
    /// `other` blocks this task.
    BlockedBy { this: Uuid, other: Uuid },
}

impl Direction {
    fn of(body: &AddDependency, this: Uuid, request_id: &str) -> Result<Self, ApiError> {
        match (body.blocks, body.blocked_by) {
            (Some(_), Some(_)) => Err(ApiError::bad_request(
                codes::MALFORMED_BODY,
                "Send either `blocks` or `blocked_by`, not both",
                request_id,
            )),
            (Some(other), None) => Ok(Self::Blocks { this, other }),
            (None, Some(other)) => Ok(Self::BlockedBy { this, other }),
            (None, None) => Err(ApiError::bad_request(
                codes::MISSING_FIELD,
                "Send `blocks` with the task this one blocks, or `blocked_by` \
                 with the task that blocks it",
                request_id,
            )),
        }
    }

    /// `(blocker, blocked)` — the row's `from_task_id` and `to_task_id`.
    const fn edge(self) -> (Uuid, Uuid) {
        match self {
            Self::Blocks { this, other } => (this, other),
            Self::BlockedBy { this, other } => (other, this),
        }
    }

    const fn other(self) -> Uuid {
        match self {
            Self::Blocks { other, .. } | Self::BlockedBy { other, .. } => other,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Blocks { .. } => "blocks",
            Self::BlockedBy { .. } => "blocked_by",
        }
    }
}
