use swiper_derive::preemptible;

#[cfg(test)]
mod tests {
    use core::task;
    use std::{ptr, task::{Context, Poll}};

    use futures_lite::future;
    use lite_async_test::async_test;
    use swiper_derive::preemptible;
    use swiper_stealing::{PreemptionError, Revocable, RevocableCell};

    #[test]
    fn basic_preemption() {
        let mut inner: i32 = 0;
        let data = RevocableCell::new(&mut inner, "example");

        #[preemptible(x)]
        async fn increment(x: &mut i32) {
            loop {
                *x += 1;
                future::yield_now().await;
            }
        }

        #[preemptible(x)]
        async fn decrement(x: &mut i32) {
            loop {
                if *x == 0 {
                    return;
                }
                *x -= 1;
                future::yield_now().await;
            }
        }

        let increment = increment(&data);
        let decrement = decrement(&data);

        // start by polling increment
        let mut cx_increment = Context::from_waker(task::Waker::noop());
        let mut pinned_increment = Box::pin(increment);
        let mut cx_decrement = Context::from_waker(task::Waker::noop());
        let mut pinned_decrement = Box::pin(decrement);

        assert!(data.current_owner().is_none());

        for i in 1..6 {
            let res = pinned_increment.as_mut().poll(&mut cx_increment);
            assert!(res.is_pending());
            assert!(data.current_owner().is_some());
            assert_eq!(unsafe { **data.data.get() }, i);
        }

        // now poll decrement
        let res = pinned_decrement.as_mut().poll(&mut cx_decrement);
        assert!(res.is_pending());
        assert_eq!(unsafe { **data.data.get() }, 4);

        // increment should now be cancelled, and should not affect the data value
        let res = pinned_increment.as_mut().poll(&mut cx_increment);
        // assert_eq!(res, Poll::Ready(Result::Err(PreemptionError {})));
        assert!(res.is_ready());
        assert_eq!(unsafe { **data.data.get() }, 4);

        // decrement should be running fine
        for i in (0..4).rev() {
            let res = pinned_decrement.as_mut().poll(&mut cx_decrement);
            assert!(res.is_pending());
            assert!(data.current_owner().is_some());
            assert_eq!(unsafe { **data.data.get() }, i);
        }

        // decrement should stop when i reaches 0
        let res = pinned_decrement.as_mut().poll(&mut cx_decrement);
        assert_eq!(res, Poll::Ready(Result::Ok(())));
        assert_eq!(unsafe { **data.data.get() }, 0);
        assert!(data.current_owner().is_none());
    }

    #[async_test]
    async fn requirement_stealing() {
        async fn wait_ticks(ticks: i32) {
            for _ in 0..ticks {
                future::yield_now().await;
            }
        }

        #[preemptible(data)]
        async fn increment_n_times(data: &mut i32, times: i32) {
            for i in 0..times {
                assert_eq!(*data, i);
                *data += 1;
                future::yield_now().await;
            }
        }

        #[preemptible(data)]
        async fn set(data: &mut i32, val: i32) {
            *data = val;
        }

        #[preemptible(data)]
        async fn data_assert(data: &mut i32, cond: fn(i32) -> bool) {
            assert!(cond(*data));
        }

        let mut x = 0;
        let data = RevocableCell::new(&mut x, "test data");

        let wait_5_then_reset = async || {
            wait_ticks(5).await;
            set(&data, 0).await?;
            data_assert(&data, |x| x == 0).await?;
            Ok::<(), PreemptionError>(())
        };

        let (a, b) = future::zip(wait_5_then_reset(), increment_n_times(&data, 100)).await;
        assert!(a.is_ok()); // a ran to completion
        assert!(b.is_err()); // b got cancelled by set after 5 ticks of a

        // data got reset to 0 in a
        data_assert(&data, |x| x == 0).await.unwrap();

        let res = future::try_zip(wait_5_then_reset(), increment_n_times(&data, 100)).await;
        assert!(res.is_err()); // a preempted b, cancelling both since they are joined
    }
}
