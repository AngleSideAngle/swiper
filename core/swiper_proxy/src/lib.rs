use std::{iter, sync::mpsc::Sender};

struct ProxyFuture<Fut, Output>
where
    Fut: Future<Output = Output> + Send,
{
    inner: Fut,
    sender: Sender<Fut>,
}

// impl Future for ProxyFuture {}

pub fn add(left: u64, right: u64) -> u64 {
    let x = {
        let mut x = (1..4).chain(iter::once(4)).collect::<Vec<i32>>();
        x.sort_unstable();
        x
    };

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
