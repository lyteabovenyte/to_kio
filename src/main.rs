//! Minimal async executor (MiniTokio) example.
//! - Demonstrates a tiny executor: Task, ArcWake, scheduling channel, and a run loop.
//! - Includes a small local (non-Send) spawner demo and a few async tasks.

use futures::task::{ArcWake, waker_ref};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

/// A `Task` is a heap-allocated, reference-counted object that owns a
/// future we can poll. We keep the future inside a `Mutex<Option<Pin<Box<...>>>>`
/// so polling can take it out, poll it, and put it back if it's `Pending`.
struct Task {
    /// The future owned by this Task. `Option` so we can `take()` it out,
    /// poll it, and put it back.
    future: Mutex<Option<Pin<Box<dyn Future<Output = ()> + Send>>>>,

    /// Sender used by `wake_by_ref` to re-schedule the task back onto the executor's queue.
    task_sender: mpsc::Sender<Arc<Task>>,
}

impl Task {
    /// Create a new `Arc<Task>` with the provided future and sender.
    fn new<F>(future: F, sender: mpsc::Sender<Arc<Task>>) -> Arc<Task>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        Arc::new(Task {
            future: Mutex::new(Some(Box::pin(future))),
            task_sender: sender,
        })
    }

    /// Convenience to spawn from synchronous context using `try_send`.
    /// If channel is full, we return Err; caller can decide what to do.
    fn spawn_to_executor<F>(
        future: F,
        sender: &mpsc::Sender<Arc<Task>>,
    ) -> Result<(), mpsc::error::TrySendError<Arc<Task>>>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let task = Task::new(future, sender.clone());
        sender.try_send(task)
    }
}

/// Implement `ArcWake` so tasks can create wakers that schedule themselves.
/// `wake_by_ref` pushes the `Arc<Task>` back into the executor's channel.
impl ArcWake for Task {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        // We use try_send so that if the executor's queue is full we don't block here.
        // In production you'd want a more robust backpressure / queueing strategy.
        // I think tokio uses the global queue for this purpose if the local queue is full
        let _ = arc_self.task_sender.try_send(arc_self.clone());
    }
}

/// The executor. Receives scheduled tasks from a channel and polls them.
/// For signalling shutdown we accept a oneshot receiver.
///
/// - `scheduled_rx` is the receiver where tasks appear when scheduled.
/// - `task_count` tracks number of outstanding spawned tasks so we can shut down automatically.
struct MiniTokio {
    scheduled_rx: mpsc::Receiver<Arc<Task>>,
    scheduled_tx: mpsc::Sender<Arc<Task>>,
    task_count: Arc<AtomicUsize>,
    shutdown_rx: oneshot::Receiver<()>,
}

impl MiniTokio {
    fn new(shutdown_rx: oneshot::Receiver<()>) -> Self {
        // channel capacity: tuned moderately high so tasks can re-schedule themselves.
        let (tx, rx) = mpsc::channel::<Arc<Task>>(1000);
        Self {
            scheduled_rx: rx,
            scheduled_tx: tx,
            task_count: Arc::new(AtomicUsize::new(0)),
            shutdown_rx,
        }
    }

    /// Spawn a future onto this mini-executor.
    /// This increments the task counter and attempts to send the created Task into the queue.
    /// We prefer `try_send` to avoid needing an async context here; in a real runtime you'd
    /// probably await send or use a different design.
    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.task_count.fetch_add(1, Ordering::SeqCst);

        // If sending fails (channel full), we decrement the counter so it stays consistent.
        if let Err(e) = Task::spawn_to_executor(future, &self.scheduled_tx) {
            eprintln!("[MiniTokio] spawn send failed: {e:?}");
            self.task_count.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Run the executor. This is an async function that should be polled on a real runtime (e.g., Tokio).
    /// It repeatedly receives tasks from `scheduled_rx` and polls them.
    async fn run(mut self) {
        loop {
            tokio::select! {
                // Shutdown request wins if it arrives.
                _ = &mut self.shutdown_rx => {
                    println!("[MiniTokio] Received shutdown signal.");
                    break;
                }

                // Receive a scheduled task and poll it once.
                maybe_task = self.scheduled_rx.recv() => {
                    match maybe_task {
                        Some(task_arc) => {
                            // Lock the task's future slot.
                            let mut slot = task_arc.future.lock().unwrap();

                            // If there's a future present, take it out to poll.
                            if let Some(mut fut) = slot.take() {
                                // Create waker that will re-schedule `task_arc` when woken.
                                let waker = waker_ref(&task_arc);
                                let mut cx = Context::from_waker(&waker);

                                match fut.as_mut().poll(&mut cx) {
                                    Poll::Pending => {
                                        // put it back to be polled later
                                        *slot = Some(fut);
                                    }
                                    Poll::Ready(()) => {
                                        // Completed -> drop the future (slot remains None).
                                        self.task_count.fetch_sub(1, Ordering::SeqCst);
                                    }
                                }
                            } // else: the slot was empty, nothing to do
                        }
                        None => {
                            // Sender side closed - no further tasks will come in.
                            println!("[MiniTokio] scheduled channel closed.");
                            break;
                        }
                    }
                }

                // If there's nothing to do and all tasks finished, exit.
                // This branch will repeatedly get chosen only when no task arrives and shutdown not received.
                // We check the task_count to determine if we can stop.
                else => {
                    if self.task_count.load(Ordering::SeqCst) == 0 {
                        println!("[MiniTokio] all tasks complete; shutting down executor.");
                        break;
                    }
                    // otherwise loop again and wait for tasks/shutdown
                }
            }
        }
    }

    /// Helper to get a clone of the task_count for external observers.
    fn task_count_handle(&self) -> Arc<AtomicUsize> {
        self.task_count.clone()
    }

    /// Helper to get a sender to schedule tasks from outside (if needed)
    fn schedule_sender(&self) -> mpsc::Sender<Arc<Task>> {
        self.scheduled_tx.clone()
    }
}

// ----------------- Local (single-thread) spawner example -----------------
// This demonstrates how to handle non-Send "local" tasks by running them on the local thread
// (synchronously). Simple toy example using an enum of local tasks.

enum LocalTask {
    AddOne(i32, oneshot::Sender<i32>),
}

struct LocalSpawner {
    queue: std::cell::RefCell<Vec<LocalTask>>,
}

impl LocalSpawner {
    fn new() -> Self {
        Self {
            queue: std::cell::RefCell::new(Vec::new()),
        }
    }

    fn spawn(&self, t: LocalTask) {
        self.queue.borrow_mut().push(t);
    }

    fn run(&self) {
        // Pop and execute synchronously (local-only).
        while let Some(task) = self.queue.borrow_mut().pop() {
            match task {
                LocalTask::AddOne(n, tx) => {
                    let _ = tx.send(n + 1);
                }
            }
        }
    }
}

// ----------------- Demo async tasks -----------------

async fn async_task(id: usize, delay_ms: u64) {
    println!("[Task-{id}] started.");
    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    println!("[Task-{id}] completed.");
}

// ----------------- Main -----------------

#[tokio::main]
async fn main() {
    println!("---- LocalTask Executor demo ----");
    let local_spawner = LocalSpawner::new();
    let (tx, rx) = oneshot::channel::<i32>();
    local_spawner.spawn(LocalTask::AddOne(10, tx));
    local_spawner.run();
    let result = rx.await.expect("local task oneshot closed");
    println!("Local task result: {result}");

    println!("---- MiniTokio Executor demo ----");

    // Create a shutdown channel for the mini-executor.
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    // Build the mini-executor.
    let mini = MiniTokio::new(shutdown_rx);
    let task_count_handle = mini.task_count_handle();
    let schedule_sender = mini.schedule_sender();

    // Spawn some demo tasks onto the mini-executor.
    // You must call `mini.spawn(...)` (synchronous) which uses try_send to push tasks to the queue.
    mini.spawn(async_task(1, 300));
    mini.spawn(async_task(2, 600));
    mini.spawn(async_task(3, 900));

    // Start a background watcher that will send the shutdown signal once task_count hits 0.
    // This runs on the real Tokio runtime (the outer runtime).
    tokio::spawn(async move {
        loop {
            if task_count_handle.load(Ordering::SeqCst) == 0 {
                // It's okay if send fails (receiver might have already been dropped).
                let _ = shutdown_tx.send(());
                break;
            }
            // wait for a little bit to poll the task again
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });

    // Run the mini-executor (this awaits until shutdown or all tasks done).
    mini.run().await;
    println!("[Main] MiniTokio executor shut down.");
}
