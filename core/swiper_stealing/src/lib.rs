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
    option::Iter,
    pin::Pin,
    task::{Context, Poll},
};
use std::env::current_dir;

struct LastThiefInfo {}

/// this gets revoked, unsafe to manually interact with outside of PreemptableFuture api
/// currently requires single threaded async execution
/// DOES NOT WORK WITH MULTITHREADED BC CRITICAL SECTIONS FOR ALL ASYNC POLLING NEEDS TO OVERLAP
// TODO replace bool flag with owning task info for introspection
pub struct RevocableCell<T> {
    data: UnsafeCell<T>,
    current_flag: Cell<usize>,
    is_required: Cell<bool>,
}

impl<T> RevocableCell<T> {
    pub fn new(data: T) -> Self {
        Self {
            data: data.into(),
            current_flag: Cell::new(0),
            is_required: Cell::new(false),
        }
    }

    pub fn steal_flag(&self) -> usize {
        // revoke previous flag pointer by incrementing
        self.is_required.set(true);
        let next_flag = self.current_flag.get().wrapping_add(1);
        self.current_flag.set(next_flag);
        next_flag
    }

    pub fn is_required(&self) -> bool {
        self.is_required.get()
    }
}

#[derive(Debug, Clone)]
pub struct PreemptionError;

pub struct PreemptibleFuture<'mutex, F: Future, T, const N: usize> {
    inner: F,
    requirements: [&'mutex RevocableCell<T>; N],
    current_flags: [Option<usize>; N],
}

impl<F: Future, T, const N: usize> Future for PreemptibleFuture<'_, F, T, N> {
    type Output = Result<F::Output, PreemptionError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // pin guarantees all movement sensitive data is not moved
        // in order to extract the fields of the Pin<&mut Self>,
        // the inner representation needs to be extracted
        // and the movement sensitive part (inner Future) needs to be re-pinned
        let instance = unsafe { self.get_unchecked_mut() };
        let inner = unsafe { Pin::new_unchecked(&mut instance.inner) };

        for (requirement, flag) in instance
            .requirements
            .iter()
            .zip(instance.current_flags.iter_mut())
        {
            let current_flag = flag.get_or_insert_with(|| requirement.steal_flag());

            if *current_flag != requirement.current_flag.get() {
                return Poll::Ready(Err(PreemptionError));
            }
        }

        inner.poll(cx).map(Ok)
    }
}

// impl Drop for Future

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
//     ) -> PreemptableFuture<'mutex, Func::CallRefFuture, Requirement>
//     where Func: 'mutex;
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
//         let data = requirement.data.get();
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

    struct CountForever<'a> {
        num: &'a mut i32,
        incr: i32,
    }

    impl Future for CountForever<'_> {
        type Output = i32;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let instance = unsafe { self.get_unchecked_mut() };
            *instance.num += instance.incr;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }

    #[test]
    fn flag_stealing() {
        let cell = RevocableCell::new(0);
        {
            assert!(!cell.is_required());
            let a = cell.steal_flag();
            assert!(cell.is_required());
            assert_eq!(a, cell.current_flag.get());
            // steal new flag
            let b = cell.steal_flag();
            assert_ne!(a, cell.current_flag.get());
            assert_eq!(b, cell.current_flag.get());
            assert!(cell.is_required());
        }
        // assert!(!cell.is_required());
    }

    #[test]
    fn future_mutexing() {
        let resource = RevocableCell::new(0);

        let plus_5 = CountForever {
            num: unsafe { &mut *resource.data.get() },
            incr: 5,
        };
        let minus_1 = CountForever {
            num: unsafe { &mut *resource.data.get() },
            incr: -1,
        };

        let plus_5 = PreemptibleFuture {
            inner: plus_5,
            requirements: [&resource],
            current_flags: [None],
        };
        let minus_1 = PreemptibleFuture {
            inner: minus_1,
            requirements: [&resource],
            current_flags: [None],
        };
    }
}
