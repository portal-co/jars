//! Which execution host the compiler is emitting.
//!
//! The default is the mailbox task host. Object and entity hosts emit
//! synchronous methods; the entity host also gives each instance a Bevy
//! entity id stored in the `jars-bevy` object table when that crate is linked
//! by the generated program.

use std::cell::Cell;

/// How generated Java instances are addressed and called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjectModel {
    /// Mailbox actors. Cross-object calls are `.await`ed.
    #[default]
    Task,
    /// `Rc<RefCell<_>>` state and direct method calls.
    Object,
    /// Direct method calls plus a Bevy entity id. The generated program
    /// depends on `jars-bevy`.
    Entity,
}

impl ObjectModel {
    #[must_use]
    pub const fn is_sync(self) -> bool {
        matches!(self, Self::Object | Self::Entity)
    }

    #[must_use]
    pub const fn is_entity(self) -> bool {
        matches!(self, Self::Entity)
    }
}

thread_local! {
    static CURRENT: Cell<ObjectModel> = const { Cell::new(ObjectModel::Task) };
}

pub(crate) struct ModelGuard(ObjectModel);

impl Drop for ModelGuard {
    fn drop(&mut self) {
        CURRENT.with(|cell| cell.set(self.0));
    }
}

pub(crate) fn enter(model: ObjectModel) -> ModelGuard {
    CURRENT.with(|cell| ModelGuard(cell.replace(model)))
}

pub(crate) fn current() -> ObjectModel {
    CURRENT.with(|cell| cell.get())
}

pub(crate) fn is_sync() -> bool {
    current().is_sync()
}

/// `.await` on the task host, empty on the synchronous hosts.
pub(crate) fn await_token() -> &'static str {
    if is_sync() { "" } else { ".await" }
}

/// Runtime method name. Synchronous hosts call the `_sync` twin.
pub(crate) fn runtime_method_name(name: &str) -> String {
    if is_sync() {
        format!("{name}_sync")
    } else {
        name.to_owned()
    }
}

pub(crate) fn string_builder_new(source: &str) -> String {
    if is_sync() {
        format!("jars_runtime::JavaStringBuilder::direct({source})?")
    } else {
        format!("jars_runtime::JavaStringBuilder::new(program.spawner.clone(), {source})?")
    }
}

pub(crate) fn string_builder_empty() -> String {
    if is_sync() {
        "jars_runtime::JavaStringBuilder::empty_direct()?".to_owned()
    } else {
        "jars_runtime::JavaStringBuilder::empty(program.spawner.clone())?".to_owned()
    }
}

/// `JavaArray` construction. Synchronous hosts do not take a spawner.
pub(crate) fn array_new(element_rust: &str, length: &str, default: &str) -> String {
    if is_sync() {
        format!("jars_runtime::JavaArray::<{element_rust}>::direct({length}, {default})")
    } else {
        format!(
            "jars_runtime::JavaArray::<{element_rust}>::new(program.spawner.clone(), {length}, {default})"
        )
    }
}

pub(crate) fn state_read(place: &str) -> String {
    if is_sync() {
        format!("{place}.borrow()")
    } else {
        format!("{place}.lock().expect(\"actor state mutex\")")
    }
}

pub(crate) fn state_write(place: &str) -> String {
    if is_sync() {
        format!("{place}.borrow_mut()")
    } else {
        format!("{place}.lock().expect(\"actor state mutex\")")
    }
}

/// Drops `.await` from a synchronous emission. Task emission is unchanged.
pub(crate) fn finish_source(source: String) -> String {
    if is_sync() {
        source.replace(".await", "")
    } else {
        source
    }
}
