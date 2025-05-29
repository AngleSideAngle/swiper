use swiper_derive::preemptible;


#[preemptible(x)]
async fn idk(x: i32) { x }

#[cfg(test)]
mod tests {
    use swiper_derive::preemptible;

    use super::*;

    #[test]
    fn it_works() {

        #[preemptible(x)]
        async fn idk(x: i32) { x }
    }
}
