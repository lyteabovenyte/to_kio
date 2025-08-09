#![feature(prelude_import)]
//! A polished, minimal async executor demonstrating how Tokio and task scheduling work under the hood.
#![allow(unused)]
#[prelude_import]
use std::prelude::rust_2024::*;
#[macro_use]
extern crate std;
use futures::task::{ArcWake, waker_ref};
use std::{
    cell::RefCell, future::Future, pin::Pin, sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};
/// A scheduled task that can be polled.
struct Task {
    future: Mutex<Option<Pin<Box<dyn Future<Output = ()> + Send>>>>,
    task_sender: mpsc::Sender<Arc<Task>>,
}
impl ArcWake for Task {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        let _ = arc_self.task_sender.try_send(arc_self.clone());
    }
}
struct MiniTokio {
    scheduled: mpsc::Receiver<Arc<Task>>,
    task_count: Arc<std::sync::atomic::AtomicUsize>,
    shutdown_rx: oneshot::Receiver<()>,
}
impl MiniTokio {
    fn new(
        shutdown_rx: oneshot::Receiver<()>,
    ) -> (Self, mpsc::Sender<Arc<Task>>, Arc<std::sync::atomic::AtomicUsize>) {
        let (tx, rx) = mpsc::channel(1000);
        let task_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        (
            Self {
                scheduled: rx,
                task_count: task_count.clone(),
                shutdown_rx,
            },
            tx,
            task_count,
        )
    }
    async fn run(&mut self) {
        loop {
            {
                #[doc(hidden)]
                mod __tokio_select_util {
                    pub(super) enum Out<_0, _1> {
                        _0(_0),
                        _1(_1),
                        Disabled,
                    }
                    pub(super) type Mask = u8;
                }
                use ::tokio::macros::support::Future;
                use ::tokio::macros::support::Pin;
                use ::tokio::macros::support::Poll::{Ready, Pending};
                const BRANCHES: u32 = 2;
                let mut disabled: __tokio_select_util::Mask = Default::default();
                if !true {
                    let mask: __tokio_select_util::Mask = 1 << 0;
                    disabled |= mask;
                }
                if !true {
                    let mask: __tokio_select_util::Mask = 1 << 1;
                    disabled |= mask;
                }
                let mut output = {
                    let futures_init = (&mut self.shutdown_rx, self.scheduled.recv());
                    let mut futures = (
                        ::tokio::macros::support::IntoFuture::into_future(
                            futures_init.0,
                        ),
                        ::tokio::macros::support::IntoFuture::into_future(futures_init.1),
                    );
                    let mut futures = &mut futures;
                    ::tokio::macros::support::poll_fn(|cx| {
                            match ::tokio::macros::support::poll_budget_available(cx) {
                                ::core::task::Poll::Ready(t) => t,
                                ::core::task::Poll::Pending => {
                                    return ::core::task::Poll::Pending;
                                }
                            };
                            let mut is_pending = false;
                            let start = 0;
                            for i in 0..BRANCHES {
                                let branch;
                                #[allow(clippy::modulo_one)]
                                {
                                    branch = (start + i) % BRANCHES;
                                }
                                match branch {
                                    #[allow(unreachable_code)]
                                    0 => {
                                        let mask = 1 << branch;
                                        if disabled & mask == mask {
                                            continue;
                                        }
                                        let (fut, ..) = &mut *futures;
                                        let mut fut = unsafe { Pin::new_unchecked(fut) };
                                        let out = match Future::poll(fut, cx) {
                                            Ready(out) => out,
                                            Pending => {
                                                is_pending = true;
                                                continue;
                                            }
                                        };
                                        disabled |= mask;
                                        #[allow(unused_variables)] #[allow(unused_mut)]
                                        match &out {
                                            _ => {}
                                            _ => continue,
                                        }
                                        return Ready(__tokio_select_util::Out::_0(out));
                                    }
                                    #[allow(unreachable_code)]
                                    1 => {
                                        let mask = 1 << branch;
                                        if disabled & mask == mask {
                                            continue;
                                        }
                                        let (_, fut, ..) = &mut *futures;
                                        let mut fut = unsafe { Pin::new_unchecked(fut) };
                                        let out = match Future::poll(fut, cx) {
                                            Ready(out) => out,
                                            Pending => {
                                                is_pending = true;
                                                continue;
                                            }
                                        };
                                        disabled |= mask;
                                        #[allow(unused_variables)] #[allow(unused_mut)]
                                        match &out {
                                            Some(task) => {}
                                            _ => continue,
                                        }
                                        return Ready(__tokio_select_util::Out::_1(out));
                                    }
                                    _ => {
                                        ::core::panicking::panic_fmt(
                                            format_args!(
                                                "internal error: entered unreachable code: {0}",
                                                format_args!(
                                                    "reaching this means there probably is an off by one bug",
                                                ),
                                            ),
                                        );
                                    }
                                }
                            }
                            if is_pending {
                                Pending
                            } else {
                                Ready(__tokio_select_util::Out::Disabled)
                            }
                        })
                        .await
                };
                match output {
                    __tokio_select_util::Out::_0(_) => {
                        {
                            ::std::io::_print(
                                format_args!("[MiniTokio] Received shutdown signal.\n"),
                            );
                        };
                        break;
                    }
                    __tokio_select_util::Out::_1(Some(task)) => {
                        let mut future_slot = task.future.lock().unwrap();
                        if let Some(mut fut) = future_slot.take() {
                            let waker = waker_ref(&task);
                            let mut cx = Context::from_waker(&waker);
                            match fut.as_mut().poll(&mut cx) {
                                Poll::Pending => {
                                    *future_slot = Some(fut);
                                }
                                Poll::Ready(_) => {
                                    self.task_count
                                        .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                                }
                            }
                        }
                    }
                    __tokio_select_util::Out::Disabled => {
                        if self.task_count.load(std::sync::atomic::Ordering::SeqCst) == 0
                        {
                            {
                                ::std::io::_print(
                                    format_args!("[MiniTokio] All tasks done.\n"),
                                );
                            };
                            break;
                        }
                    }
                    _ => {
                        ::core::panicking::panic_fmt(
                            format_args!(
                                "internal error: entered unreachable code: {0}",
                                format_args!("failed to match bind"),
                            ),
                        );
                    }
                }
            }
        }
    }
}
/// Spawn a new future to be run by MiniTokio.
fn spawn<F>(
    future: F,
    sender: mpsc::Sender<Arc<Task>>,
    task_count: Arc<std::sync::atomic::AtomicUsize>,
)
where
    F: Future<Output = ()> + Send + 'static,
{
    let task = Arc::new(Task {
        future: Mutex::new(Some(Box::pin(future))),
        task_sender: sender.clone(),
    });
    task_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let _ = sender.try_send(task);
}
enum LocalTask {
    AddOne(i32, oneshot::Sender<i32>),
}
struct LocalSpawner {
    queue: RefCell<Vec<LocalTask>>,
}
impl LocalSpawner {
    fn new() -> Self {
        Self {
            queue: RefCell::new(Vec::new()),
        }
    }
    fn spawn(&self, task: LocalTask) {
        self.queue.borrow_mut().push(task);
    }
    fn run(&self) {
        while let Some(task) = self.queue.borrow_mut().pop() {
            match task {
                LocalTask::AddOne(n, tx) => {
                    let _ = tx.send(n + 1);
                }
            }
        }
    }
}
async fn async_task(id: usize, delay_ms: u64) {
    {
        ::std::io::_print(format_args!("[Task-{0}] Started.\n", id));
    };
    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    {
        ::std::io::_print(format_args!("[Task-{0}] Completed.\n", id));
    };
}
fn main() {
    let body = async {
        {
            ::std::io::_print(format_args!("---- LocalTask Executor ----\n"));
        };
        let spawner = LocalSpawner::new();
        let (tx, rx) = oneshot::channel();
        spawner.spawn(LocalTask::AddOne(10, tx));
        spawner.run();
        let result = rx.await.unwrap();
        {
            ::std::io::_print(format_args!("Local task result: {0}\n", result));
        };
        {
            ::std::io::_print(format_args!("---- MiniTokio Executor ----\n"));
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (mut mini_tokio, sender, task_count) = MiniTokio::new(shutdown_rx);
        for i in 1..=3 {
            spawn(async_task(i, i as u64 * 400), sender.clone(), task_count.clone());
        }
        tokio::spawn(async move {
            loop {
                if task_count.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                    let _ = shutdown_tx.send(());
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
        mini_tokio.run().await;
        {
            ::std::io::_print(format_args!("[Main] MiniTokio executor shut down.\n"));
        };
    };
    #[allow(
        clippy::expect_used,
        clippy::diverging_sub_expression,
        clippy::needless_return
    )]
    {
        return tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed building the Runtime")
            .block_on(body);
    }
}
