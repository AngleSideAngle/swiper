use core::{
    cell::{Cell, UnsafeCell},
    fmt::Display,
    ptr::NonNull,
};

use crate::preemptible_future::ThiefInfo;

/// Contains metadata about a [`RevocableCell`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequirementInfo {
    pub name: &'static str,
}

impl Display for RequirementInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Requirement {{ name: {} }} ", self.name)
    }
}

/// Keeps track of the current owner of a requirement.
///
/// Thiefs ([`PreemptibleFuture`]) acts as guards to the requirement by ensuring they do not access a revoked requirement.
/// Requirements should be checked for equality using `std::ptr::eq`, rather than `ThiefInfo` because `ThiefInfo` is not unique.
///
/// This provides a type-independent reference to downcast `RevocableCell<T>` into.
pub trait Requirement {
    /// Sets the current owner of this requirement to the provided `thief`.
    /// This will revoke access to the previous owner, if it existed.
    fn steal_ownership(&self, thief: &ThiefInfo);

    /// Releases the current flag owner.
    /// This means no thief will have access to this requirement.
    fn release_ownership(&self);

    /// Returns information about the current flag owner.
    fn current_owner(&self) -> Option<&ThiefInfo>;

    /// Returns information about the current requirement.
    fn info(&self) -> RequirementInfo;
}

/// A pointer to a mutable location in memory that enables reference holders to call [`steal_flag()`](Self::steal_flag) to revoke flags from other reference holders.
///
/// This struct cannot be directly used in a safe manner, and must be accessed inside a [`PreemptibleFuture`].
pub struct RevocableCell<T> {
    pub data: UnsafeCell<T>,
    owner: Cell<Option<NonNull<ThiefInfo>>>,
    name: &'static str,
}

impl<T> RevocableCell<T> {
    /// Creates a new [`RevocableCell`] with ownership of `data`.
    ///
    /// The cell will default having no owner.
    pub fn new(data: T, name: &'static str) -> Self {
        Self {
            data: data.into(),
            owner: Cell::new(None),
            name,
        }
    }
}

impl<T> Requirement for RevocableCell<T> {
    fn steal_ownership(&self, thief: &ThiefInfo) {
        self.owner.set(Some(thief.into()));
    }

    fn release_ownership(&self) {
        self.owner.set(None);
    }

    fn current_owner(&self) -> Option<&ThiefInfo> {
        self.owner.get().map(|ptr| unsafe { ptr.as_ref() })
    }

    fn info(&self) -> RequirementInfo {
        RequirementInfo { name: self.name }
    }
}

#[cfg(test)]
mod tests {
    use core::ptr;

    use super::*;

    #[test]
    fn flag_stealing() {
        let cell = RevocableCell::new(0, "test");
        let thief1 = ThiefInfo { name: "test" };
        let thief2 = ThiefInfo { name: "test" };
        {
            assert!(cell.current_owner().is_none());
            cell.steal_ownership(&thief1);
            assert!(ptr::eq(
                cell.current_owner().expect("should be owned"),
                &thief1
            ));
            // steal new flag
            cell.steal_ownership(&thief2);
            assert!(cell.current_owner().is_some());
            assert!(ptr::eq(
                cell.current_owner().expect("should be owned"),
                &thief2
            ));
            assert!(!ptr::eq(
                cell.current_owner().expect("should be owned"),
                &thief1
            ));
        }
        cell.release_ownership();
        assert!(cell.current_owner().is_none());
    }
}
