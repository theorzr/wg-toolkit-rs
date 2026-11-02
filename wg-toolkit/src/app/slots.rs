//! Values addressed by generational keys: a slot index and the generation of the value
//! in it, so that a key to a removed value never reaches the value that took its slot.

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

    /// Insert a value, returning its slot and generation.
    pub fn insert(&mut self, value: T) -> (u32, u32) {
        match self.free.pop() {
            Some(index) => {
                let slot = &mut self.slots[index as usize];
                slot.generation = slot.generation.wrapping_add(1);
                slot.value = Some(value);
                (index, slot.generation)
            }
            None => {
                self.slots.push(Slot { generation: 0, value: Some(value) });
                ((self.slots.len() - 1) as u32, 0)
            }
        }
    }

    pub fn get(&self, index: u32, generation: u32) -> Option<&T> {
        self.slots.get(index as usize)
            .filter(|slot| slot.generation == generation)
            .and_then(|slot| slot.value.as_ref())
    }

    pub fn get_mut(&mut self, index: u32, generation: u32) -> Option<&mut T> {
        self.slots.get_mut(index as usize)
            .filter(|slot| slot.generation == generation)
            .and_then(|slot| slot.value.as_mut())
    }

    pub fn remove(&mut self, index: u32, generation: u32) -> Option<T> {
        let slot = self.slots.get_mut(index as usize).filter(|slot| slot.generation == generation)?;
        let value = slot.value.take()?;
        self.free.push(index);
        Some(value)
    }

}


#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn a_removed_value_is_not_reached_through_its_slot() {
        let mut slots = Slots::default();
        let (a, a_gen) = slots.insert("a");
        assert_eq!(slots.remove(a, a_gen), Some("a"));
        let (b, b_gen) = slots.insert("b");
        assert_eq!(a, b, "the slot is reused");
        assert_eq!(slots.get(a, a_gen), None);
        assert_eq!(slots.get(b, b_gen), Some(&"b"));
    }

}
