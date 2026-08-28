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

/// The result type used at every generated Java boundary.  The concrete error
/// remains downcastable, so generated exception tables can select Java catch
/// clauses without giving the generated program a bytecode runtime.
pub type JavaResult<T> = anyhow::Result<T>;
pub type JavaError = anyhow::Error;

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

macro_rules! java_error {
    ($name:ident, $text:literal) => {
        #[derive(Debug)]
        pub struct $name;

        impl Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str($text)
            }
        }

        impl std::error::Error for $name {}
    };
}

java_error!(ArithmeticException, "integer division by zero");
java_error!(NullPointerException, "null Java reference");
java_error!(ClassCastException, "invalid Java reference cast");
java_error!(ArrayIndexOutOfBoundsException, "array index out of bounds");
java_error!(NegativeArraySizeException, "negative Java array size");
java_error!(ArrayStoreException, "invalid Java reference array store");

/// Returned after an earlier `<clinit>` failed.  The first caller sees the
/// original error; later active uses see this cached failure instead.
#[derive(Debug)]
pub struct ClassInitializationFailed(pub &'static str);

impl Display for ClassInitializationFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "class initialization previously failed for {}", self.0)
    }
}

impl std::error::Error for ClassInitializationFailed {}

pub fn null_pointer() -> anyhow::Error {
    NullPointerException.into()
}
pub fn class_cast() -> anyhow::Error {
    ClassCastException.into()
}
pub fn class_initialization_failed(class: &'static str) -> anyhow::Error {
    ClassInitializationFailed(class).into()
}
pub fn array_index_out_of_bounds() -> anyhow::Error {
    ArrayIndexOutOfBoundsException.into()
}
pub fn negative_array_size() -> anyhow::Error {
    NegativeArraySizeException.into()
}
pub fn array_store() -> anyhow::Error {
    ArrayStoreException.into()
}

/// Matches the closed runtime throwable hierarchy without reflection.  The AOT
/// compiler embeds only the requested class name from an exception table.
pub fn catches(error: &JavaError, class: &str) -> bool {
    match class {
        "java/lang/ArithmeticException" => error.is::<ArithmeticException>(),
        "java/lang/NullPointerException" => error.is::<NullPointerException>(),
        "java/lang/ClassCastException" => error.is::<ClassCastException>(),
        "java/lang/ArrayIndexOutOfBoundsException" => error.is::<ArrayIndexOutOfBoundsException>(),
        "java/lang/NegativeArraySizeException" => error.is::<NegativeArraySizeException>(),
        "java/lang/ArrayStoreException" => error.is::<ArrayStoreException>(),
        "java/lang/Exception" | "java/lang/RuntimeException" | "java/lang/Throwable" => {
            error.is::<ArithmeticException>()
                || error.is::<NullPointerException>()
                || error.is::<ClassCastException>()
                || error.is::<ArrayIndexOutOfBoundsException>()
                || error.is::<NegativeArraySizeException>()
                || error.is::<ArrayStoreException>()
        }
        _ => false,
    }
}

/// Java `int` division, including the specified wrapping `MIN / -1` case.
pub fn idiv(left: i32, right: i32) -> JavaResult<i32> {
    if right == 0 {
        Err(ArithmeticException.into())
    } else {
        Ok(left.wrapping_div(right))
    }
}

/// Java `int` remainder, including the specified wrapping `MIN % -1` case.
pub fn irem(left: i32, right: i32) -> JavaResult<i32> {
    if right == 0 {
        Err(ArithmeticException.into())
    } else {
        Ok(left.wrapping_rem(right))
    }
}

/// Java `long` division, including the specified wrapping `MIN / -1` case.
pub fn ldiv(left: i64, right: i64) -> JavaResult<i64> {
    if right == 0 {
        Err(ArithmeticException.into())
    } else {
        Ok(left.wrapping_div(right))
    }
}

/// Java `long` remainder, including the specified wrapping `MIN % -1` case.
pub fn lrem(left: i64, right: i64) -> JavaResult<i64> {
    if right == 0 {
        Err(ArithmeticException.into())
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
    identity: Rc<()>,
}

impl<M> Clone for ActorRef<M> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            identity: self.identity.clone(),
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
    (
        ActorRef {
            sender,
            identity: Rc::new(()),
        },
        Mailbox { receiver },
    )
}

impl<M> ActorRef<M> {
    /// Compares actor addresses without observing or exposing their state.
    #[must_use]
    pub fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.identity, &other.identity)
    }

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

/// A typed Java array address.  Arrays are actors just like generated object
/// instances: aliases can cross object actors, but the mutable elements never
/// leave this mailbox implementation.
pub struct JavaArray<T> {
    actor: ActorRef<ArrayMessage<T>>,
}

impl<T> Clone for JavaArray<T> {
    fn clone(&self) -> Self {
        Self {
            actor: self.actor.clone(),
        }
    }
}

enum ArrayMessage<T> {
    Length {
        reply: Reply<JavaResult<i32>>,
    },
    Get {
        index: i32,
        reply: Reply<JavaResult<T>>,
    },
    Set {
        index: i32,
        value: T,
        reply: Reply<JavaResult<()>>,
    },
}

impl<T: Clone + 'static> JavaArray<T> {
    /// Java reference identity for two typed array addresses.
    #[must_use]
    pub fn same(&self, other: &Self) -> bool {
        self.actor.same(&other.actor)
    }

    /// Creates a default-filled Java array.  Negative sizes use the Java
    /// failure rather than Rust's allocation diagnostics.
    pub fn new<S: Spawner>(spawner: S, length: i32, default: T) -> JavaResult<Self> {
        if length < 0 {
            return Err(negative_array_size());
        }
        let (actor, mailbox) = actor_channel();
        spawner.spawn(async move {
            use futures::{FutureExt as _, StreamExt as _};
            let state = Rc::new(std::sync::Mutex::new(vec![default; length as usize]));
            let mut in_flight: FuturesUnordered<std::pin::Pin<Box<dyn Future<Output = ()>>>> =
                FuturesUnordered::new();
            loop {
                let spawn_handler =
                    |message: ArrayMessage<T>, state: Rc<std::sync::Mutex<Vec<T>>>| {
                        Box::pin(async move {
                            match message {
                                ArrayMessage::Length { reply } => {
                                    let length =
                                        state.lock().expect("array state mutex").len() as i32;
                                    let _ = reply.send(Ok(length));
                                }
                                ArrayMessage::Get { index, reply } => {
                                    let value = if index < 0 {
                                        Err(array_index_out_of_bounds())
                                    } else {
                                        state
                                            .lock()
                                            .expect("array state mutex")
                                            .get(index as usize)
                                            .cloned()
                                            .ok_or_else(array_index_out_of_bounds)
                                    };
                                    let _ = reply.send(value);
                                }
                                ArrayMessage::Set {
                                    index,
                                    value,
                                    reply,
                                } => {
                                    let result = if index < 0 {
                                        Err(array_index_out_of_bounds())
                                    } else {
                                        let mut elements = state.lock().expect("array state mutex");
                                        match elements.get_mut(index as usize) {
                                            Some(slot) => {
                                                *slot = value;
                                                Ok(())
                                            }
                                            None => Err(array_index_out_of_bounds()),
                                        }
                                    };
                                    let _ = reply.send(result);
                                }
                            }
                        }) as std::pin::Pin<Box<dyn Future<Output = ()>>>
                    };
                if in_flight.is_empty() {
                    let message = match mailbox.recv().await {
                        Ok(message) => message,
                        Err(_) => break,
                    };
                    in_flight.push(spawn_handler(message, state.clone()));
                } else {
                    select_biased! {
                        message = mailbox.recv().fuse() => match message {
                            Ok(message) => in_flight.push(spawn_handler(message, state.clone())),
                            Err(_) => break,
                        },
                        _ = in_flight.next().fuse() => {},
                    }
                }
            }
        });
        Ok(Self { actor })
    }

    pub async fn length(&self) -> JavaResult<i32> {
        let (reply, response) = reply();
        self.actor.send(ArrayMessage::Length { reply }).await?;
        response.recv().await?
    }

    pub async fn get(&self, index: i32) -> JavaResult<T> {
        let (reply, response) = reply();
        self.actor.send(ArrayMessage::Get { index, reply }).await?;
        response.recv().await?
    }

    pub async fn set(&self, index: i32, value: T) -> JavaResult<()> {
        let (reply, response) = reply();
        self.actor
            .send(ArrayMessage::Set {
                index,
                value,
                reply,
            })
            .await?;
        response.recv().await?
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

/// The Rust representation of `java.io.PrintStream` values such as
/// `System.out`.  It carries no state: all instances compare equal, matching
/// the single process-wide stream Java code observes.
#[derive(Debug, Clone, Copy)]
pub struct PrintStream;

impl PrintStream {
    #[must_use]
    pub fn __same(&self, _other: &Self) -> bool {
        true
    }
}

/// Rust implementations backing `java.lang.Math` static methods.
pub mod math {
    #[must_use]
    pub fn min(a: i32, b: i32) -> i32 {
        a.min(b)
    }
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
        assert_eq!(idiv(i32::MIN, -1).unwrap(), i32::MIN);
        assert_eq!(irem(i32::MIN, -1).unwrap(), 0);
        assert_eq!(ldiv(i64::MIN, -1).unwrap(), i64::MIN);
        assert_eq!(lrem(i64::MIN, -1).unwrap(), 0);
        assert_eq!(iushr(-1, 1), i32::MAX);
        assert_eq!(lushr(-1, 1), i64::MAX);
        assert!(idiv(1, 0).unwrap_err().is::<ArithmeticException>());
    }

    #[test]
    fn arrays_serialize_mutation_inside_their_actor() {
        let runtime = Runtime::new();
        runtime.block_on(async {
            let array = JavaArray::new(runtime.clone(), 2, 0_i32).unwrap();
            array.set(0, 40).await.unwrap();
            array.set(1, 2).await.unwrap();
            assert_eq!(array.length().await.unwrap(), 2);
            assert_eq!(
                array.get(0).await.unwrap() + array.get(1).await.unwrap(),
                42
            );
            assert!(
                array
                    .get(2)
                    .await
                    .unwrap_err()
                    .is::<ArrayIndexOutOfBoundsException>()
            );
        });
    }
}
