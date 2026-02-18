//! Generic helper functions for working with GTK ListStore.

use gtk::gio;
use gtk::glib;
use gtk::prelude::{Cast, IsA, ListModelExt};

/// Find the index of an item in a ListStore that matches the predicate.
pub fn find_index<T, F>(store: &gio::ListStore, predicate: F) -> Option<u32>
where
    T: IsA<glib::Object>,
    F: Fn(&T) -> bool,
{
    for i in 0..store.n_items() {
        if let Some(obj) = store.item(i) {
            if let Some(item) = obj.downcast_ref::<T>() {
                if predicate(item) {
                    return Some(i);
                }
            }
        }
    }
    None
}

/// Apply a function to each item in a ListStore.
pub fn for_each<T, F>(store: &gio::ListStore, mut f: F)
where
    T: IsA<glib::Object>,
    F: FnMut(&T),
{
    for i in 0..store.n_items() {
        if let Some(obj) = store.item(i) {
            if let Some(item) = obj.downcast_ref::<T>() {
                f(item);
            }
        }
    }
}

/// Find an item by predicate and apply a function to it.
/// Returns true if the item was found.
pub fn with_item<T, P, F>(store: &gio::ListStore, predicate: P, f: F) -> bool
where
    T: IsA<glib::Object>,
    P: Fn(&T) -> bool,
    F: FnOnce(&T),
{
    if let Some(idx) = find_index::<T, _>(store, predicate) {
        if let Some(obj) = store.item(idx) {
            if let Some(item) = obj.downcast_ref::<T>() {
                f(item);
                return true;
            }
        }
    }
    false
}
