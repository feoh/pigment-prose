//! Streaming PNG output for exports (task 09).
//!
//! Encoding policy (docs/architecture.md, "Color, alpha and output
//! encoding"; docs/export.md):
//! - 8-bit **RGB**, non-interlaced. The painting is opaque paper, so the
//!   renderer's constant alpha of 255 is dropped rather than stored.
//! - One `sRGB` chunk (perceptual intent) and nothing else besides `IHDR`,
//!   `IDAT` and `IEND`: no text, time, EXIF, physical-size (DPI) or ICC
//!   chunks, so no prose, path, identity or watermark can reach the file.
//! - Rows are compressed as they arrive (`png::StreamWriter`), so host memory
//!   is one renderer band plus the encoder's buffers, never the whole image.
//! - The file is written beside the destination and renamed into place only
//!   after `IEND` is written and synced ([`crate::atomic`]).

use std::fmt;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use pigment_core::error::{SinkError, SinkErrorKind};
use pigment_core::request::TileSink;

use crate::atomic::AtomicFile;

/// The 12 bytes of every PNG's final chunk.
const IEND: [u8; 12] = [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82];

/// Deflate effort. Output is lossless either way; this trades file size for
/// export time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PngCompression {
    /// `png::Compression::Fast` (fdeflate). The export default.
    #[default]
    Fast,
    /// `png::Compression::Balanced`: smaller files, slower.
    Balanced,
}

impl PngCompression {
    fn png(self) -> png::Compression {
        match self {
            PngCompression::Fast => png::Compression::Fast,
            PngCompression::Balanced => png::Compression::Balanced,
        }
    }
}

/// `StorageFull`/`QuotaExceeded` become [`SinkErrorKind::DiskFull`].
pub(crate) fn io_error(context: &str, e: &io::Error) -> SinkError {
    let kind = match e.kind() {
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded => SinkErrorKind::DiskFull,
        _ => SinkErrorKind::Io,
    };
    SinkError {
        kind,
        detail: format!("{context}: {e}"),
    }
}

fn encoding_error(context: &str, e: png::EncodingError) -> SinkError {
    match e {
        png::EncodingError::IoError(e) => io_error(context, &e),
        other => SinkError {
            kind: SinkErrorKind::Encode,
            detail: format!("{context}: {other}"),
        },
    }
}

fn misuse(detail: String) -> SinkError {
    SinkError {
        kind: SinkErrorKind::Other,
        detail,
    }
}

/// What the encoder has written so far, shared between the `StreamWriter`
/// (which owns its writer) and the encoder that must get it back.
struct Tracked<W> {
    out: Option<W>,
    written: u64,
    tail: [u8; 12],
    /// The last write failure, which the png crate may have discarded.
    failed: Option<io::ErrorKind>,
}

/// A `Write` handle onto [`Tracked`]. Keeps the byte count and the last 12
/// bytes, so `finish` can prove `IEND` really reached the writer (the png
/// crate writes it from a destructor and swallows that error).
struct SharedWriter<W>(Arc<Mutex<Tracked<W>>>);

impl<W: Write> Write for SharedWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut t = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let out = t
            .out
            .as_mut()
            .ok_or_else(|| io::Error::other("the output was already closed"))?;
        let n = match out.write(buf) {
            Ok(n) => n,
            Err(e) => {
                t.failed = Some(e.kind());
                return Err(e);
            }
        };
        t.written += n as u64;
        let tail = t.tail;
        let keep = 12usize.saturating_sub(n);
        let mut next = [0u8; 12];
        next[..keep].copy_from_slice(&tail[12 - keep..]);
        let take = n.min(12);
        next[keep..].copy_from_slice(&buf[n - take..n]);
        t.tail = next;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut t = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        t.out.as_mut().map_or(Ok(()), Write::flush)
    }
}

/// Band-by-band PNG encoder over any writer. [`PngSink`] uses it on a file;
/// tests use it on memory and on failing writers.
pub(crate) struct BandEncoder<W: Write + Send + 'static> {
    shared: Arc<Mutex<Tracked<W>>>,
    stream: Option<png::StreamWriter<'static, SharedWriter<W>>>,
    width: u32,
    height: u32,
    next_row: u32,
    rgb_row: Vec<u8>,
}

impl<W: Write + Send + 'static> BandEncoder<W> {
    pub(crate) fn new(
        out: W,
        width: u32,
        height: u32,
        compression: PngCompression,
    ) -> Result<BandEncoder<W>, SinkError> {
        if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
            return Err(SinkError {
                kind: SinkErrorKind::Encode,
                detail: format!("{width}x{height} is not a valid PNG size"),
            });
        }
        let row = (width as usize)
            .checked_mul(3)
            .ok_or_else(|| misuse(format!("a {width} px row overflows")))?;
        let shared = Arc::new(Mutex::new(Tracked {
            out: Some(out),
            written: 0,
            tail: [0; 12],
            failed: None,
        }));
        let mut enc = png::Encoder::new(SharedWriter(shared.clone()), width, height);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        enc.set_compression(compression.png());
        let header = enc
            .write_header()
            .map_err(|e| encoding_error("writing the PNG header", e))?;
        // 256 KiB IDAT chunks: few chunk headers, bounded buffering.
        let stream = header
            .into_stream_writer_with_size(256 << 10)
            .map_err(|e| encoding_error("starting the PNG stream", e))?;
        Ok(BandEncoder {
            shared,
            stream: Some(stream),
            width,
            height,
            next_row: 0,
            rgb_row: vec![0; row],
        })
    }

    pub(crate) fn band(
        &mut self,
        first_row: u32,
        rows: u32,
        rgba8: &[u8],
    ) -> Result<(), SinkError> {
        let stride = self.width as usize * 4;
        if first_row != self.next_row
            || rows == 0
            || rows > self.height - self.next_row
            || rgba8.len() != rows as usize * stride
        {
            return Err(misuse(format!(
                "band of {rows} rows ({} bytes) at row {first_row}; expected row {} of {} \
                 with {stride} bytes per row",
                rgba8.len(),
                self.next_row,
                self.height
            )));
        }
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| misuse("the PNG stream is closed".into()))?;
        for row in rgba8.chunks_exact(stride) {
            for (d, s) in self
                .rgb_row
                .as_chunks_mut::<3>()
                .0
                .iter_mut()
                .zip(row.as_chunks::<4>().0)
            {
                *d = [s[0], s[1], s[2]];
            }
            if let Err(e) = stream.write_all(&self.rgb_row) {
                return Err(self.recorded_error("writing image rows", e));
            }
        }
        self.next_row += rows;
        Ok(())
    }

    /// The png crate re-wraps writer errors; report the kind the writer
    /// actually failed with.
    fn recorded_error(&self, context: &str, e: io::Error) -> SinkError {
        let failed = self
            .shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .failed;
        match failed {
            Some(kind) => io_error(context, &io::Error::new(kind, e)),
            None => io_error(context, &e),
        }
    }

    /// Finish the stream, check that `IEND` was written, and return the
    /// writer with the total byte count.
    pub(crate) fn finish(mut self) -> Result<(W, u64), SinkError> {
        if self.next_row != self.height {
            return Err(misuse(format!(
                "finished after {} of {} rows",
                self.next_row, self.height
            )));
        }
        let stream = self
            .stream
            .take()
            .ok_or_else(|| misuse("the PNG stream is closed".into()))?;
        // Writes the last IDAT; dropping the owned inner writer writes IEND.
        if let Err(e) = stream.finish() {
            let e = match e {
                png::EncodingError::IoError(e) => e,
                other => return Err(encoding_error("finishing the PNG stream", other)),
            };
            return Err(self.recorded_error("finishing the PNG stream", e));
        }
        let mut t = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        let mut out = t
            .out
            .take()
            .ok_or_else(|| misuse("the output was already closed".into()))?;
        out.flush().map_err(|e| io_error("flushing the PNG", &e))?;
        if t.tail != IEND {
            let e = io::Error::from(t.failed.unwrap_or(io::ErrorKind::WriteZero));
            return Err(io_error("writing the PNG end marker", &e));
        }
        Ok((out, t.written))
    }
}

impl<W: Write + Send + 'static> Drop for BandEncoder<W> {
    fn drop(&mut self) {
        // An abandoned stream's destructor still writes; close the output
        // first so nothing more reaches it.
        self.shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .out = None;
    }
}

/// A [`TileSink`] that streams an export to `destination` as PNG.
///
/// Lifecycle: [`PngSink::create`] opens a temporary file beside the
/// destination (failing early for a missing or unwritable directory);
/// `begin` writes the header; `band` compresses rows; `finish` completes and
/// syncs the file and renames it over the destination; `abort` (or drop)
/// deletes the temporary file. An existing destination is replaced only by
/// a complete file. Overwrite confirmation belongs to the UI, not here.
pub struct PngSink {
    destination: PathBuf,
    compression: PngCompression,
    file: Option<AtomicFile>,
    encoder: Option<BandEncoder<BufWriter<File>>>,
    bytes: u64,
    finished: bool,
}

impl fmt::Debug for PngSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PngSink")
            .field("destination", &self.destination)
            .field("compression", &self.compression)
            .field("open", &self.file.is_some())
            .field("bytes", &self.bytes)
            .field("finished", &self.finished)
            .finish()
    }
}

impl PngSink {
    pub fn create(destination: &Path) -> Result<PngSink, SinkError> {
        PngSink::with_compression(destination, PngCompression::default())
    }

    pub fn with_compression(
        destination: &Path,
        compression: PngCompression,
    ) -> Result<PngSink, SinkError> {
        let file = AtomicFile::create(destination)
            .map_err(|e| io_error(&format!("cannot write {}", destination.display()), &e))?;
        Ok(PngSink {
            destination: destination.to_path_buf(),
            compression,
            file: Some(file),
            encoder: None,
            bytes: 0,
            finished: false,
        })
    }

    pub fn destination(&self) -> &Path {
        &self.destination
    }

    /// The temporary file being written, until finish or abort.
    pub fn temp_path(&self) -> Option<&Path> {
        self.file.as_ref().map(AtomicFile::temp_path)
    }

    /// Size of the finished file in bytes (0 before `finish`).
    pub fn bytes_written(&self) -> u64 {
        self.bytes
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    fn context(&self) -> String {
        format!("writing {}", self.destination.display())
    }

    fn with_context(&self, mut e: SinkError) -> SinkError {
        e.detail = format!("{}: {}", self.context(), e.detail);
        e
    }
}

impl TileSink for PngSink {
    fn begin(&mut self, width: u32, height: u32) -> Result<(), SinkError> {
        let file = self
            .file
            .as_mut()
            .and_then(AtomicFile::file)
            .ok_or_else(|| misuse("the export was already finished or aborted".into()))?;
        let handle = file
            .try_clone()
            .map_err(|e| io_error("opening the temporary file", &e))?;
        let enc = BandEncoder::new(
            BufWriter::with_capacity(1 << 20, handle),
            width,
            height,
            self.compression,
        )
        .map_err(|e| self.with_context(e))?;
        self.encoder = Some(enc);
        Ok(())
    }

    fn band(&mut self, first_row: u32, rows: u32, rgba8: &[u8]) -> Result<(), SinkError> {
        let Some(enc) = self.encoder.as_mut() else {
            return Err(misuse("band before begin".into()));
        };
        enc.band(first_row, rows, rgba8)
            .map_err(|e| self.with_context(e))
    }

    fn finish(&mut self) -> Result<(), SinkError> {
        let enc = self
            .encoder
            .take()
            .ok_or_else(|| misuse("finish before begin".into()))?;
        let (buffered, bytes) = enc.finish().map_err(|e| self.with_context(e))?;
        // Close the cloned handle before the rename (required on Windows).
        let handle = buffered
            .into_inner()
            .map_err(|e| io_error(&self.context(), e.error()))?;
        drop(handle);
        let file = self
            .file
            .take()
            .ok_or_else(|| misuse("the export was already finished or aborted".into()))?;
        file.commit()
            .map_err(|e| io_error(&format!("saving {}", self.destination.display()), &e))?;
        self.bytes = bytes;
        self.finished = true;
        Ok(())
    }

    fn abort(&mut self) {
        self.encoder = None;
        if let Some(f) = self.file.take() {
            f.discard();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    fn gradient(w: u32, h: u32) -> Vec<u8> {
        (0..h)
            .flat_map(|y| (0..w).flat_map(move |x| [x as u8, y as u8, (x ^ y) as u8, 255]))
            .collect()
    }

    fn feed(sink: &mut dyn TileSink, w: u32, h: u32, band: u32) {
        let img = gradient(w, h);
        sink.begin(w, h).unwrap();
        let mut y = 0;
        while y < h {
            let rows = band.min(h - y);
            let s = (y * w * 4) as usize;
            sink.band(y, rows, &img[s..s + (rows * w * 4) as usize])
                .unwrap();
            y += rows;
        }
    }

    /// Chunk types in file order.
    pub(crate) fn chunk_types(png: &[u8]) -> Vec<String> {
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let mut i = 8;
        let mut out = Vec::new();
        while i < png.len() {
            let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
            out.push(String::from_utf8_lossy(&png[i + 4..i + 8]).into_owned());
            i += 12 + len;
        }
        out
    }

    fn decode(bytes: &[u8]) -> (png::OutputInfo, Vec<u8>) {
        let dec = png::Decoder::new(io::Cursor::new(bytes));
        let mut r = dec.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        buf.truncate(info.buffer_size());
        (info, buf)
    }

    #[test]
    fn streams_rgb_srgb_with_no_metadata() {
        let dir = TestDir::new("png-sink");
        let dest = dir.path().join("out.png");
        let mut sink = PngSink::create(&dest).unwrap();
        feed(&mut sink, 301, 157, 64);
        sink.finish().unwrap();
        assert!(sink.is_finished());
        let bytes = std::fs::read(&dest).unwrap();
        assert_eq!(sink.bytes_written(), bytes.len() as u64);
        let chunks = chunk_types(&bytes);
        assert_eq!(chunks.first().map(String::as_str), Some("IHDR"));
        assert_eq!(chunks[1], "sRGB");
        assert_eq!(chunks.last().map(String::as_str), Some("IEND"));
        assert!(
            chunks[2..chunks.len() - 1].iter().all(|c| c == "IDAT"),
            "{chunks:?}"
        );
        let (info, rgb) = decode(&bytes);
        assert_eq!((info.width, info.height), (301, 157));
        assert_eq!(info.color_type, png::ColorType::Rgb);
        assert_eq!(info.bit_depth, png::BitDepth::Eight);
        let want: Vec<u8> = gradient(301, 157)
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2]])
            .collect();
        assert!(rgb == want, "lossless round trip");
        assert_eq!(dir.entries(), vec!["out.png".to_string()]);
    }

    #[test]
    fn abort_removes_the_partial_file_and_keeps_the_old_one() {
        let dir = TestDir::new("png-abort");
        let dest = dir.path().join("keep.png");
        std::fs::write(&dest, b"previous export").unwrap();
        let mut sink = PngSink::create(&dest).unwrap();
        let img = gradient(64, 64);
        sink.begin(64, 64).unwrap();
        sink.band(0, 32, &img[..64 * 32 * 4]).unwrap();
        let temp = sink.temp_path().unwrap().to_path_buf();
        assert!(temp.exists());
        sink.abort();
        sink.abort(); // idempotent
        assert!(!temp.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"previous export");
        assert_eq!(dir.entries(), vec!["keep.png".to_string()]);
    }

    #[test]
    fn dropping_an_unfinished_sink_cleans_up() {
        let dir = TestDir::new("png-drop");
        {
            let mut sink = PngSink::create(&dir.path().join("x.png")).unwrap();
            sink.begin(64, 64).unwrap();
        }
        assert!(dir.entries().is_empty());
    }

    #[test]
    fn misordered_or_short_bands_are_errors() {
        let dir = TestDir::new("png-misuse");
        let mut sink = PngSink::create(&dir.path().join("x.png")).unwrap();
        let img = gradient(64, 64);
        sink.begin(64, 64).unwrap();
        assert!(sink.band(8, 8, &img[..64 * 8 * 4]).is_err(), "skipped rows");
        assert!(sink.band(0, 8, &img[..64 * 7 * 4]).is_err(), "short data");
        sink.band(0, 8, &img[..64 * 8 * 4]).unwrap();
        let e = sink.finish().unwrap_err();
        assert!(e.detail.contains("8 of 64 rows"), "{}", e.detail);
        sink.abort();
        assert!(dir.entries().is_empty());
    }

    #[test]
    fn unwritable_destinations_fail_at_create() {
        let dir = TestDir::new("png-unwritable");
        let e = PngSink::create(&dir.path().join("no/such/dir/x.png")).unwrap_err();
        assert_eq!(e.kind, SinkErrorKind::Io);
        assert!(e.detail.contains("cannot write"), "{}", e.detail);
        assert!(PngSink::create(dir.path()).is_err(), "a directory");
    }

    /// Accepts `limit` bytes, then fails like a full disk.
    struct FullDisk {
        limit: usize,
        written: usize,
    }

    impl Write for FullDisk {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.written + buf.len() > self.limit {
                return Err(io::Error::from(io::ErrorKind::StorageFull));
            }
            self.written += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_full_disk_is_reported_as_disk_full() {
        // Fails in the header, while streaming rows, and at IEND.
        for limit in [10, 5_000, 0] {
            let (w, h) = (512u32, 512u32);
            let img: Vec<u8> = (0..w * h * 4)
                .map(|i| (i.wrapping_mul(2654435761) >> 24) as u8)
                .collect();
            let full = FullDisk { limit, written: 0 };
            let result = BandEncoder::new(full, w, h, PngCompression::Fast).and_then(|mut e| {
                e.band(0, h, &img)?;
                e.finish().map(|_| ())
            });
            let err = result.unwrap_err();
            assert_eq!(err.kind, SinkErrorKind::DiskFull, "limit {limit}: {err}");
        }
        // Enough room for everything but the 12-byte IEND chunk.
        let mut probe = BandEncoder::new(Vec::new(), 64, 64, PngCompression::Fast).unwrap();
        probe.band(0, 64, &gradient(64, 64)).unwrap();
        let (full, n) = probe.finish().unwrap();
        assert_eq!(full.len() as u64, n);
        let mut enc = BandEncoder::new(
            FullDisk {
                limit: full.len() - 1,
                written: 0,
            },
            64,
            64,
            PngCompression::Fast,
        )
        .unwrap();
        enc.band(0, 64, &gradient(64, 64)).unwrap();
        let err = enc.finish().map(|_| ()).unwrap_err();
        assert_eq!(err.kind, SinkErrorKind::DiskFull, "{err}");
    }

    #[test]
    fn invalid_sizes_are_rejected_before_writing() {
        for (w, h) in [(0, 10), (10, 0), (u32::MAX, 1)] {
            assert!(BandEncoder::new(Vec::new(), w, h, PngCompression::Fast).is_err());
        }
    }
}
