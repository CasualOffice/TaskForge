//! # casual-task-workflow
//!
//! Configurable statuses over five permanent states (`docs/23-WORKFLOW-AND-STATE-MACHINE.md`).
//!
//! **Owns:** workflows, statuses, transitions, the status-to-state mapping, transition validation, and the status-migration rules that keep in-flight tasks explicable.
//!
//! **Must never own:** the task aggregate itself, or the act of writing a status. A transition is a command executed by `casual-task-app`.
//!
//! Boundary contract: `docs/19-WORKSPACE-SCAFFOLD-DESIGN.md`. An illegal
//! dependency here is a build failure, not a review comment.
//!
//! The state machine is implemented (C-007). Status editing and status
//! migration are not. See `docs/14-EXECUTION-TRACKER.md`.

pub mod workflow;

pub use workflow::{
    Rejection, Status, Transition, TransitionRequest, ValidTransition, Workflow, WorkflowError,
};
