//! Asynchronous runtime glue. The GUI owns the executor (the GLib main loop
//! in the GTK interface, a local pool polled every frame in the egui
//! interface) and registers it with [`set_spawner`]; platform code spawns
//! its tasks with [`spawn_local`]. Timers come from `async-io`, whose
//! reactor runs in its own thread and wakes tasks of any executor.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// A task of the GUI thread.
pub type LocalTask = Pin<Box<dyn Future<Output = ()> + 'static>>;

thread_local! {
    static SPAWNER: RefCell<Option<Box<dyn Fn(LocalTask)>>> = const { RefCell::new(None) };
}

/// Register the executor of the GUI thread.
pub fn set_spawner(f: impl Fn(LocalTask) + 'static) {
    SPAWNER.with(|s| *s.borrow_mut() = Some(Box::new(f)));
}

/// Run a task on the GUI thread.
///
/// # Panics
/// If no executor was registered with [`set_spawner`] on this thread.
pub fn spawn_local(fut: impl Future<Output = ()> + 'static) {
    SPAWNER.with(|s| match s.borrow().as_ref() {
        Some(spawn) => spawn(Box::pin(fut)),
        None => panic!("pwtool_app::rt::spawn_local: no executor registered on this thread"),
    });
}

/// Wait for the given time.
pub async fn sleep(d: Duration) {
    async_io::Timer::after(d).await;
}
