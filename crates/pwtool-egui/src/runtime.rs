//! Executor of the GUI thread: tasks (dialogs that wait for the user,
//! timers, portal requests) are polled in every frame; waking a task asks
//! egui for a new frame. Registered with `pwtool_app::rt`, so the platform
//! services spawn their tasks here too.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};

type Task = Pin<Box<dyn Future<Output = ()>>>;

struct TaskWaker {
    woken: AtomicBool,
    ctx: Mutex<Option<egui::Context>>,
}

impl Wake for TaskWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.store(true, Ordering::SeqCst);
        if let Some(ctx) = self.ctx.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
    }
}

struct Entry {
    task: Task,
    waker: Arc<TaskWaker>,
}

thread_local! {
    static TASKS: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    static NEW: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) };
    static CTX: RefCell<Option<egui::Context>> = const { RefCell::new(None) };
}

/// Run a task on the GUI thread.
pub fn spawn(fut: impl Future<Output = ()> + 'static) {
    let ctx = CTX.with(|c| c.borrow().clone());
    let waker = Arc::new(TaskWaker { woken: AtomicBool::new(true), ctx: Mutex::new(ctx.clone()) });
    NEW.with(|n| n.borrow_mut().push(Entry { task: Box::pin(fut), waker }));
    if let Some(c) = ctx {
        c.request_repaint();
    }
}

/// Install the executor for this thread (also for `pwtool_app::rt`).
pub fn install(ctx: &egui::Context) {
    CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
    pwtool_app::rt::set_spawner(|t| spawn(t));
}

/// Poll the woken tasks; called once per frame. Tasks spawned while
/// polling are polled in the same call.
pub fn poll() {
    let ctx = CTX.with(|c| c.borrow().clone());
    for _ in 0..8 {
        let mut tasks = TASKS.with(|t| std::mem::take(&mut *t.borrow_mut()));
        let new = NEW.with(|n| std::mem::take(&mut *n.borrow_mut()));
        for e in &new {
            *e.waker.ctx.lock().unwrap() = ctx.clone();
        }
        tasks.extend(new);
        let mut keep = Vec::with_capacity(tasks.len());
        let mut polled = 0;
        for mut e in tasks {
            if !e.waker.woken.swap(false, Ordering::SeqCst) {
                keep.push(e);
                continue;
            }
            polled += 1;
            let waker = Waker::from(e.waker.clone());
            let mut cx = Context::from_waker(&waker);
            if e.task.as_mut().poll(&mut cx) == Poll::Pending {
                keep.push(e);
            }
        }
        TASKS.with(|t| {
            let mut t = t.borrow_mut();
            keep.append(&mut t);
            *t = keep;
        });
        let more = NEW.with(|n| !n.borrow().is_empty());
        if polled == 0 && !more {
            break;
        }
    }
}

/// Number of unfinished tasks.
pub fn pending() -> usize {
    TASKS.with(|t| t.borrow().len()) + NEW.with(|n| n.borrow().len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn tasks_and_channels() {
        let done = Rc::new(Cell::new(0));
        let (tx, rx) = async_channel::unbounded::<u32>();
        let d = done.clone();
        spawn(async move {
            while let Ok(v) = rx.recv().await {
                d.set(d.get() + v);
            }
        });
        poll();
        assert_eq!(done.get(), 0);
        tx.try_send(5).unwrap();
        poll();
        assert_eq!(done.get(), 5);
        drop(tx);
        poll();
        assert_eq!(pending(), 0);
    }
}
