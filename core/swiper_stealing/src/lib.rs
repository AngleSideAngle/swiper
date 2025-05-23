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

#![cfg_attr(not(feature = "std"), no_std)]

use core::{
    cell::{Cell, UnsafeCell},
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

struct LastThiefInfo {}

/// this gets revoked, unsafe to manually interact with outside of PreemptableFuture api
/// currently requires single threaded async execution
/// DOES NOT WORK WITH MULTITHREADED BC CRITICAL SECTIONS FOR ALL ASYNC POLLING NEEDS TO OVERLAP
// TODO replace bool flag with owning task info for introspection
pub struct RevocableCell<'flag, T> {
    data: UnsafeCell<T>,
    current_flag: Cell<Option<&'flag Cell<Option<bool>>>>,
}

impl<'flag, T> RevocableCell<'flag, T> {
    pub fn new(data: T) -> Self {
        Self {
            data: data.into(),
            current_flag: Cell::new(None),
        }
    }

    pub fn steal_flag(&self, new_flag: &'flag Cell<Option<bool>>) {
        // revoke previous flag pointer (set to false)
        if let Some(previous_flag_ptr) = self.current_flag.get() {
            previous_flag_ptr.set(Some(false));
        }

        // replace old flag with new flag (set to true)
        new_flag.set(Some(true));
        self.current_flag.set(Some(new_flag));
    }

    pub fn is_claimed(&self) -> bool {
        self.current_flag.get().is_some_and(|cell| cell.get().unwrap_or(false))
    }
}

#[derive(Debug, Clone)]
pub struct PreemptionError;

pub struct PreemptableFuture<'mutex, 'flag, F: Future, T> where 'flag: 'mutex {
    inner: F,
    requirement: &'mutex RevocableCell<'mutex, T>,
    valid_flag: &'flag Cell<Option<bool>>,
}

// impl<'mutex, F: Future, T> PreemptableFuture<'mutex, F, T> {
//     fn new(inner: F, requirement: &'mutex RevocableCell<'mutex, T>) -> Self {
//         Self {
//             inner,
//             requirement,
//             valid_flag: Cell::new(None),
//         }
//     }
// }

impl<F: Future, T> Future for PreemptableFuture<'_, '_, F, T> {
    type Output = Result<F::Output, PreemptionError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // pin guarantees all movement sensitive data is not moved
        // in order to extract the fields of the Pin<&mut Self>,
        // the inner representation needs to be extracted
        // and the movement sensitive part (inner Future) needs to be re-pinned
        let instance = unsafe { self.get_unchecked_mut() };
        let inner = unsafe { Pin::new_unchecked(&mut instance.inner) };
        let requirement = instance.requirement;
        let valid_flag = &instance.valid_flag;

        if valid_flag.get().is_none() {
            requirement.steal_flag(valid_flag);
        }

        if !valid_flag.get().is_some_and(|c| c) {
            return Poll::Ready(Err(PreemptionError));
        }

        inner.poll(cx).map(Ok)
    }
}

// pub fn with_requirement<Requirement, Out, Fut, Func>(
//     func: Func,
//     requirement: &RevocableCell<Requirement>,
// ) -> PreemptableFuture<'_, Fut, Requirement>
// where
//     Func: AsyncFnOnce(&mut Requirement) -> Out,
//     Fut: Future<Output = Out>,
// {
//     let data = unsafe { &mut *requirement.data.get() };
//     let inner = func(data);
//     PreemptableFuture {
//         inner,
//         requirement,
//         valid_flag: Option::None,
//     }
// }

// this functionality seems to depend on async_fn_traits
// https://doc.rust-lang.org/unstable-book/library-features/async-fn-traits.html?highlight=async_fn#async_fn_traits
// looks like this is blocking on variadic generics, which won't be here anytime soon

// pub trait StandardTask<Requirement, Output, Func>
// where
//     Func: AsyncFnMut(&mut Requirement) -> Output,
// {
//     fn with_requirement<'mutex>(
//         &mut self,
//         requirement: &'mutex RevocableCell<Requirement>,
//     ) -> PreemptableFuture<'mutex, Func::CallRefFuture, Requirement>;
// }

// // FnMut -> Future could be ASyncFnMut when async_fn_traits gets stable
// impl<Requirement, Output, Func> StandardTask<Requirement, Output> for Func
// where
//     Func: AsyncFnMut(&mut Requirement) -> Output,
// {
//     fn with_requirement<'mutex>(
//         &mut self,
//         requirement: &'mutex RevocableCell<Requirement>,
//     ) -> PreemptableFuture<'mutex, Func::CallRefFuture, Requirement> {
//         let data = requirement.data.get_mut();
//         let inner = self(data);
//         PreemptableFuture {
//             inner,
//             requirement,
//             valid_flag: Option::None,
//         }
//     }
// }

#[cfg(test)]
mod tests {

    use super::*;

    struct PollForever {}

    impl Future for PollForever {
        type Output = i32;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }

    #[test]
    fn flag_stealing() {
        let cell = RevocableCell::new(2);
        let flag_1 = Cell::new(None); // unclaimed flag
        assert!(!cell.is_claimed());
        cell.steal_flag(&flag_1);
        assert_eq!(flag_1.get(), Some(true));
        assert!(cell.is_claimed());
        // steal new flag
        let flag_2 = Cell::new(None);
        cell.steal_flag(&flag_2);
        assert_eq!(flag_1.get(), Some(false));
        assert_eq!(flag_2.get(), Some(true));
        assert!(cell.is_claimed());
        // drop(flag_1);
        assert!(!cell.is_claimed());
    }

    async fn incr(x: &mut String) {
        x.push('h');
    }

    // #[test]
    // fn future_mutexing() {
    //     let mut resource = RevocableCell::new("hi".to_string());
    //     // let incr = async |x: &mut String| loop {
    //     //     x.push('h');
    //     // };
    //     let add_j = async |x: &mut String| loop {
    //         x.push('j');
    //     };

    //     let fut_1 = with_requirement(incr, &resource);
    //     let fut_2 = with_requirement(add_j, &resource);
    // }
}
