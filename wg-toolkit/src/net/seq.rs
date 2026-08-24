//! Small utility for safe and checked storage of sequence numbers, as used on network.

use std::ops::{Add, AddAssign, Sub, SubAssign};
use std::cmp::Ordering;
use std::fmt;


/// A sequence number is a special wrapper around a `u32` that has special maximum 
/// value and with wrapping by default which avoids overflowing the sequence number
/// while still allowing comparison between two values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Seq(u32);

impl Seq {

    /// Zero constant for this number.
    pub const ZERO: Self = Self(0);

    const SIZE: u32 = 0x1000_0000;
    const MASK: u32 = 0x0FFF_FFFF;

    /// Create a new sequence number, returning none if illegal value.
    #[inline]
    pub const fn new(num: u32) -> Option<Self> {
        if num <= Self::MASK {
            Some(Self(num))
        } else {
            None
        }
    }

    /// Get the underlying real value of this sequence number.
    #[inline]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Compare this sequence number with another one, this comparison isn't implemented
    /// as standard partial or total ordering because it's isn't transitive, because it
    /// allows comparison of wrapping values, and so the comparison is based on the
    /// difference between the two value, and not there absolute value.
    /// 
    /// You must be really careful when using this ordering to sort an array, prefer to
    /// use the underlying `u32` value for comparison.
    #[inline]
    pub const fn wrapping_cmp(self, other: Self) -> Ordering {
        let a = self.0;
        let b = other.0;
        if a == b {
            Ordering::Equal
        } else if (a.wrapping_sub(b) & Self::MASK) > (Self::SIZE / 2) {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }

}

impl fmt::Display for Seq {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl Default for Seq {
    #[inline]
    fn default() -> Self {
        Self::ZERO
    }
}

impl Add<u32> for Seq {
    type Output = Self;
    #[inline]
    fn add(self, rhs: u32) -> Self::Output {
        Self(self.0.wrapping_add(rhs) & Self::MASK)
    }
}

impl AddAssign<u32> for Seq {
    #[inline]
    fn add_assign(&mut self, rhs: u32) {
        self.0 = self.0.wrapping_add(rhs) & Self::MASK;
    }
}

impl Sub<u32> for Seq {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: u32) -> Self::Output {
        Self(self.0.wrapping_sub(rhs) & Self::MASK)
    }
}

impl SubAssign<u32> for Seq {
    #[inline]
    fn sub_assign(&mut self, rhs: u32) {
        self.0 = self.0.wrapping_sub(rhs) & Self::MASK;
    }
}

/// Delta between two sequence numbers.
impl Sub for Seq {
    type Output = u32;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        (self - rhs.0).get()
    }
}

impl TryFrom<u32> for Seq {
    type Error = ();
    #[inline]
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value).ok_or(())
    }
}


/// An allocator for contiguous sequence numbers.
#[derive(Debug)]
pub struct SeqAlloc {
    next: Seq,
}

impl SeqAlloc {

    #[inline]
    pub const fn new(next: Seq) -> Self {
        Self {
            next,
        }
    }

    #[inline]
    pub fn alloc(&mut self, count: u32) -> Seq {
        let ret = self.next;
        self.next += count;
        ret
    }

    /// Ensure that the next allocated sequence number will be strictly after the given
    /// one, without moving it backward if it's already ahead. Used to keep an allocator
    /// that isn't the sole writer of a sequence space (e.g. a proxy that mostly forwards
    /// a peer's own sequence numbers verbatim) in sync with numbers observed passing
    /// through, so that an on-demand allocation continues seamlessly from whatever the
    /// remote end has already seen instead of colliding or lagging behind it.
    #[inline]
    pub fn observe(&mut self, seq: Seq) {
        let candidate = seq + 1;
        if candidate.wrapping_cmp(self.next) == Ordering::Greater {
            self.next = candidate;
        }
    }

}


#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn invalid_value() {
        assert!(Seq::new(0).is_some());
        assert!(Seq::new(0x0FFF_FFFF).is_some());
        assert!(Seq::new(0x1000_0000).is_none());
    }

    #[test]
    fn ordering() {

        const ZERO: Seq = Seq(0);
        const HALF: Seq = Seq(0x0800_0000);
        const FULL: Seq = Seq(0x0FFF_FFFF);

        assert_eq!(ZERO - 1, FULL);

        assert_eq!(Seq::wrapping_cmp(ZERO + 0, ZERO + 1), Ordering::Less);
        assert_eq!(Seq::wrapping_cmp(ZERO + 0, ZERO - 1), Ordering::Greater);

        assert_eq!(Seq::wrapping_cmp(ZERO + 0, HALF - 1 + 0), Ordering::Less);
        assert_eq!(Seq::wrapping_cmp(ZERO + 0, HALF - 1 + 1), Ordering::Greater); // Because we are too far.

        // Check that the limit of less/greater is moving, relative.
        assert_eq!(Seq::wrapping_cmp(ZERO + 1, HALF - 1 + 1), Ordering::Less);
        assert_eq!(Seq::wrapping_cmp(ZERO + 1, HALF - 1 + 2), Ordering::Greater);

    }

}
