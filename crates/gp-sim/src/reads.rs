//! What a run reads of its [`Spec`]: which kinds, levels, machines,
//! counters and cameras, and which `ext` hooks it calls, so a scorer can
//! tell which clips a change to the model can move (docs/gate.md,
//! "Affected clips").
//!
//! The spec's lists of those are [`Parts`], whose every access on a thread
//! that is [`recording`] notes what it reads: an index notes that element,
//! anything that sees the whole list (a slice, an iterator) notes all of
//! them. Nothing else reaches the elements, so a run that noted no read of
//! an element ran the same whatever that element holds. The names and the
//! count of each list aren't noted: a scorer compares them whole.
//!
//! Recording is off unless asked for, and costs a thread-local read then.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::ops::{Deref, DerefMut, Index, IndexMut};

/// The lists of a spec a run's reads are noted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum List {
    Kind,
    Level,
    Machine,
    Counter,
    Camera,
}

impl List {
    pub const ALL: [List; 5] = [
        List::Kind,
        List::Level,
        List::Machine,
        List::Counter,
        List::Camera,
    ];

    /// The part's name in a read set: `kinds`, `levels` and so on, as the
    /// spec's field.
    pub fn name(self) -> &'static str {
        match self {
            List::Kind => "kinds",
            List::Level => "levels",
            List::Machine => "machines",
            List::Counter => "counters",
            List::Camera => "cameras",
        }
    }
}

/// An element type of a [`Parts`] list.
pub trait Element {
    const PART: List;
    /// The element's name, which names it in a read set.
    fn name(&self) -> &str;
}

/// What a run read: per part, the indexes of the elements read, or every
/// element; and the hooks called, by the name the model gives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reads {
    pub parts: [Seen; 5],
    pub hooks: BTreeSet<String>,
}

/// The elements of one list a run read: all of them, or those whose bits
/// are set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seen {
    pub all: bool,
    bits: Vec<u64>,
}

impl Seen {
    pub fn has(&self, i: usize) -> bool {
        self.all
            || self
                .bits
                .get(i / 64)
                .is_some_and(|w| w & (1 << (i % 64)) != 0)
    }

    /// The indexes read, when not all of them.
    pub fn some(&self) -> BTreeSet<usize> {
        (0..self.bits.len() * 64).filter(|&i| self.has(i)).collect()
    }

    #[inline]
    fn add(&mut self, i: usize) {
        let w = i / 64;
        if w >= self.bits.len() {
            self.bits.resize(w + 1, 0);
        }
        self.bits[w] |= 1 << (i % 64);
    }
}

impl Reads {
    pub fn part(&self, p: List) -> &Seen {
        &self.parts[p as usize]
    }

    /// Adds what `o` read.
    pub fn union(&mut self, o: &Reads) {
        for (a, b) in self.parts.iter_mut().zip(&o.parts) {
            a.all |= b.all;
            if a.bits.len() < b.bits.len() {
                a.bits.resize(b.bits.len(), 0);
            }
            for (x, y) in a.bits.iter_mut().zip(&b.bits) {
                *x |= y;
            }
        }
        self.hooks.extend(o.hooks.iter().cloned());
    }
}

thread_local! {
    static ON: Cell<bool> = const { Cell::new(false) };
    static SET: RefCell<Reads> = RefCell::new(Reads::default());
}

/// Runs `f` with this thread's reads noted, and gives what it read. A
/// recording inside another adds its reads to the outer one's too.
pub fn recording<R>(f: impl FnOnce() -> R) -> (R, Reads) {
    let was_on = ON.with(|o| o.replace(true));
    let before = SET.with(|s| std::mem::take(&mut *s.borrow_mut()));
    let r = f();
    ON.with(|o| o.set(was_on));
    let got = SET.with(|s| std::mem::replace(&mut *s.borrow_mut(), before));
    if was_on {
        SET.with(|s| s.borrow_mut().union(&got));
    }
    (r, got)
}

/// Runs `f` with this thread's reads not noted: for work shared by every
/// run that a scorer accounts for apart (the model's pictures).
pub fn paused<R>(f: impl FnOnce() -> R) -> R {
    let was_on = ON.with(|o| o.replace(false));
    let r = f();
    ON.with(|o| o.set(was_on));
    r
}

/// Whether this thread is recording.
pub fn on() -> bool {
    ON.with(Cell::get)
}

#[inline]
fn note(p: List, i: usize) {
    if ON.with(Cell::get) {
        SET.with(|s| {
            let mut s = s.borrow_mut();
            let seen = &mut s.parts[p as usize];
            if !seen.all {
                seen.add(i);
            }
        });
    }
}

#[inline]
fn note_all(p: List) {
    if ON.with(Cell::get) {
        SET.with(|s| {
            let mut s = s.borrow_mut();
            let seen = &mut s.parts[p as usize];
            seen.all = true;
            seen.bits.clear();
        });
    }
}

/// Notes a call of the hook the model names `name`.
#[inline]
pub fn note_hook(name: &str) {
    if ON.with(Cell::get) {
        SET.with(|s| {
            let mut s = s.borrow_mut();
            if !s.hooks.contains(name) {
                s.hooks.insert(name.to_owned());
            }
        });
    }
}

/// A list of a spec's elements whose reads are noted ([`recording`]).
/// Indexing notes the element; [`Parts::get`] notes it when it is there;
/// anything through the slice (an iterator, a range, a search) notes all
/// of them. [`Parts::len`] and [`Parts::index_of`] read only the count and
/// the names, which a scorer compares whole.
#[derive(Clone, PartialEq, Eq)]
pub struct Parts<T>(Vec<T>);

impl<T> Default for Parts<T> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for Parts<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl<T> From<Vec<T>> for Parts<T> {
    fn from(v: Vec<T>) -> Self {
        Self(v)
    }
}

impl<T> FromIterator<T> for Parts<T> {
    fn from_iter<I: IntoIterator<Item = T>>(i: I) -> Self {
        Self(i.into_iter().collect())
    }
}

impl<T: Element> Parts<T> {
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<&T> {
        if i < self.0.len() {
            note(T::PART, i);
        }
        self.0.get(i)
    }

    pub fn get_mut(&mut self, i: usize) -> Option<&mut T> {
        if i < self.0.len() {
            note(T::PART, i);
        }
        self.0.get_mut(i)
    }

    /// The index of the element named `name`, reading only the names.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.0.iter().position(|e| e.name() == name)
    }

    /// The element named `name`, which notes that element alone.
    pub fn named(&self, name: &str) -> Option<&T> {
        self.index_of(name).and_then(|i| self.get(i))
    }

    /// The names, in order, reading nothing else.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(Element::name)
    }

    pub fn push(&mut self, e: T) {
        self.0.push(e);
    }

    /// The list without noting any read: for a scorer that compares specs,
    /// never for a run.
    pub fn unnoted(&self) -> &[T] {
        &self.0
    }

    pub fn into_vec(self) -> Vec<T> {
        self.0
    }
}

impl<T: Element> Index<usize> for Parts<T> {
    type Output = T;
    #[inline]
    fn index(&self, i: usize) -> &T {
        note(T::PART, i);
        &self.0[i]
    }
}

impl<T: Element> IndexMut<usize> for Parts<T> {
    #[inline]
    fn index_mut(&mut self, i: usize) -> &mut T {
        note(T::PART, i);
        &mut self.0[i]
    }
}

macro_rules! by_range {
    ($($r:ty),*) => {$(
        impl<T: Element> Index<$r> for Parts<T> {
            type Output = [T];
                    fn index(&self, r: $r) -> &[T] {
                note_all(T::PART);
                &self.0[r]
            }
        }
    )*};
}
by_range!(
    std::ops::Range<usize>,
    std::ops::RangeTo<usize>,
    std::ops::RangeFrom<usize>,
    std::ops::RangeFull,
    std::ops::RangeInclusive<usize>
);

impl<T: Element> Deref for Parts<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        note_all(T::PART);
        &self.0
    }
}

impl<T: Element> DerefMut for Parts<T> {
    fn deref_mut(&mut self) -> &mut [T] {
        note_all(T::PART);
        &mut self.0
    }
}

impl<'a, T: Element> IntoIterator for &'a Parts<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        note_all(T::PART);
        self.0.iter()
    }
}

impl<'a, T: Element> IntoIterator for &'a mut Parts<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        note_all(T::PART);
        self.0.iter_mut()
    }
}

impl<T: Element> IntoIterator for Parts<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        note_all(T::PART);
        self.0.into_iter()
    }
}

impl<T: Element> Extend<T> for Parts<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, i: I) {
        self.0.extend(i);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct K(&'static str, u32);
    impl Element for K {
        const PART: List = List::Kind;
        fn name(&self) -> &str {
            self.0
        }
    }

    #[test]
    fn a_run_notes_each_element_it_reads_and_all_of_a_list_it_walks() {
        let p: Parts<K> = vec![K("a", 1), K("b", 2), K("c", 3)].into();
        // Off: nothing is noted.
        let _ = p[0].1;
        let ((), r) = recording(|| {
            let _ = p[1].1;
            let _ = p.get(2);
            let _ = p.get(9);
            let _ = p.index_of("a");
            let _ = p.len();
        });
        assert_eq!(r.part(List::Kind).some(), [1, 2].into());
        assert!(!r.part(List::Kind).all);
        let (sum, r) = recording(|| p.iter().map(|k| k.1).sum::<u32>());
        assert_eq!(sum, 6);
        assert!(r.part(List::Kind).all);
        let ((), r) = recording(|| for _ in &p {});
        assert!(r.part(List::Kind).all);
        // A recording inside another adds to it.
        let ((), outer) = recording(|| {
            let _ = p[0].1;
            let ((), inner) = recording(|| {
                let _ = p[2].1;
                note_hook("h");
            });
            assert_eq!(inner.part(List::Kind).some(), [2].into());
        });
        assert_eq!(outer.part(List::Kind).some(), [0, 2].into());
        assert_eq!(outer.hooks, ["h".to_owned()].into());
        assert!(!on());
    }
}
