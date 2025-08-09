//! A polished, minimal async executor demonstrating how Tokio and task scheduling work under the hood.

#![allow(unused)]
use futures::task::{ArcWake, waker_ref};
use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

// ===== Task implementation for MiniTokio =====

/// A scheduled task that can be polled.
struct Task {
    future: Mutex<Option<Pin<Box<dyn Future<Output = ()> + Send>>>>, // FutureBox
    task_sender: mpsc::Sender<Arc<Task>>,
}

impl ArcWake for Task {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        let _ = arc_self.task_sender.try_send(arc_self.clone());
    }
}

// ===== MiniTokio Executor =====

struct MiniTokio {
    scheduled: mpsc::Receiver<Arc<Task>>,
    task_count: Arc<std::sync::atomic::AtomicUsize>,
    shutdown_rx: oneshot::Receiver<()>,
}

impl MiniTokio {
    fn new(
        shutdown_rx: oneshot::Receiver<()>,
    ) -> (
        Self,
        mpsc::Sender<Arc<Task>>,
        Arc<std::sync::atomic::AtomicUsize>,
    ) {
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
            tokio::select! {
                biased;

                _ = &mut self.shutdown_rx => {
                    println!("[MiniTokio] Received shutdown signal.");
                    break;
                }

                Some(task) = self.scheduled.recv() => {
                    let mut future_slot = task.future.lock().unwrap();
                    if let Some(mut fut) = future_slot.take() {
                        let waker = waker_ref(&task);
                        let mut cx = Context::from_waker(&waker);
                        match fut.as_mut().poll(&mut cx) {
                            Poll::Pending => {
                                *future_slot = Some(fut);
                            }
                            Poll::Ready(_) => {
                                self.task_count.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                            }
                        }
                    }
                }

                else => {
                    if self.task_count.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                        println!("[MiniTokio] All tasks done.");
                        break;
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
) where
    F: Future<Output = ()> + Send + 'static,
{
    let task = Arc::new(Task {
        future: Mutex::new(Some(Box::pin(future))),
        task_sender: sender.clone(),
    });

    task_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let _ = sender.try_send(task);
}

// ===== LocalSpawner + LocalTask (non-Send example) =====

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

// ===== Async demo tasks =====

async fn async_task(id: usize, delay_ms: u64) {
    println!("[Task-{id}] Started.");
    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    println!("[Task-{id}] Completed.");
}

// ===== Main entry =====

#[tokio::main]
async fn main() {
    // --- Local Spawner test ---
    println!("---- LocalTask Executor ----");
    let spawner = LocalSpawner::new();
    let (tx, rx) = oneshot::channel();
    spawner.spawn(LocalTask::AddOne(10, tx));
    spawner.run();
    let result = rx.await.unwrap();
    println!("Local task result: {result}");

    // --- MiniTokio executor test ---
    println!("---- MiniTokio Executor ----");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (mut mini_tokio, sender, task_count) = MiniTokio::new(shutdown_rx);

    // Schedule async tasks
    for i in 1..=3 {
        spawn(
            async_task(i, i as u64 * 400),
            sender.clone(),
            task_count.clone(),
        );
    }

    // Shutdown trigger: wait until task count reaches zero
    tokio::spawn(async move {
        loop {
            if task_count.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                let _ = shutdown_tx.send(());
                break;
            }
            // Wait for the tasks to be complete just a little bit.
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });

    // Run the mini executor
    mini_tokio.run().await;
    println!("[Main] MiniTokio executor shut down.");
}
