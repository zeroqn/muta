use std::future::Future;
use std::time::Duration;

use futures::future;

pub async fn timeout<T, E>(
    duration: Duration,
    fut: impl Future<Output = Result<T, E>>,
) -> Result<T, anyhow::Error>
where
    E: Into<anyhow::Error> + Send + Sync + 'static,
{
    futures::pin_mut!(fut);

    match future::select(fut, futures_timer::Delay::new(duration)).await {
        future::Either::Left((ret, _timeout)) => ret.map_err(Into::into),
        future::Either::Right((_unresolved, _timeout)) => {
            Err(std::io::Error::from(std::io::ErrorKind::TimedOut).into())
        }
    }
}
