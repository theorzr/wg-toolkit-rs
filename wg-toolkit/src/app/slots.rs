//! Values addressed by generational keys: a slot index and the generation of the value
//! in it, so that a key to a removed value never reaches the value that took its slot.


/// The place of a value: its slot, and the generation of the value in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotKey {
    index: u32,
    generation: u32,
}

/// See the module documentation.
#[derive(Debug)]
pub struct Slots<T> {
    slots: Vec<Slot<T>>,
    /// The slots without a value, to take before growing.
    free: Vec<u32>,
}

#[derive(Debug)]
struct Slot<T> {
    generation: u32,
    value: Option<T>,
}

impl<T> Default for Slots<T> {
    fn default() -> Self {
        Self { slots: Vec::new(), free: Vec::new() }
    }
}

impl<T> Slots<T> {

    /// Insert a value, returning its key.
    pub fn insert(&mut self, value: T) -> SlotKey {
        match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                slot.generation = slot.generation.wrapping_add(1);
                slot.value = Some(value);
                SlotKey { index, generation: slot.generation }
            }
            None => {
                self.slots.push(Slot { generation: 0, value: Some(value) });
                SlotKey { index: (self.slots.len() - 1) as u32, generation: 0 }
            }
        }
    }

    pub fn get(&self, key: SlotKey) -> Option<&T> {
        self.slots.get(key.index as usize)
            .filter(|slot| slot.generation == key.generation)
            .and_then(|slot| slot.value.as_ref())
    }

    pub fn get_mut(&mut self, key: SlotKey) -> Option<&mut T> {
        self.slots.get_mut(key.index as usize)
            .filter(|slot| slot.generation == key.generation)
            .and_then(|slot| slot.value.as_mut())
    }

    /// Every value, with its key.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (SlotKey, &mut T)> {
        self.slots.iter_mut().enumerate()
            .filter_map(|(index, slot)| Some((SlotKey { index: index as u32, generation: slot.generation }, slot.value.as_mut()?)))
    }

    pub fn remove(&mut self, key: SlotKey) -> Option<T> {
        let slot = self.slots.get_mut(key.index as usize).filter(|slot| slot.generation == key.generation)?;
        let value = slot.value.take()?;
        self.free.push(key.index);
        Some(value)
    }

}


#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn a_removed_value_is_not_reached_through_its_slot() {
        let mut slots = Slots::default();
        let a = slots.insert("a");
        assert_eq!(slots.remove(a), Some("a"));
        let b = slots.insert("b");
        assert_eq!(a.index, b.index, "the slot is reused");
        assert_eq!(slots.get(a), None);
        assert_eq!(slots.get(b), Some(&"b"));
    }

}
