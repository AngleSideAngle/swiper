#![no_std]
#![doc = include_str!("../README.md")]

pub mod requirement;
pub mod thief;

/// Contains information about a preemption, including the newly scheduled incoming task, the newly cancelled outgoing task, and the requirement that was preempted
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreemptionError {
    incoming: Option<thief::ThiefInfo>,
    outgoing: thief::ThiefInfo,
    requirement: requirement::RequirementInfo,
}

/// Result that is either `Ok` or `PreemptionError`
pub type Result<T> = core::result::Result<T, PreemptionError>;

impl core::fmt::Display for PreemptionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if let Some(incoming) = self.incoming {
            write!(
                f,
                "outgoing task {} was preempted by incoming task {} stealing its requirement {}",
                self.outgoing, incoming, self.requirement
            )
        } else {
            write!(
                f,
                "outgoing task {} was preempted by an unknown incoming task stealing its requirement {}",
                self.outgoing, self.requirement
            )
        }
    }
}
