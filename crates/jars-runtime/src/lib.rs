//! The small executor and actor primitives used by Rust emitted by `jars-core`.

use std::{cell::RefCell, fmt::Display, future::Future, rc::Rc};

use futures::{
    channel::oneshot,
    executor::{LocalPool, LocalSpawner},
    task::LocalSpawnExt,
};

/// Async scheduling pieces used by generated actor implementations. Re-exporting
/// these keeps generated programs dependent only on `jars-runtime`.
pub use futures::{FutureExt, StreamExt, select_biased, stream::FuturesUnordered};

/// An executor capable of running the background tasks for generated actors.
///
/// Generated Java code is generic over this trait, so consumers may provide an
/// executor other than [`Runtime`].  Actors deliberately have no `Send` bound:
/// Java v1 objects are serialized by their mailbox, not shared between threads.
pub trait Spawner: Clone + 'static {
    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + 'static;
}

/// The single-threaded executor used by generated executable entry points.
#[derive(Clone)]
pub struct Runtime {
    pool: Rc<RefCell<LocalPool>>,
    spawner: LocalSpawner,
}

impl Runtime {
    #[must_use]
    pub fn new() -> Self {
        let pool = LocalPool::new();
        let spawner = pool.spawner();
        Self {
            pool: Rc::new(RefCell::new(pool)),
            spawner,
        }
    }

    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.pool.borrow_mut().run_until(future)
    }

    fn local_spawner(&self) -> LocalSpawner {
        self.spawner.clone()
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Spawner for Runtime {
    fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + 'static,
    {
        self.local_spawner()
            .spawn_local(future)
            .expect("the local runtime is still available");
    }
}

/// A failed actor delivery or reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallError {
    ActorStopped,
    ReplyDropped,
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ActorStopped => f.write_str("actor stopped before receiving the request"),
            Self::ReplyDropped => f.write_str("actor stopped before replying to the request"),
        }
    }
}

impl std::error::Error for CallError {}

/// Java execution failures that can cross an actor or static-method boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JavaError {
    Actor(CallError),
    Arithmetic(ArithmeticError),
    NullPointer,
    ClassCast,
    ClassInitializationFailed(&'static str),
}

impl From<CallError> for JavaError {
    fn from(error: CallError) -> Self {
        Self::Actor(error)
    }
}

impl Display for JavaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Actor(error) => error.fmt(f),
            Self::Arithmetic(error) => error.fmt(f),
            Self::NullPointer => f.write_str("null Java reference"),
            Self::ClassCast => f.write_str("invalid Java reference cast"),
            Self::ClassInitializationFailed(class) => {
                write!(f, "class initialization previously failed for {class}")
            }
        }
    }
}

impl std::error::Error for JavaError {}

/// Arithmetic failures that Java programs expose before throwable support exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithmeticError {
    DivisionByZero,
}

impl Display for ArithmeticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DivisionByZero => f.write_str("integer division by zero"),
        }
    }
}

impl std::error::Error for ArithmeticError {}

/// Java `int` division, including the specified wrapping `MIN / -1` case.
pub fn idiv(left: i32, right: i32) -> Result<i32, JavaError> {
    if right == 0 {
        Err(JavaError::Arithmetic(ArithmeticError::DivisionByZero))
    } else {
        Ok(left.wrapping_div(right))
    }
}

/// Java `int` remainder, including the specified wrapping `MIN % -1` case.
pub fn irem(left: i32, right: i32) -> Result<i32, JavaError> {
    if right == 0 {
        Err(JavaError::Arithmetic(ArithmeticError::DivisionByZero))
    } else {
        Ok(left.wrapping_rem(right))
    }
}

/// Java `long` division, including the specified wrapping `MIN / -1` case.
pub fn ldiv(left: i64, right: i64) -> Result<i64, JavaError> {
    if right == 0 {
        Err(JavaError::Arithmetic(ArithmeticError::DivisionByZero))
    } else {
        Ok(left.wrapping_div(right))
    }
}

/// Java `long` remainder, including the specified wrapping `MIN % -1` case.
pub fn lrem(left: i64, right: i64) -> Result<i64, JavaError> {
    if right == 0 {
        Err(JavaError::Arithmetic(ArithmeticError::DivisionByZero))
    } else {
        Ok(left.wrapping_rem(right))
    }
}

#[must_use]
pub fn iushr(value: i32, shift: i32) -> i32 {
    ((value as u32) >> ((shift as u32) & 31)) as i32
}

#[must_use]
pub fn lushr(value: i64, shift: i32) -> i64 {
    ((value as u64) >> ((shift as u32) & 63)) as i64
}

/// A cloneable address for a single serialized actor.
pub struct ActorRef<M> {
    sender: async_channel::Sender<M>,
}

impl<M> Clone for ActorRef<M> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
        }
    }
}

/// The receiving half of an actor's mailbox.
pub struct Mailbox<M> {
    receiver: async_channel::Receiver<M>,
}

/// Makes an unbounded actor mailbox.  The generated actor owns the mailbox;
/// callers retain only the cloneable [`ActorRef`].
pub fn actor_channel<M>() -> (ActorRef<M>, Mailbox<M>) {
    let (sender, receiver) = async_channel::unbounded();
    (ActorRef { sender }, Mailbox { receiver })
}

impl<M> ActorRef<M> {
    pub async fn send(&self, message: M) -> Result<(), CallError> {
        self.sender
            .send(message)
            .await
            .map_err(|_| CallError::ActorStopped)
    }
}

impl<M> Mailbox<M> {
    pub async fn recv(&self) -> Result<M, CallError> {
        self.receiver
            .recv()
            .await
            .map_err(|_| CallError::ActorStopped)
    }
}

/// The sending half of a typed actor-method reply.
pub struct Reply<T>(oneshot::Sender<T>);

/// The receiving half of a typed actor-method reply.
pub struct Response<T>(oneshot::Receiver<T>);

pub fn reply<T>() -> (Reply<T>, Response<T>) {
    let (sender, receiver) = oneshot::channel();
    (Reply(sender), Response(receiver))
}

impl<T> Reply<T> {
    pub fn send(self, value: T) -> Result<(), CallError> {
        self.0.send(value).map_err(|_| CallError::ReplyDropped)
    }
}

impl<T> Response<T> {
    pub async fn recv(self) -> Result<T, CallError> {
        self.0.await.map_err(|_| CallError::ReplyDropped)
    }
}

pub fn println<T: Display>(value: T) {
    std::println!("{value}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailbox_and_reply_preserve_request_order() {
        let runtime = Runtime::new();
        runtime.block_on(async {
            enum Message {
                Add(i32, Reply<i32>),
            }
            let (actor, mailbox) = actor_channel();
            runtime.spawn(async move {
                let mut total = 0;
                while let Ok(Message::Add(value, reply)) = mailbox.recv().await {
                    total += value;
                    let _ = reply.send(total);
                }
            });

            let (first_reply, first_response) = reply();
            let (second_reply, second_response) = reply();
            actor.send(Message::Add(2, first_reply)).await.unwrap();
            actor.send(Message::Add(3, second_reply)).await.unwrap();
            assert_eq!(first_response.recv().await.unwrap(), 2);
            assert_eq!(second_response.recv().await.unwrap(), 5);
        });
    }

    #[test]
    fn closed_actor_and_dropped_reply_are_reported() {
        let runtime = Runtime::new();
        runtime.block_on(async {
            let (actor, mailbox) = actor_channel::<()>();
            drop(mailbox);
            assert_eq!(actor.send(()).await, Err(CallError::ActorStopped));

            let (reply, response) = reply::<i32>();
            drop(reply);
            assert_eq!(response.recv().await, Err(CallError::ReplyDropped));
        });
    }

    #[test]
    fn spawner_trait_accepts_a_delegating_executor() {
        #[derive(Clone)]
        struct Delegating(Runtime);
        impl Spawner for Delegating {
            fn spawn<F>(&self, future: F)
            where
                F: Future<Output = ()> + 'static,
            {
                self.0.spawn(future);
            }
        }

        fn schedule<S: Spawner>(spawner: S, seen: Rc<RefCell<bool>>) {
            spawner.spawn(async move { *seen.borrow_mut() = true });
        }

        let runtime = Runtime::new();
        let seen = Rc::new(RefCell::new(false));
        schedule(Delegating(runtime.clone()), seen.clone());
        runtime.pool.borrow_mut().run_until_stalled();
        assert!(*seen.borrow());
    }

    #[test]
    fn java_integer_helpers_match_wrapping_and_zero_division_rules() {
        assert_eq!(idiv(i32::MIN, -1), Ok(i32::MIN));
        assert_eq!(irem(i32::MIN, -1), Ok(0));
        assert_eq!(ldiv(i64::MIN, -1), Ok(i64::MIN));
        assert_eq!(lrem(i64::MIN, -1), Ok(0));
        assert_eq!(iushr(-1, 1), i32::MAX);
        assert_eq!(lushr(-1, 1), i64::MAX);
        assert_eq!(
            idiv(1, 0),
            Err(JavaError::Arithmetic(ArithmeticError::DivisionByZero))
        );
    }
}
