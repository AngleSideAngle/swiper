use crate::{PreemptionError, Result};
use core::{
    fmt::Display,
    pin::Pin,
    ptr,
    task::{Context, Poll},
};

use crate::requirement::Requirement;

/// Contains metadata about a [`PreemptibleFuture`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThiefInfo {
    pub name: &'static str,
}

impl Display for ThiefInfo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Thief {{ name: {} }} ", self.name)
    }
}

/// Wraps a [`Future`] to safetly implement preemption against other [`PreemptibleFuture`] with overlapping [`RevocableCell`] requirements. This provides the following guarantees if used with safe apis:
///
/// - each `RevocableCell` can have at most 1 owner task
/// - each `PreemptibleFuture` task is guaranteed to own all required `RevocableCell` arguments when it is first polled
/// - any `PreemptibleFuture` that no longer has ownership over any of its requirements is cancelled when it is next polled
///
/// See the [module-level documentation](self) for more on the preemption and requirement system.
pub struct PreemptibleFuture<'mutex, Fut, Output, const N: usize>
where
    Fut: Future<Output = Output>,
{
    inner: Fut,
    pub info: ThiefInfo,
    requirements: [&'mutex dyn Requirement; N],
    first_run: bool,
}

impl<'mutex, Fut, Output, const N: usize> PreemptibleFuture<'mutex, Fut, Output, N>
where
    Fut: Future<Output = Output>,
{
    pub fn new(inner: Fut, name: &'static str, requirements: [&'mutex dyn Requirement; N]) -> Self {
        Self {
            inner,
            info: ThiefInfo { name },
            requirements,
            first_run: true,
        }
    }
}

impl<Fut, Output, const N: usize> Future for PreemptibleFuture<'_, Fut, Output, N>
where
    Fut: Future<Output = Output>,
{
    type Output = Result<Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // pin guarantees all movement sensitive data is not moved
        // in order to extract the fields of the Pin<&mut Self>,
        // the inner representation needs to be extracted
        // and the movement sensitive part (inner Future) needs to be re-pinned
        let instance = unsafe { self.get_unchecked_mut() };
        let inner = unsafe { Pin::new_unchecked(&mut instance.inner) };
        let info = unsafe { Pin::new_unchecked(&mut instance.info) }.get_mut();

        // steal ownership of all resources on first run
        // otherwise check if the `current_owner()` of reach resource points to this `ThiefInfo`
        if instance.first_run {
            instance
                .requirements
                .iter()
                .for_each(|req| req.steal_ownership(info));
            instance.first_run = false;
        } else {
            for requirement in instance.requirements {
                // cancel if requirement is owned by a different task or not owned by any task
                // having a requirement not be owned should not actually occur (since it's physically unsafe)
                // but it is a valid state so it must be handled
                let incoming = if let Some(owner) = requirement.current_owner() {
                    if ptr::eq(owner, info) {
                        continue;
                    }
                    Some(*owner)
                } else {
                    None
                };

                let err = PreemptionError {
                    incoming,
                    outgoing: *info,
                    requirement: requirement.info(),
                };
                return Poll::Ready(Err(err));
            }
        }

        // we verified ownership of all resources now
        let res = inner.poll(cx).map(Ok);
        if res.is_ready() {
            for req in instance.requirements {
                req.release_ownership();
            }
        }
        res
    }
}

#[cfg(test)]
mod tests {

    use core::task;

    use crate::requirement::RevocableCell;
    extern crate std;

    use super::*;
    use std::boxed::Box;

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
    fn future_mutexing() {
        let resource = RevocableCell::new(0, "test");

        let plus_5 = CountForever {
            num: unsafe { &mut *resource.data.get() },
            incr: 5,
        };
        let minus_1 = CountForever {
            num: unsafe { &mut *resource.data.get() },
            incr: -1,
        };

        let plus_5 = PreemptibleFuture::new(plus_5, "plus_5", [&resource]);
        let minus_1 = PreemptibleFuture::new(minus_1, "minus_1", [&resource]);

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
        let expected_err = PreemptionError {
            incoming: Some(pinned_minus_1.info),
            outgoing: pinned_plus_5.info,
            requirement: resource.info(),
        };
        let res = pinned_plus_5.as_mut().poll(&mut cx_plus_5);
        assert!(res.is_ready());
        assert_eq!(unsafe { *resource.data.get() }, 9);
        assert_eq!(res, Poll::Ready(Result::Err(expected_err)));

        // minus_1 should still work
        let res = pinned_minus_1.as_mut().poll(&mut cx_minus_1);
        assert!(res.is_pending());
        assert_eq!(unsafe { *resource.data.get() }, 8);
    }
}
