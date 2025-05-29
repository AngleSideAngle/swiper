#![cfg_attr(not(feature = "std"), no_std)]
#![doc = include_str!("../README.md")]

use core::{
    cell::{Cell, UnsafeCell},
    fmt::Display,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

pub struct FlagHolder {
    name: str,
}

/// Allows a "flag" to be stolen or released by a flag holder.
///
/// Flag owners must ensure they have continued access by checking their assigned `usize` against the current counter value provided by [`get_counter`](Self::get_counter).
///
/// This provides a type-independent reference to downcast `RevocableCell<T>` into.
pub trait Revocable {
    /// Increments the internal counter by 1 (wrapping addition) and returns the new value.
    ///
    /// This also registers a new flag holder.
    /// TODO replace Cell<bool> with owner info
    fn steal_flag(&self) -> usize;

    /// Releases the current flag owner.
    fn release_flag(&self);

    /// Returns the current value of the internal counter.
    fn get_counter(&self) -> usize;

    /// Gets information on the current flag holder.
    fn is_required(&self) -> bool;
}

/// A pointer to a mutable location in memory that enables reference holders to call [`steal_flag()`](Self::steal_flag) to revoke flags from other reference holders.
///
/// This struct cannot be directly used in a safe manner, and must be accessed inside a [`PreemptibleFuture`].
pub struct RevocableCell<T> {
    pub data: UnsafeCell<T>,
    current_flag: Cell<usize>,
    is_required: Cell<bool>,
}

impl<T> RevocableCell<T> {
    /// Creates a new [`RevocableCell`] with ownership of `data`.
    ///
    /// The cell will default having no owner (eg. is_required -> false).
    pub fn new(data: T) -> Self {
        Self {
            data: data.into(),
            current_flag: Cell::new(0),
            is_required: Cell::new(false),
        }
    }
}

impl<T> Revocable for RevocableCell<T> {
    fn steal_flag(&self) -> usize {
        self.is_required.set(true);
        let next_flag = self.current_flag.get().wrapping_add(1);
        self.current_flag.set(next_flag);
        next_flag
    }

    fn release_flag(&self) {
        self.is_required.set(false);
    }

    fn get_counter(&self) -> usize {
        self.current_flag.get()
    }

    fn is_required(&self) -> bool {
        self.is_required.get()
    }
}

impl<T> Revocable for &RevocableCell<T> {
    fn steal_flag(&self) -> usize {
        (**self).steal_flag()
    }

    fn release_flag(&self) {
        (**self).release_flag();
    }

    fn get_counter(&self) -> usize {
        (**self).get_counter()
    }

    fn is_required(&self) -> bool {
        (**self).is_required()
    }
}

#[derive(Debug, Clone)]
pub struct PreemptionError;

impl Display for PreemptionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "task was preempted by another task stealing its resource"
        )
        // TODO store, report specific resources
    }
}

impl PartialEq for PreemptionError {
    fn eq(&self, other: &Self) -> bool {
        true
        // TODO fix type
    }
}

/// Wraps a [`Future`] to safetly implement preemption against other [`PreemptibleFuture`] with overlapping [`RevocableCell`] requirements.
///
/// See the [module-level documentation](self) for more on the preemption and requirement system.
pub struct PreemptibleFuture<'mutex, Fut, Output, const N: usize>
where
    Fut: Future<Output = Output>,
{
    inner: Fut,
    requirements: [&'mutex dyn Revocable; N],
    current_flags: [Option<usize>; N],
}

impl<'mutex, Fut, Output, const N: usize> PreemptibleFuture<'mutex, Fut, Output, N>
where
    Fut: Future<Output = Output>,
{
    pub fn new(inner: Fut, requirements: [&'mutex dyn Revocable; N]) -> Self {
        Self {
            inner,
            requirements,
            current_flags: [None; N],
        }
    }

    // fn from_fn<Args: Tuple, Fun: AsyncFnMut(Args) -> Output>(
    //     fun: Fun,
    //     requirements: [&'mutex dyn Revocable; N],
    // ) -> Self {
    //     Self {
    //         inner: fun(),
    //         requirements,
    //         current_flags: [None; N],
    //     }
    // }
}

impl<Fut, Output, const N: usize> Future for PreemptibleFuture<'_, Fut, Output, N>
where
    Fut: Future<Output = Output>,
{
    type Output = Result<Output, PreemptionError>;

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

            if *current_flag != requirement.get_counter() {
                return Poll::Ready(Err(PreemptionError));
            }
        }

        let res = inner.poll(cx).map(Ok);
        if res.is_ready() {
            for req in instance.requirements {
                req.release_flag();
            }
        }
        res
    }
}

#[cfg(test)]
mod tests {

    use core::task;

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
            current_flags: Default::default(),
        };
        let minus_1 = PreemptibleFuture {
            inner: minus_1,
            requirements: [&resource],
            current_flags: Default::default(),
        };

        // start by polling plus_5
        let mut cx_plus_5 = Context::from_waker(task::Waker::noop());
        let mut pinned_plus_5 = Box::pin(plus_5);
        let res = pinned_plus_5.as_mut().poll(&mut cx_plus_5);
        assert!(res.is_pending());
        assert_eq!(unsafe { *resource.data.get() }, 5);
        let res = pinned_plus_5.as_mut().poll(&mut cx_plus_5);
        assert!(res.is_pending());
        assert_eq!(unsafe { *resource.data.get() }, 10);

        // now poll minus_1, this should steal from plus_5
        let mut cx_minus_1 = Context::from_waker(task::Waker::noop());
        let mut pinned_minus_1 = Box::pin(minus_1);
        let res = pinned_minus_1.as_mut().poll(&mut cx_minus_1);
        assert!(res.is_pending());
        assert_eq!(unsafe { *resource.data.get() }, 9);

        // poll plus_5 again, should finish with preemption error
        let res = pinned_plus_5.as_mut().poll(&mut cx_plus_5);
        assert!(res.is_ready());
        assert_eq!(unsafe { *resource.data.get() }, 9);
        assert_eq!(res, Poll::Ready(Result::Err(PreemptionError {})));

        // minus_1 should still work
        let res = pinned_minus_1.as_mut().poll(&mut cx_minus_1);
        assert!(res.is_pending());
        assert_eq!(unsafe { *resource.data.get() }, 8);
    }
}
