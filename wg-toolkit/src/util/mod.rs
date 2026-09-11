//! Provides various internal utilities.

use std::fmt::{self, Write};

pub mod io;
pub mod fnv;
pub mod cuckoo;
pub mod thread;


/// A helper structure for pretty printing of bytes. It provides format implementations 
/// for upper and lower hex formatters (`{:x}`, `{:X}`).
pub struct BytesFmt<'a>(pub &'a [u8]);

impl fmt::UpperHex for BytesFmt<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            f.write_fmt(format_args!("{:02X}", byte))?;
        }
        Ok(())
    }
}

impl fmt::LowerHex for BytesFmt<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            f.write_fmt(format_args!("{:02x}", byte))?;
        }
        Ok(())
    }
}

/// A helper structure for pretty printing of bytes with ASCII escaping if not printable.
/// We are intentionally not using standard escape sequence, to avoid being too verbose.
pub struct AsciiFmt<'a>(pub &'a [u8]);

impl fmt::Debug for AsciiFmt<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {

        const CAP: u32 = 4;
        const HALF_CAP: u32 = CAP / 2;

        #[inline]
        fn is_graphic(byte: u8) -> bool {
            byte.is_ascii_graphic() || byte == b' '
        }

        // We accumulate bytes into this buffer, this is used to compute the number of
        // alphanumeric characters present in the buffer and determine if it's relevant 
        // to print it as ASCII. This buffer stores 4 ASCII chars.
        let mut buffer = 0u32;
        let mut graphic_count = 0u32;
        let mut alphanumeric_count = 0u32;
        let mut human = false;

        // We chain CAP nul chars that will never be printed but are used to empty the 
        // buffer of all actual bytes.
        for (i, byte) in self.0.iter().copied().chain([0u8; CAP as usize]).enumerate() {

            // Start pop only when all 4 initial buffered characters are passed.
            if i >= CAP as usize {

                let pop_byte = (buffer >> ((CAP - 1) * 8)) as u8;

                // Thresholds for switching print mode...
                if !human && graphic_count == CAP && alphanumeric_count >= HALF_CAP {
                    human = true;
                    if i > CAP as usize {
                        f.write_char(' ')?;
                    }
                    f.write_char('\"')?;
                } else if human && !is_graphic(pop_byte) {
                    human = false;
                    f.write_str("\" ")?;
                }

                if human {
                    f.write_char(pop_byte as char)?;
                } else {
                    f.write_fmt(format_args!("{:02X}", pop_byte))?;
                }

                graphic_count -= is_graphic(pop_byte) as u32;
                alphanumeric_count -= pop_byte.is_ascii_alphanumeric() as u32;

            }

            buffer <<= 8;
            buffer |= byte as u32;
            graphic_count += is_graphic(byte) as u32;
            alphanumeric_count += byte.is_ascii_alphanumeric() as u32;

        }

        if human {
            f.write_char('"')?;
        }

        Ok(())

    }
}

/// A helper structure to truncate the output of some display implementor, adding 
/// trailing '...' if necessary.
pub struct TruncateFmt<F>(pub F, pub usize);

impl<F: fmt::Display> fmt::Display for TruncateFmt<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut buf = String::new();
        buf.write_fmt(format_args!("{}", self.0))?;
        if buf.len() > self.1 {
            buf.truncate(self.1 - 3);
            buf.push_str("...");
        }
        f.write_str(&buf)
    }
}

pub struct SizeFmt(pub u64);

impl fmt::Display for SizeFmt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            n @ 0..=999 => 
                write!(f, "{:>4} B", n),
            n @ 1_000..=999_999 => 
                write!(f, "{:>3} kB", n / 1_000),
            n @ 1_000_000..=999_999_999 => 
                write!(f, "{:>3} MB", n / 1_000_000),
            n @ 1_000_000_000..=999_999_999_999 => 
                write!(f, "{:>3} GB", n / 1_000_000_000),
            n =>
                write!(f, "{:>3} TB", n / 1_000_000_000_000),
        }
    }
}
