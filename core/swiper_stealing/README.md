# Swiper Stealing

This crate contains definitions for `RevocableCell` and `PreemptibleFuture` structs.
`RevocableCell` is similar to `RefCell` in that it provides borrow checking guarantees at runtime, rather than compile time.
However, unlike `RefCell`, ownership is checked whenever the future is polled, rather than when the data is accessed.
This more coarse model is useful in contexts where long running tasks need exclusive ownership over a piece of hardware, and cannot wait for other tasks to yield ownership.

What this translates to is newly scheduled `PreemptibleFuture` tasks preempt currently running `PreemptibleFuture` tasks with overlapping `RevocableCell` requirements when polled for the first time by stealing access to the requirement.
Any preempted `PreemptibleFuture`, which just had its requirements stolen, will return a `Poll::Ready<swiper_stealing::Result::<T>::Err>` the next time its polled and not poll its inner future.
When a `PreemptibleFuture` finishes execution successfully, it revokes its ownership of all `RevocableCell` requirements.

```rust
# use swiper_stealing::requirement::RevocableCell;
# use futures_lite::future;

let cell = RevocableCell::new(0, "example");
let increment = cell.run("increment", async |x| {
  loop {
    *x += 1;
    future::yield_now().await;
  }
});
```

This crate makes no heap allocations, and is async runtime agnostic.
This means it can be used with various single threaded async runtimes, such as [smol](https://github.com/smol-rs/smol) and [embassy](https://github.com/embassy-rs/embassy).
To maintain the ownership invariant of `RevocableCell`, each `PreemptibleFuture` must not be polled in parallel with another, meaning a `RevocableCell` is `Send`, but not `Sync`.
Because of this, it is unsafe to use `swiper-stealing` with multithreaded async runtimes, such as tokio.
For proper multithreaded functionality with this crate, see `swiper-proxy`.

The only safe api provided by this crate for accessing the contents of a `RevocableCell` is `RevocableCell::run`, which is demonstrated in the above example.
Since rust lacks variadic generics, more complex behavior, such as writing functions that have multiple `RevocableCell` arguments, must be done using the `preemptible` macro provided by the `swiper-derive` crate.

