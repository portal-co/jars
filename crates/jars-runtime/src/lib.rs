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
java_error!(
    StringIndexOutOfBoundsException,
    "string index out of bounds"
);

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
pub fn string_index_out_of_bounds() -> anyhow::Error {
    StringIndexOutOfBoundsException.into()
}

/// Materializes the runtime half of the shared standard-library declaration.
/// The same declaration is consumed by `jars-core` to build its exact JVM
/// member table and Java reference-coercion rules.
macro_rules! runtime_stdlib {
    (
        $(
            class {
                name: $name:literal,
                rust_type: $rust_type:expr,
                coercions: [$($coercions:tt)*],
                runtime: { $($runtime:tt)* },
                members: [$($members:tt)*],
            }
        )*
    ) => {
        $($($runtime)*)*
    };
}

jars_stdlib::java_stdlib!(runtime_stdlib);

/// Matches the closed runtime throwable hierarchy without reflection.  The AOT
/// compiler embeds only the requested class name from an exception table.
pub fn catches(error: &JavaError, class: &str) -> bool {
    match class {
        "java/lang/ArithmeticException" => error.is::<ArithmeticException>(),
        "java/lang/NullPointerException" => error.is::<NullPointerException>(),
        "java/lang/ClassCastException" => error.is::<ClassCastException>(),
        "java/lang/ArrayIndexOutOfBoundsException" => error.is::<ArrayIndexOutOfBoundsException>(),
        "java/lang/StringIndexOutOfBoundsException" => {
            error.is::<StringIndexOutOfBoundsException>()
        }
        "java/lang/NegativeArraySizeException" => error.is::<NegativeArraySizeException>(),
        "java/lang/ArrayStoreException" => error.is::<ArrayStoreException>(),
        "java/lang/Exception" | "java/lang/RuntimeException" | "java/lang/Throwable" => {
            error.is::<ArithmeticException>()
                || error.is::<NullPointerException>()
                || error.is::<ClassCastException>()
                || error.is::<ArrayIndexOutOfBoundsException>()
                || error.is::<StringIndexOutOfBoundsException>()
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

/// A typed Java array address.  The task host stores elements in a mailbox
/// actor. The object and entity hosts store them in a `RefCell` and call the
/// `_sync` methods directly. Aliases share one store; elements never escape it.
pub struct JavaArray<T> {
    store: ArrayStore<T>,
}

enum ArrayStore<T> {
    Actor(ActorRef<ArrayMessage<T>>),
    Direct(Rc<RefCell<Vec<T>>>),
}

impl<T> Clone for JavaArray<T> {
    fn clone(&self) -> Self {
        Self {
            store: match &self.store {
                ArrayStore::Actor(actor) => ArrayStore::Actor(actor.clone()),
                ArrayStore::Direct(state) => ArrayStore::Direct(Rc::clone(state)),
            },
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
        match (&self.store, &other.store) {
            (ArrayStore::Actor(left), ArrayStore::Actor(right)) => left.same(right),
            (ArrayStore::Direct(left), ArrayStore::Direct(right)) => Rc::ptr_eq(left, right),
            _ => false,
        }
    }

    /// Creates a default-filled Java array on the synchronous hosts.
    pub fn direct(length: i32, default: T) -> JavaResult<Self> {
        if length < 0 {
            return Err(negative_array_size());
        }
        Ok(Self {
            store: ArrayStore::Direct(Rc::new(RefCell::new(vec![default; length as usize]))),
        })
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
        Ok(Self {
            store: ArrayStore::Actor(actor),
        })
    }

    pub async fn length(&self) -> JavaResult<i32> {
        match &self.store {
            ArrayStore::Direct(state) => Ok(state.borrow().len() as i32),
            ArrayStore::Actor(actor) => {
                let (reply, response) = reply();
                actor.send(ArrayMessage::Length { reply }).await?;
                response.recv().await?
            }
        }
    }

    pub fn length_sync(&self) -> JavaResult<i32> {
        match &self.store {
            ArrayStore::Direct(state) => Ok(state.borrow().len() as i32),
            ArrayStore::Actor(_) => Err(crate::direct_call_on_actor()),
        }
    }

    pub async fn get(&self, index: i32) -> JavaResult<T> {
        match &self.store {
            ArrayStore::Direct(state) => direct_array_get(state, index),
            ArrayStore::Actor(actor) => {
                let (reply, response) = reply();
                actor.send(ArrayMessage::Get { index, reply }).await?;
                response.recv().await?
            }
        }
    }

    pub fn get_sync(&self, index: i32) -> JavaResult<T> {
        match &self.store {
            ArrayStore::Direct(state) => direct_array_get(state, index),
            ArrayStore::Actor(_) => Err(crate::direct_call_on_actor()),
        }
    }

    pub async fn set(&self, index: i32, value: T) -> JavaResult<()> {
        match &self.store {
            ArrayStore::Direct(state) => direct_array_set(state, index, value),
            ArrayStore::Actor(actor) => {
                let (reply, response) = reply();
                actor
                    .send(ArrayMessage::Set {
                        index,
                        value,
                        reply,
                    })
                    .await?;
                response.recv().await?
            }
        }
    }

    pub fn set_sync(&self, index: i32, value: T) -> JavaResult<()> {
        match &self.store {
            ArrayStore::Direct(state) => direct_array_set(state, index, value),
            ArrayStore::Actor(_) => Err(crate::direct_call_on_actor()),
        }
    }
}

fn direct_array_get<T: Clone>(state: &Rc<RefCell<Vec<T>>>, index: i32) -> JavaResult<T> {
    if index < 0 {
        return Err(array_index_out_of_bounds());
    }
    state
        .borrow()
        .get(index as usize)
        .cloned()
        .ok_or_else(array_index_out_of_bounds)
}

fn direct_array_set<T: Clone>(state: &Rc<RefCell<Vec<T>>>, index: i32, value: T) -> JavaResult<()> {
    if index < 0 {
        return Err(array_index_out_of_bounds());
    }
    let mut elements = state.borrow_mut();
    match elements.get_mut(index as usize) {
        Some(slot) => {
            *slot = value;
            Ok(())
        }
        None => Err(array_index_out_of_bounds()),
    }
}

fn direct_call_on_actor() -> anyhow::Error {
    anyhow::anyhow!("synchronous Java call used a mailbox actor")
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

/// Wraps an arbitrary Java entrypoint result so generated harness code can
/// treat `()` and meaningful values uniformly.
pub struct JavaUnit<T>(pub T);

pub fn println<T: Display>(value: T) {
    std::println!("{value}");
}

/// Prints a nullable Java string: Java's `println(String)` emits the literal
/// `null` for a null reference.
pub fn println_string(value: Option<JavaString>) {
    match value {
        Some(value) => std::println!("{}", value.as_str()),
        None => std::println!("null"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static FLITE_TEST_NAME: std::sync::Mutex<Option<Option<String>>> = std::sync::Mutex::new(None);

    unsafe extern "C" fn fake_register_cmu_us_kal16(
        name: *const std::ffi::c_char,
    ) -> *mut std::ffi::c_void {
        let name = if name.is_null() {
            None
        } else {
            Some(
                unsafe { std::ffi::CStr::from_ptr(name) }
                    .to_string_lossy()
                    .into_owned(),
            )
        };
        *FLITE_TEST_NAME.lock().unwrap() = Some(name);
        0x1234_usize as *mut std::ffi::c_void
    }

    #[test]
    fn flite_native_shim_marshals_nullable_java_strings_and_pointer_results() {
        *FLITE_TEST_NAME.lock().unwrap() = None;
        let result =
            register_cmu_us_kal16_with(Some(JavaString::new("kal16")), fake_register_cmu_us_kal16)
                .unwrap()
                .unwrap();
        assert_eq!(
            *FLITE_TEST_NAME.lock().unwrap(),
            Some(Some("kal16".to_owned()))
        );
        assert_eq!(result.as_ptr(), 0x1234_usize as *mut std::ffi::c_void);

        *FLITE_TEST_NAME.lock().unwrap() = None;
        let result = register_cmu_us_kal16_with(None, fake_register_cmu_us_kal16)
            .unwrap()
            .unwrap();
        assert_eq!(result.as_ptr(), 0x1234_usize as *mut std::ffi::c_void);
        assert_eq!(*FLITE_TEST_NAME.lock().unwrap(), Some(None));
        assert!(
            register_cmu_us_kal16_with(
                Some(JavaString::new("bad\0name")),
                fake_register_cmu_us_kal16,
            )
            .is_err()
        );
    }

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
    fn char_sequence_helpers_follow_java_utf16_and_whitespace_rules() {
        assert_eq!(char_sequence_char_at("a😀", 0).unwrap(), u16::from(b'a'));
        assert_eq!(char_sequence_char_at("a😀", 1).unwrap(), 0xd83d);
        assert_eq!(char_sequence_char_at("a😀", 2).unwrap(), 0xde00);
        assert!(
            char_sequence_char_at("a", -1)
                .unwrap_err()
                .is::<StringIndexOutOfBoundsException>()
        );
        let error = string_index_out_of_bounds();
        assert!(catches(&error, "java/lang/StringIndexOutOfBoundsException"));
        assert!(catches(&error, "java/lang/RuntimeException"));
        assert!(character::is_whitespace(u16::from(b' ')));
        assert!(character::is_whitespace(u16::from(b'\t')));
        assert!(!character::is_whitespace(0x00a0));
        assert!(!character::is_whitespace(0x0085));
    }

    #[test]
    fn generated_char_sequence_dispatches_to_a_rust_written_actor_class() {
        let runtime = Runtime::new();
        runtime.block_on(async {
            let string = CharSequence::from_string("\u{1680}");
            assert_eq!(string.length().await.unwrap(), 1);

            let owned = JavaString::new("a😀");
            assert_eq!(owned.as_str(), "a😀");
            assert!(owned.__same(&JavaString::new("a😀")));
            let sequence = CharSequence::from_java_string(owned.clone());
            assert_eq!(sequence.length().await.unwrap(), 3);

            let builder = JavaStringBuilder::new(runtime.clone(), owned).unwrap();
            let sequence = CharSequence::from_string_builder(builder.clone());
            assert_eq!(sequence.length().await.unwrap(), 3);
            assert_eq!(sequence.char_at(1).await.unwrap(), 0xd83d);
            assert!(sequence.__same(&CharSequence::from_string_builder(builder)));
        });
    }

    #[test]
    fn direct_array_and_string_builder_mutate_without_a_mailbox() {
        let array = JavaArray::direct(2, 0_i32).unwrap();
        array.set_sync(0, 20).unwrap();
        array.set_sync(1, 22).unwrap();
        assert_eq!(array.get_sync(0).unwrap() + array.get_sync(1).unwrap(), 42);
        let builder = JavaStringBuilder::direct(JavaString::new("4")).unwrap();
        let builder = builder.append_int_sync(2).unwrap();
        assert_eq!(builder.to_string_value_sync().unwrap().as_str(), "42");
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
