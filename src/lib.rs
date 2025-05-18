//! macro to wrap inner function with `MutexingFuture` and replace args with `Arc<WimpyMutex<T>>` args
//! `MutexingFuture` wraps original function's future and validates `WimpyMutex` every time the function is polled
//!
//! ## Desired behaviors/granularity:
//!
//! ### Cancellation layer
//!
//! #[requirement] macro at the struct level
//! all &mut struct interactions now need to get derefmut, which locks wimpy mutex
//!
//!
//! ### Ideal Code
//!
//! ```rs
//! struct Example { ... }
//!
//! impl for RequirementMutex<Example> {
//!     #[enforce_mutexing]
//!     async fn(&mut self) { ... }
//! }
//! ```
//!
//!
//! Annotation for struct to convert struct into WimpyMutex<Self> with deref impl
//! That way any function called on the struct that mutates it will take ownership of the struct

use std::{
    cell::{Cell, RefCell, UnsafeCell},
    future::Future,
    pin::Pin,
    rc::{Rc, Weak},
    task::{Context, Poll},
};

struct LastThiefInfo {}

type OwnershipFlag = Rc<Cell<bool>>;

/// this gets revoked, unsafe to manually interact with outside of PreemptableFuture api
/// currently requires single threaded async execution
/// DOES NOT WORK WITH MULTITHREADED BC CRITICAL SECTIONS FOR ALL ASYNC POLLING NEEDS TO OVERLAP
// TODO replace bool flag with owning task info for introspection
struct RevocableCell<T> {
    data: UnsafeCell<T>,
    current_flag: RefCell<Weak<Cell<bool>>>,
}

impl<T> RevocableCell<T> {
    pub fn new(data: T) -> Self {
        Self {
            data: data.into(),
            current_flag: RefCell::new(Weak::new()),
        }
    }

    pub fn steal_flag(&self) -> OwnershipFlag {
        if let Some(flag) = self.current_flag.borrow().upgrade() {
            // revoke previous guard's access
            flag.set(false);
        }

        // replace revoked flag with newly claimed flag
        let new_flag = Rc::new(Cell::new(true));
        self.current_flag.replace(Rc::downgrade(&new_flag));

        new_flag
    }

    pub fn is_claimed(&self) -> bool {
        self.current_flag
            .borrow()
            .upgrade()
            .is_some_and(|cell| cell.get())
    }
}

#[derive(Debug, Clone)]
struct PreemptionError;

struct PreemptableFuture<'mutex, F: Future, T> {
    inner: F,
    requirement: &'mutex RevocableCell<T>, // should be a guard
    valid_flag: Option<OwnershipFlag>,
}

// impl<'mutex, F: Future, T> PreemptableFuture<'mutex, F, T> {
//     fn new(inner: F, requirement: &'mutex RevocableCell<T>) -> Self {
//         Self {
//             inner,
//             requirement,
//             valid_flag: Option::None,
//         }
//     }
// }

impl<F: Future, T> Future for PreemptableFuture<'_, F, T> {
    type Output = Result<F::Output, PreemptionError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // pin guarantees all movement sensitive data is not moved
        // in order to extract the fields of the Pin<&mut Self>,
        // the inner representation needs to be extracted
        // and the movement sensitive part (inner Future) needs to be re-pinned
        let instance = unsafe { self.get_unchecked_mut() };
        let inner = unsafe { Pin::new_unchecked(&mut instance.inner) };
        let requirement = instance.requirement;
        let valid_flag = &mut instance.valid_flag;

        let lock = valid_flag.get_or_insert_with(|| requirement.steal_flag());

        if !lock.get() {
            return Poll::Ready(Err(PreemptionError));
        }

        inner.poll(cx).map(Ok)
    }
}

// impl<F: Future, T> Drop for PreemptableFuture<F, T> {
//     fn drop(&mut self) {
//         // TODO relinquish mutex lock
//         // this is necessary for situations where a subsystem is uncommanded
//         // since default commands should be triggered in this situation
//         todo!()
//     }
// }

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
