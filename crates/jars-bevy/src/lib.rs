//! Entity host for generated jars programs.
//!
//! A Bevy `Entity` is only an identity. Instance state stays in this table,
//! behind `Rc<RefCell<_>>`, because a Java call can re-enter while a Bevy
//! `World` query cannot. The app pumps a headless schedule and does not open
//! a window.

use std::{any::Any, cell::RefCell, collections::HashMap, rc::Rc};

use bevy_app::{App, Update};
use bevy_ecs::{
    change_detection::NonSend,
    prelude::{ResMut, Resource},
    entity::Entity,
};

/// Bevy identity for one Java instance. Field values live in [`ObjectTable`].
pub type ObjectId = Entity;

/// Host-owned Java instance state. Generated methods keep their own `Rc` clone
/// and call through it directly; the table keeps that state reachable from the
/// schedule.
pub struct ObjectTable {
    next: u32,
    objects: HashMap<u64, Box<dyn Any>>,
}

impl ObjectTable {
    #[must_use]
    pub fn new() -> Self {
        Self {
            next: 1,
            objects: HashMap::new(),
        }
    }

    /// Stores `value` and returns a fresh entity id. Index `0` is left unused
    /// because Bevy treats that bit pattern as a placeholder.
    pub fn insert<T: 'static>(&mut self, value: Rc<RefCell<T>>) -> ObjectId {
        let index = self.next;
        self.next = self
            .next
            .checked_add(1)
            .expect("object table exhausted");
        let id = ObjectId::from_raw_u32(index).expect("object id");
        self.objects.insert(id.to_bits(), Box::new(value));
        id
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
}

impl Default for ObjectTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Schedule-owned handle to the object table. `Rc` is not `Send`, so this is a
/// non-send resource and the schedule runs on the thread that created it.
pub struct JavaObjects(pub Rc<RefCell<ObjectTable>>);

/// Builds a headless app that owns `table`. No window plugin is installed.
#[must_use]
pub fn headless_app(table: Rc<RefCell<ObjectTable>>) -> App {
    let mut app = App::new();
    app.insert_non_send(JavaObjects(table));
    app
}

/// Runs the main schedule once.
pub fn update_once(app: &mut App) {
    app.update();
}

#[derive(Resource, Default)]
struct Seen(usize);

fn count_objects(objects: NonSend<JavaObjects>, mut seen: ResMut<Seen>) {
    seen.0 = objects.0.borrow().len();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_schedule_observes_one_inserted_object() {
        let table = Rc::new(RefCell::new(ObjectTable::new()));
        let id = table.borrow_mut().insert(Rc::new(RefCell::new(42_i32)));
        assert_ne!(id, Entity::PLACEHOLDER);
        let mut app = headless_app(Rc::clone(&table));
        app.init_resource::<Seen>();
        app.add_systems(Update, count_objects);
        update_once(&mut app);
        assert_eq!(app.world().resource::<Seen>().0, 1);
        assert_eq!(table.borrow().len(), 1);
    }
}
