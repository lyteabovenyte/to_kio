#### tokio crate for acynchronous functionality

##### Notes: 
- tokio just cares about the `task`s that you `spawn`ed, not all the inner `Future`s of the tesk, and calls the `poll` method on them to get the `Poll` enum back which could be either `Reade<T>` or `Pending`, the latter one return a `Waker` and hand to the `Runtime` to be polled again when it's ready.

- an example of implementing the `Future` trait for a type:

```rust
struct Delay {
    when: Instant,
}

impl Future for Delay {
    type Output = &'static str;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if Instant::now() >= self.when {
            println!("Hello there.");
            Poll::Ready("done")
        } else {
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}
```

- Rust Futures are *State Machines*:

```rust
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

enum MainFuture {
    // Initialized, never polled
    State0,
    // Waiting on `Delay`, i.e. the `future.await` line.
    State1(Delay),
    // The future has completed.
    Terminated,
}

impl Future for MainFuture {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>)
        -> Poll<()>
    {
        use MainFuture::*;

        loop {
            match *self {
                State0 => {
                    let when = Instant::now() +
                        Duration::from_millis(10);
                    let future = Delay { when };
                    *self = State1(future);
                }
                State1(ref mut my_future) => {
                    match Pin::new(my_future).poll(cx) {
                        Poll::Ready(out) => {
                            assert_eq!(out, "done");
                            *self = Terminated;
                            return Poll::Ready(());
                        }
                        Poll::Pending => {
                            return Poll::Pending;
                        }
                    }
                }
                Terminated => {
                    panic!("future polled after completion")
                }
            }
        }
    }
}
```

- let this be here for now:

```rust
use futures::task::{ArcWake, waker_ref};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, mpsc};
use std::task::{Context, Poll};

// Define a Task struct which wraps the future and an executor channel sender
struct Task {
    future: Mutex<Pin<Box<dyn Future<Output = ()> + Send>>>,
    task_sender: mpsc::Sender<Arc<Task>>,
}

impl Task {
    // Schedule the task by sending it to the executor queue
    fn schedule(self: &Arc<Self>) {
        let _ = self.task_sender.send(self.clone());
    }

    // Poll the wrapped future, passing the Context with the Waker
    fn poll(self: Arc<Self>) {
        // Create a Waker from this task
        let waker = waker_ref(&self);
        let mut cx = Context::from_waker(&*waker);

        // Lock and poll the future
        let mut future = self.future.lock().unwrap();
        if let Poll::Pending = future.as_mut().poll(&mut cx) {
            // Future is not ready, it will call waker.wake() when ready
        }
        // If ready, the future completes and task won't be rescheduled
    }

    // Spawn a new task from a Future
    fn spawn(future: impl Future<Output = ()> + Send + 'static, sender: mpsc::Sender<Arc<Task>>) {
        let task = Arc::new(Task {
            future: Mutex::new(Box::pin(future)),
            task_sender: sender,
        });
        task.schedule(); // Schedule initial polling
    }
}

// Implement ArcWake so that when waker.wake() is called, it reschedules the task
impl ArcWake for Task {
    fn wake_by_ref(arc_self: &Arc<Self>) {
        arc_self.schedule();
    }
}

fn main() {
    // Create a channel for scheduling tasks
    let (sender, receiver) = mpsc::channel::<Arc<Task>>();

    // Spawn a simple async task
    Task::spawn(
        async {
            println!("Hello from async task!");
        },
        sender.clone(),
    );

    // Simple executor loop: receive tasks and poll them
    while let Ok(task) = receiver.recv() {
        task.poll();
    }
}

```

- `Task` in tokio:
every `tokio::spawn` spawns a new task which is being run until you await it and take ownership of the final reuslt.
```rust
tokio::spawn(async {
    // this runs even if no one await it.
    // to we should keep the JoinHandle<T> that is returned
    // to be notified about the execution and get the Future
    // using await -> it returns Result<T, tokio::task::JoinError>
})

// JoinError is something like this:
enum JoinError {
    Cancellede,
    panic<Box<dyn Any + Send + 'static>
}
```

- At a lower level, the JoinHandle<T> holds:
    - A pointer to the heap-allocated task (Task<T>), which includes:
        - The task’s future (Pin<Box<dyn Future>>)
        - Task state (scheduled, running, completed)
        - Waker and context information

- calling an async function returns a anonymous type that implements the `Future` trait. the `Future` trait is something like this. 
```rust
use std::pin::Pin;
use std::task::{Context, Poll};

pub trait Future {
    type Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context)
        -> Poll<Self::Output>;
}
```

- Forgetting to wake a task after returning `Poll::Pending` is a common source of bugs.

- 






