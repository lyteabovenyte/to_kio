//! Demonstrates how to implement a (very) basic asynchronous rust executor and
//! timer.
#![allow(unused_variables)]
#![allow(dead_code)]
#![allow(unused_mut)]
#![allow(clippy::await_holding_refcell_ref)]
use tokio::runtime::Builder;
use tokio::sync::{mpsc, oneshot};
use tokio::task::LocalSet;

use futures::future::BoxFuture;
use futures::task::{self, ArcWake};
use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    // Test LocalSpawner
    let spawner = LocalSpawner::new();
    let (send, response) = oneshot::channel();
    let local_task = LocalTask::AddOne(10, send);
    spawner.spawn(local_task);
    let eleven = response.await.unwrap();
    assert_eq!(eleven, 11);

    // Test MiniTokio
    println!("------ MiniTokio Task and Scheduler test -------");
    let mut mini_tokio = MiniTokio::new();

    // Spawn tasks
    mini_tokio.spawn(async {
        spawn(async {
            delay(Duration::from_millis(100)).await;
            println!("world");
        });

        spawn(async {
            println!("hello");
        });
    });

    // Setup shutdown after 200ms
    let (shutdown_send, shutdown_recv) = oneshot::channel();
    mini_tokio.spawn(async move {
        delay(Duration::from_millis(200)).await;
        let _ = shutdown_send.send(());
    });

    // Run with shutdown capability
    tokio::select! {
        _ = mini_tokio.run() => {},
        _ = shutdown_recv => {
            println!("MiniTokio shutdown complete");
        }
    }
}

struct MiniTokio {
    scheduled: mpsc::Receiver<Arc<Task>>,
    sender: mpsc::Sender<Arc<Task>>,
    shutdown: (mpsc::Sender<()>, mpsc::Receiver<()>),
}

impl MiniTokio {
    fn new() -> MiniTokio {
        let (sender, scheduled) = mpsc::channel(20);
        let (shutdown_sender, shutdown_receiver) = mpsc::channel(1);
        MiniTokio {
            scheduled,
            sender,
            shutdown: (shutdown_sender, shutdown_receiver),
        }
    }

    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        Task::spawn(future, &self.sender);
    }

    async fn run(&mut self) {
        CURRENT.with(|cell| {
            *cell.borrow_mut() = Some(self.sender.clone());
        });

        tokio::select! {
            _ = async {
                while let Some(task) = self.scheduled.recv().await {
                    task.poll();
                }
            } => {},
            _ = self.shutdown.1.recv() => {
                println!("MiniTokio Runtime has been shutdown.");
            }
        }
    }

    fn shutdown(&self) {
        let _ = self.shutdown.0.send(());
    }
}

thread_local! {
    static CURRENT: RefCell<Option<mpsc::Sender<Arc<Task>>>> = RefCell::new(None);
}

pub fn spawn<F>(future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    CURRENT.with(|cell| {
        let borrow = cell.borrow();
        let sender = borrow.as_ref().unwrap();
        Task::spawn(future, sender);
    });
}

async fn delay(dur: Duration) {
    struct Delay {
        when: Instant,
        waker: Option<Arc<Mutex<Waker>>>,
    }

    impl Future for Delay {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if let Some(waker) = &self.waker {
                let mut waker = waker.lock().unwrap();
                if !waker.will_wake(cx.waker()) {
                    *waker = cx.waker().clone();
                }
            } else {
                let when = self.when;
                let waker = Arc::new(Mutex::new(cx.waker().clone()));
                self.waker = Some(waker.clone());

                thread::spawn(move || {
                    let now = Instant::now();
                    if now < when {
                        thread::sleep(when - now);
                    }
                    let waker = waker.lock().unwrap();
                    waker.wake_by_ref();
                });
            }

            if Instant::now() >= self.when {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }
    }

    let future = Delay {
        when: Instant::now() + dur,
        waker: None,
    };
    future.await;
}

struct Task {
    future: Mutex<BoxFuture<'static, ()>>,
    executor: mpsc::Sender<Arc<Task>>,
}

impl Task {
    fn spawn<F>(future: F, sender: &mpsc::Sender<Arc<Task>>)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let task = Arc::new(Task {
            future: Mutex::new(Box::pin(future)),
            executor: sender.clone(),
        });
        let _ = sender.send(task);
    }

    fn poll(self: Arc<Self>) {
        let waker = task::waker(self.clone());
        let mut cx = Context::from_waker(&waker);
        let mut future = self.future.try_lock().unwrap();

        match future.as_mut().poll(&mut cx) {
            Poll::Ready(()) => {}
            Poll::Pending => {}
        }
    }
}

impl ArcWake for Task {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        arc_self.executor.send(arc_self.clone());
    }
}

#[derive(Debug)]
enum LocalTask {
    PrintNumber(u32),
    AddOne(u32, oneshot::Sender<u32>),
}

#[derive(Clone)]
struct LocalSpawner {
    send: mpsc::UnboundedSender<LocalTask>,
}

impl LocalSpawner {
    pub fn new() -> Self {
        let (send, mut recv) = mpsc::unbounded_channel();
        let rt = Builder::new_current_thread().enable_all().build().unwrap();

        std::thread::spawn(move || {
            let local = LocalSet::new();
            local.spawn_local(async move {
                while let Some(new_task) = recv.recv().await {
                    tokio::task::spawn_local(run_task(new_task));
                }
            });
            rt.block_on(local);
        });

        Self { send }
    }

    pub fn spawn(&self, task: LocalTask) {
        self.send.send(task).expect("LocalSet thread has shut down");
    }
}

async fn run_task(task: LocalTask) {
    match task {
        LocalTask::PrintNumber(n) => println!("{}", n),
        LocalTask::AddOne(n, response) => {
            let _ = response.send(n + 1);
        }
    }
}
