use swiper_derive::preemptible;

#[cfg(test)]
mod tests {
    use core::task;
    use std::{
        cell::UnsafeCell,
        pin::Pin,
        task::{Context, Poll},
    };

    use lite_async_test::async_test;
    use swiper_derive::preemptible;
    use swiper_stealing::{PreemptionError, RevocableCell};

    use super::*;

    // dummy future that returns ready the 2nd time it's polled
    struct WaitOnce {
        polled: bool,
    }

    impl WaitOnce {
        fn new() -> Self {
            Self { polled: false }
        }
    }

    impl Future for WaitOnce {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            if self.polled {
                Poll::Ready(())
            } else {
                self.polled = true;
                Poll::Pending
            }
        }
    }

    #[test]
    fn basic_preemption() {
        let mut inner: i32 = 0;
        let data = RevocableCell::new(&mut inner);

        #[preemptible(x)]
        async fn increment(x: &mut i32) {
            loop {
                *x += 1;
                WaitOnce::new().await;
            }
        }

        #[preemptible(x)]
        async fn decrement(x: &mut i32) {
            loop {
                *x -= 1;
                WaitOnce::new().await;
            }
        }

        let increment = increment(&data);
        let decrement = decrement(&data);

        // start by polling increment
        let mut cx_increment = Context::from_waker(task::Waker::noop());
        let mut pinned_increment = Box::pin(increment);
        let mut cx_decrement = Context::from_waker(task::Waker::noop());
        let mut pinned_decrement = Box::pin(decrement);

        for i in 1..6 {
            let res = pinned_increment.as_mut().poll(&mut cx_increment);
            assert!(res.is_pending());
            assert_eq!(unsafe { **data.data.get() }, i);
        }

        // now poll decrement
        let res = pinned_decrement.as_mut().poll(&mut cx_decrement);
        assert!(res.is_pending());
        assert_eq!(unsafe { **data.data.get() }, 4);

        // increment should now be cancelled, and should not affect the data value
        let res = pinned_increment.as_mut().poll(&mut cx_increment);
        assert_eq!(res, Poll::Ready(Result::Err(PreemptionError {})));
        assert_eq!(unsafe { **data.data.get() }, 4);

        // decrement should be running fine
        for i in (0..4).rev() {
            let res = pinned_decrement.as_mut().poll(&mut cx_decrement);
            assert!(res.is_pending());
            assert_eq!(unsafe { **data.data.get() }, i);
        }

    }
}
