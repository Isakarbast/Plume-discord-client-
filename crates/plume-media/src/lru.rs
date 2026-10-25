use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

/// A least-recently-used cache bounded by total *bytes*, not entry count,
/// because one image can be a hundred times bigger than another.
pub struct ByteLru<K, V> {
    capacity: usize,
    used: usize,
    entries: HashMap<K, (V, usize)>,
    /// Keys from least to most recently used.
    order: VecDeque<K>,
}

impl<K: Hash + Eq + Clone, V: Clone> ByteLru<K, V> {
    pub fn new(capacity: usize) -> Self {
        ByteLru {
            capacity,
            used: 0,
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    pub fn used(&self) -> usize {
        self.used
    }

    pub fn get(&mut self, key: &K) -> Option<V> {
        let value = self.entries.get(key)?.0.clone();
        self.touch(key);
        Some(value)
    }

    /// Adds an entry, evicting the least recently used ones to make room.
    /// Something bigger than the whole cache isn't kept at all.
    pub fn insert(&mut self, key: K, value: V, size: usize) {
        if size > self.capacity {
            return;
        }
        if let Some((_, old)) = self.entries.remove(&key) {
            self.used -= old;
            self.order.retain(|k| k != &key);
        }
        while self.used + size > self.capacity {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some((_, freed)) = self.entries.remove(&oldest) {
                self.used -= freed;
            }
        }
        self.used += size;
        self.entries.insert(key.clone(), (value, size));
        self.order.push_back(key);
    }

    fn touch(&mut self, key: &K) {
        // O(n), but n is a few hundred images at most.
        if let Some(index) = self.order.iter().position(|k| k == key)
            && let Some(key) = self.order.remove(index)
        {
            self.order.push_back(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recently_used_by_size() {
        let mut lru = ByteLru::new(100);
        lru.insert("a", 1, 40);
        lru.insert("b", 2, 40);
        assert_eq!(lru.get(&"a"), Some(1)); // a is now the most recent
        lru.insert("c", 3, 40); // needs room: drops b, not a

        assert_eq!(lru.get(&"b"), None);
        assert_eq!(lru.get(&"a"), Some(1));
        assert_eq!(lru.get(&"c"), Some(3));
        assert_eq!(lru.used(), 80);
    }

    #[test]
    fn too_big_items_are_not_cached() {
        let mut lru = ByteLru::new(10);
        lru.insert("huge", 1, 11);
        assert_eq!(lru.get(&"huge"), None);
        assert_eq!(lru.used(), 0);
    }

    #[test]
    fn replacing_a_key_updates_its_size() {
        let mut lru = ByteLru::new(100);
        lru.insert("a", 1, 60);
        lru.insert("a", 2, 30);
        assert_eq!(lru.used(), 30);
        assert_eq!(lru.get(&"a"), Some(2));
    }
}
