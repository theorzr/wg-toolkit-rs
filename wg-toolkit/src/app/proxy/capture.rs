//! Compressed capture of every packet a proxy forwards, for offline analysis.
//!
//! A capture is a gzip stream (readable with `zcat`) holding a header followed by one
//! record per packet, in the order the proxy received them, both directions and every
//! peer interleaved. Packets are stored decrypted, so reading a capture never needs
//! the session keys. A packet that could not be decrypted is stored as received and
//! flagged as such: this is normally the client's `baseAppLogin`, which is sent before
//! encryption is enabled.
//!
//! The stream is flushed regularly, so a capture cut short by killing the proxy is
//! still readable up to its last flush; [`CaptureReader`] treats a truncated tail as
//! the end of the capture.
//!
//! Layout, all integers little endian:
//!
//! ```text
//! header: b"WGTKCAP\0", u32 version, u64 start time (µs since the Unix epoch)
//! record: u8 flags, u64 time (µs since start), addr peer, addr real, u16 len, [u8; len]
//! addr:   u8 4, [u8; 4] ip, u16 port
//!       | u8 6, [u8; 16] ip, u16 port
//! flags:  bit 0 = direction (0 out, from the peer; 1 in, toward the peer)
//!         bit 1 = undecryptable (the packet is stored as received)
//! ```

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::fs::File;

use byteorder::{LittleEndian as LE, ReadBytesExt, WriteBytesExt};

use flate2::write::GzEncoder;
use flate2::read::GzDecoder;
use flate2::Compression;

use crate::net::packet::{Packet, PACKET_CAP, PACKET_HEADER_LEN};

use super::PacketDirection;


const MAGIC: &[u8; 8] = b"WGTKCAP\0";
const VERSION: u32 = 1;

const FLAG_IN: u8 = 0x01;
const FLAG_UNDECRYPTABLE: u8 = 0x02;

/// How long written records may stay buffered before being flushed to the file.
const FLUSH_INTERVAL: Duration = Duration::from_secs(1);


/// One captured packet.
#[derive(Debug, Clone)]
pub struct CaptureRecord {
    /// Time since the start of the capture.
    pub time: Duration,
    pub direction: PacketDirection,
    /// The local peer (the client) this packet was exchanged with.
    pub peer_addr: SocketAddr,
    /// The real application the peer was forwarded to at that time.
    pub real_addr: SocketAddr,
    /// The decrypted packet, or the packet as received if it could not be decrypted.
    pub packet: Packet,
    /// The packet could not be decrypted and is stored as received.
    pub undecryptable: bool,
}

/// Writes a capture, see the [module documentation](self).
#[derive(Debug)]
pub struct CaptureWriter<W: Write> {
    inner: GzEncoder<W>,
    start: Instant,
    last_flush: Instant,
}

impl CaptureWriter<BufWriter<File>> {

    /// Create a capture file at the given path, replacing any existing one.
    pub fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::new(BufWriter::new(File::create(path)?))
    }

}

impl<W: Write> CaptureWriter<W> {

    /// Start a capture on the given writer, the header is written immediately.
    pub fn new(writer: W) -> io::Result<Self> {

        let mut inner = GzEncoder::new(writer, Compression::default());
        let start_unix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();

        inner.write_all(MAGIC)?;
        inner.write_u32::<LE>(VERSION)?;
        inner.write_u64::<LE>(start_unix.as_micros() as u64)?;

        let now = Instant::now();
        Ok(Self { inner, start: now, last_flush: now })

    }

    /// Append a packet received now.
    pub fn write(&mut self,
        direction: PacketDirection,
        peer_addr: SocketAddr,
        real_addr: SocketAddr,
        packet: &Packet,
        undecryptable: bool,
    ) -> io::Result<()> {

        let mut flags = 0;
        if direction == PacketDirection::In { flags |= FLAG_IN; }
        if undecryptable { flags |= FLAG_UNDECRYPTABLE; }

        let w = &mut self.inner;
        w.write_u8(flags)?;
        w.write_u64::<LE>(self.start.elapsed().as_micros() as u64)?;
        write_addr(w, peer_addr)?;
        write_addr(w, real_addr)?;
        w.write_u16::<LE>(packet.len() as u16)?;
        w.write_all(packet.slice())?;

        if self.last_flush.elapsed() >= FLUSH_INTERVAL {
            self.flush()?;
        }

        Ok(())

    }

    /// Flush everything written so far to the underlying writer, so it can be read back
    /// even if the capture is never finished.
    pub fn flush(&mut self) -> io::Result<()> {
        self.last_flush = Instant::now();
        self.inner.flush()
    }

    /// Terminate the gzip stream and return the underlying writer.
    pub fn finish(self) -> io::Result<W> {
        self.inner.finish()
    }

}

/// Reads a capture, see the [module documentation](self).
///
/// Iterating yields every record in order. A capture whose tail is truncated (the
/// proxy was killed before finishing it) simply ends at its last complete record,
/// [`Self::truncated`] tells if that happened.
#[derive(Debug)]
pub struct CaptureReader<R: Read> {
    inner: GzDecoder<R>,
    start_unix: Duration,
    truncated: bool,
    done: bool,
}

impl CaptureReader<BufReader<File>> {

    /// Open a capture file.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::new(BufReader::new(File::open(path)?))
    }

}

impl<R: Read> CaptureReader<R> {

    /// Read the header of a capture from the given reader.
    pub fn new(reader: R) -> io::Result<Self> {

        let mut inner = GzDecoder::new(reader);

        let mut magic = [0; 8];
        inner.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "not a packet capture"));
        }

        let version = inner.read_u32::<LE>()?;
        if version != VERSION {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                format!("unsupported packet capture version {version}")));
        }

        let start_unix = Duration::from_micros(inner.read_u64::<LE>()?);
        Ok(Self {
            inner,
            start_unix,
            truncated: false,
            done: false,
        })

    }

    /// When the capture started, as a duration since the Unix epoch.
    pub fn start_unix(&self) -> Duration {
        self.start_unix
    }

    /// Whether the capture ended on a truncated record or an unfinished stream.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    fn read_record(&mut self) -> io::Result<Option<CaptureRecord>> {

        let r = &mut self.inner;

        // A finished stream ends cleanly here, with no more bytes, while an unfinished
        // one makes the decoder fail with an unexpected end of file.
        let mut flags = [0];
        if r.read(&mut flags)? == 0 {
            return Ok(None);
        }
        let flags = flags[0];

        let time = Duration::from_micros(r.read_u64::<LE>()?);
        let peer_addr = read_addr(r)?;
        let real_addr = read_addr(r)?;

        let len = r.read_u16::<LE>()? as usize;
        if !(PACKET_HEADER_LEN..=PACKET_CAP).contains(&len) {
            return Err(io::Error::new(io::ErrorKind::InvalidData,
                format!("captured packet of {len} bytes is not a valid packet length")));
        }

        let mut packet = Packet::new();
        r.read_exact(&mut packet.buf_mut()[..len])?;
        packet.set_len(len);

        Ok(Some(CaptureRecord {
            time,
            direction: if flags & FLAG_IN != 0 { PacketDirection::In } else { PacketDirection::Out },
            peer_addr,
            real_addr,
            packet,
            undecryptable: flags & FLAG_UNDECRYPTABLE != 0,
        }))

    }

}

impl<R: Read> Iterator for CaptureReader<R> {

    type Item = io::Result<CaptureRecord>;

    fn next(&mut self) -> Option<Self::Item> {

        if self.done {
            return None;
        }

        match self.read_record() {
            Ok(Some(record)) => Some(Ok(record)),
            Ok(None) => {
                self.done = true;
                None
            }
            // A record cut in the middle, or a stream without its gzip trailer.
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                self.done = true;
                self.truncated = true;
                None
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }

    }

}

fn write_addr(w: &mut impl Write, addr: SocketAddr) -> io::Result<()> {
    match addr.ip() {
        IpAddr::V4(ip) => {
            w.write_u8(4)?;
            w.write_all(&ip.octets())?;
        }
        IpAddr::V6(ip) => {
            w.write_u8(6)?;
            w.write_all(&ip.octets())?;
        }
    }
    w.write_u16::<LE>(addr.port())
}

fn read_addr(r: &mut impl Read) -> io::Result<SocketAddr> {
    let ip = match r.read_u8()? {
        4 => {
            let mut octets = [0; 4];
            r.read_exact(&mut octets)?;
            IpAddr::V4(Ipv4Addr::from(octets))
        }
        6 => {
            let mut octets = [0; 16];
            r.read_exact(&mut octets)?;
            IpAddr::V6(Ipv6Addr::from(octets))
        }
        family => return Err(io::Error::new(io::ErrorKind::InvalidData,
            format!("invalid address family {family} in packet capture"))),
    };
    Ok(SocketAddr::new(ip, r.read_u16::<LE>()?))
}


#[cfg(test)]
mod tests {

    use super::*;

    fn packet(content: &[u8]) -> Packet {
        let mut packet = Packet::new();
        packet.buf_mut()[..content.len()].copy_from_slice(content);
        packet.set_len(content.len());
        packet
    }

    fn write_sample() -> Vec<u8> {
        let peer = "127.0.0.1:50000".parse().unwrap();
        let real = "[2001:db8::1]:20017".parse().unwrap();
        let mut writer = CaptureWriter::new(Vec::new()).unwrap();
        writer.write(PacketDirection::Out, peer, real, &packet(&[1, 2, 3, 4, 5, 6, 7]), true).unwrap();
        writer.write(PacketDirection::In, peer, real, &packet(&[4; 600]), false).unwrap();
        writer.finish().unwrap()
    }

    #[test]
    fn round_trip() {

        let data = write_sample();
        let mut reader = CaptureReader::new(&data[..]).unwrap();

        let first = reader.next().unwrap().unwrap();
        assert_eq!(first.direction, PacketDirection::Out);
        assert!(first.undecryptable);
        assert_eq!(first.packet.slice(), &[1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(first.peer_addr, "127.0.0.1:50000".parse().unwrap());
        assert_eq!(first.real_addr, "[2001:db8::1]:20017".parse().unwrap());

        let second = reader.next().unwrap().unwrap();
        assert_eq!(second.direction, PacketDirection::In);
        assert!(!second.undecryptable);
        assert_eq!(second.packet.slice(), &[4; 600]);
        assert!(second.time >= first.time);

        assert!(reader.next().is_none());
        assert!(!reader.truncated());

    }

    /// A capture cut anywhere still yields every complete record before the cut.
    #[test]
    fn truncated_capture() {

        let peer = "127.0.0.1:50000".parse().unwrap();
        let mut writer = CaptureWriter::new(Vec::new()).unwrap();
        writer.write(PacketDirection::Out, peer, peer, &packet(&[1, 2, 3, 4, 5, 6, 7]), false).unwrap();
        writer.flush().unwrap();
        writer.write(PacketDirection::In, peer, peer, &packet(&[4; 600]), false).unwrap();
        // Never finished: the second record is still in the encoder, and the stream
        // has no trailer, like a proxy killed mid-capture.
        let data = writer.inner.get_ref().clone();

        let mut reader = CaptureReader::new(&data[..]).unwrap();
        assert_eq!(reader.next().unwrap().unwrap().packet.slice(), &[1, 2, 3, 4, 5, 6, 7]);
        assert!(reader.next().is_none());
        assert!(reader.truncated());

    }

}
