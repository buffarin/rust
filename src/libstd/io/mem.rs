// Copyright 2013 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Readers and Writers for in-memory buffers

use cmp::max;
use cmp::min;
use container::Container;
use option::{Option, Some, None};
use super::{Reader, Writer, Seek, Buffer, IoError, SeekStyle, io_error,
            OtherIoError};
use vec;
use vec::{Vector, ImmutableVector, MutableVector, OwnedCloneableVector};

/// Writes to an owned, growable byte vector
///
/// # Example
///
/// ```rust
/// use std::io::MemWriter;
///
/// let mut w = MemWriter::new();
/// w.write([0, 1, 2]);
///
/// assert_eq!(w.unwrap(), ~[0, 1, 2]);
/// ```
pub struct MemWriter {
    priv buf: ~[u8],
    priv pos: uint,
}

impl MemWriter {
    /// Create a new `MemWriter`.
    pub fn new() -> MemWriter {
        MemWriter::with_capacity(128)
    }
    /// Create a new `MemWriter`, allocating at least `n` bytes for
    /// the internal buffer.
    pub fn with_capacity(n: uint) -> MemWriter {
        MemWriter { buf: vec::with_capacity(n), pos: 0 }
    }

    /// Acquires an immutable reference to the underlying buffer of this
    /// `MemWriter`.
    ///
    /// No method is exposed for acquiring a mutable reference to the buffer
    /// because it could corrupt the state of this `MemWriter`.
    pub fn get_ref<'a>(&'a self) -> &'a [u8] { self.buf.as_slice() }

    /// Unwraps this `MemWriter`, returning the underlying buffer
    pub fn unwrap(self) -> ~[u8] { self.buf }
}

impl Writer for MemWriter {
    fn write(&mut self, buf: &[u8]) {
        // Make sure the internal buffer is as least as big as where we
        // currently are
        let difference = self.pos as i64 - self.buf.len() as i64;
        if difference > 0 {
            self.buf.grow(difference as uint, &0);
        }

        // Figure out what bytes will be used to overwrite what's currently
        // there (left), and what will be appended on the end (right)
        let cap = self.buf.len() - self.pos;
        let (left, right) = if cap <= buf.len() {
            (buf.slice_to(cap), buf.slice_from(cap))
        } else {
            (buf, &[])
        };

        // Do the necessary writes
        if left.len() > 0 {
            vec::bytes::copy_memory(self.buf.mut_slice_from(self.pos), left);
        }
        if right.len() > 0 {
            self.buf.push_all(right);
        }

        // Bump us forward
        self.pos += buf.len();
    }
}

// FIXME(#10432)
impl Seek for MemWriter {
    fn tell(&self) -> u64 { self.pos as u64 }

    fn seek(&mut self, pos: i64, style: SeekStyle) {
        // compute offset as signed and clamp to prevent overflow
        let offset = match style {
            SeekSet => { 0 }
            SeekEnd => { self.buf.len() }
            SeekCur => { self.pos }
        } as i64;

        self.pos = max(0, offset+pos) as uint;
    }
}

/// Reads from an owned byte vector
///
/// # Example
///
/// ```rust
/// use std::io::MemReader;
///
/// let mut r = MemReader::new(~[0, 1, 2]);
///
/// assert_eq!(r.read_to_end(), ~[0, 1, 2]);
/// ```
pub struct MemReader {
    priv buf: ~[u8],
    priv pos: uint
}

impl MemReader {
    /// Creates a new `MemReader` which will read the buffer given. The buffer
    /// can be re-acquired through `unwrap`
    pub fn new(buf: ~[u8]) -> MemReader {
        MemReader {
            buf: buf,
            pos: 0
        }
    }

    /// Tests whether this reader has read all bytes in its buffer.
    ///
    /// If `true`, then this will no longer return bytes from `read`.
    pub fn eof(&self) -> bool { self.pos == self.buf.len() }

    /// Acquires an immutable reference to the underlying buffer of this
    /// `MemReader`.
    ///
    /// No method is exposed for acquiring a mutable reference to the buffer
    /// because it could corrupt the state of this `MemReader`.
    pub fn get_ref<'a>(&'a self) -> &'a [u8] { self.buf.as_slice() }

    /// Unwraps this `MemReader`, returning the underlying buffer
    pub fn unwrap(self) -> ~[u8] { self.buf }
}

impl Reader for MemReader {
    fn read(&mut self, buf: &mut [u8]) -> Option<uint> {
        if self.eof() { return None }

        let write_len = min(buf.len(), self.buf.len() - self.pos);
        {
            let input = self.buf.slice(self.pos, self.pos + write_len);
            let output = buf.mut_slice(0, write_len);
            assert_eq!(input.len(), output.len());
            vec::bytes::copy_memory(output, input);
        }
        self.pos += write_len;
        assert!(self.pos <= self.buf.len());

        return Some(write_len);
    }
}

impl Seek for MemReader {
    fn tell(&self) -> u64 { self.pos as u64 }
    fn seek(&mut self, _pos: i64, _style: SeekStyle) { fail!() }
}

impl Buffer for MemReader {
    fn fill<'a>(&'a mut self) -> &'a [u8] { self.buf.slice_from(self.pos) }
    fn consume(&mut self, amt: uint) { self.pos += amt; }
}

/// Writes to a fixed-size byte slice
///
/// If a write will not fit in the buffer, it raises the `io_error`
/// condition and does not write any data.
///
/// # Example
///
/// ```rust
/// use std::io::BufWriter;
///
/// let mut buf = [0, ..4];
/// {
///     let mut w = BufWriter::new(buf);
///     w.write([0, 1, 2]);
/// }
/// assert_eq!(buf, [0, 1, 2, 0]);
/// ```
pub struct BufWriter<'a> {
    priv buf: &'a mut [u8],
    priv pos: uint
}

impl<'a> BufWriter<'a> {
    pub fn new<'a>(buf: &'a mut [u8]) -> BufWriter<'a> {
        BufWriter {
            buf: buf,
            pos: 0
        }
    }
}

impl<'a> Writer for BufWriter<'a> {
    fn write(&mut self, buf: &[u8]) {
        // raises a condition if the entire write does not fit in the buffer
        let max_size = self.buf.len();
        if self.pos >= max_size || (self.pos + buf.len()) > max_size {
            io_error::cond.raise(IoError {
                kind: OtherIoError,
                desc: "Trying to write past end of buffer",
                detail: None
            });
            return;
        }

        vec::bytes::copy_memory(self.buf.mut_slice_from(self.pos), buf);
        self.pos += buf.len();
    }
}

// FIXME(#10432)
impl<'a> Seek for BufWriter<'a> {
    fn tell(&self) -> u64 { self.pos as u64 }

    fn seek(&mut self, pos: i64, style: SeekStyle) {
        // compute offset as signed and clamp to prevent overflow
        let offset = match style {
            SeekSet => { 0 }
            SeekEnd => { self.buf.len() }
            SeekCur => { self.pos }
        } as i64;

        self.pos = max(0, offset+pos) as uint;
    }
}


/// Reads from a fixed-size byte slice
///
/// # Example
///
/// ```rust
/// use std::io::BufReader;
///
/// let mut buf = [0, 1, 2, 3];
/// let mut r = BufReader::new(buf);
///
/// assert_eq!(r.read_to_end(), ~[0, 1, 2, 3]);
/// ```
pub struct BufReader<'a> {
    priv buf: &'a [u8],
    priv pos: uint
}

impl<'a> BufReader<'a> {
    /// Creates a new buffered reader which will read the specified buffer
    pub fn new<'a>(buf: &'a [u8]) -> BufReader<'a> {
        BufReader {
            buf: buf,
            pos: 0
        }
    }

    /// Tests whether this reader has read all bytes in its buffer.
    ///
    /// If `true`, then this will no longer return bytes from `read`.
    pub fn eof(&self) -> bool { self.pos == self.buf.len() }
}

impl<'a> Reader for BufReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> Option<uint> {
        if self.eof() { return None }

        let write_len = min(buf.len(), self.buf.len() - self.pos);
        {
            let input = self.buf.slice(self.pos, self.pos + write_len);
            let output = buf.mut_slice(0, write_len);
            assert_eq!(input.len(), output.len());
            vec::bytes::copy_memory(output, input);
        }
        self.pos += write_len;
        assert!(self.pos <= self.buf.len());

        return Some(write_len);
     }
}

impl<'a> Seek for BufReader<'a> {
    fn tell(&self) -> u64 { self.pos as u64 }

    fn seek(&mut self, _pos: i64, _style: SeekStyle) { fail!() }
}

impl<'a> Buffer for BufReader<'a> {
    fn fill<'a>(&'a mut self) -> &'a [u8] { self.buf.slice_from(self.pos) }
    fn consume(&mut self, amt: uint) { self.pos += amt; }
}

#[cfg(test)]
mod test {
    use prelude::*;
    use super::*;
    use io::*;

    #[test]
    fn test_mem_writer() {
        let mut writer = MemWriter::new();
        assert_eq!(writer.tell(), 0);
        writer.write([0]);
        assert_eq!(writer.tell(), 1);
        writer.write([1, 2, 3]);
        writer.write([4, 5, 6, 7]);
        assert_eq!(writer.tell(), 8);
        assert_eq!(writer.get_ref(), [0, 1, 2, 3, 4, 5, 6, 7]);

        writer.seek(0, SeekSet);
        assert_eq!(writer.tell(), 0);
        writer.write([3, 4]);
        assert_eq!(writer.get_ref(), [3, 4, 2, 3, 4, 5, 6, 7]);

        writer.seek(1, SeekCur);
        writer.write([0, 1]);
        assert_eq!(writer.get_ref(), [3, 4, 2, 0, 1, 5, 6, 7]);

        writer.seek(-1, SeekEnd);
        writer.write([1, 2]);
        assert_eq!(writer.get_ref(), [3, 4, 2, 0, 1, 5, 6, 1, 2]);

        writer.seek(1, SeekEnd);
        writer.write([1]);
        assert_eq!(writer.get_ref(), [3, 4, 2, 0, 1, 5, 6, 1, 2, 0, 1]);
    }

    #[test]
    fn test_buf_writer() {
        let mut buf = [0 as u8, ..8];
        {
            let mut writer = BufWriter::new(buf);
            assert_eq!(writer.tell(), 0);
            writer.write([0]);
            assert_eq!(writer.tell(), 1);
            writer.write([1, 2, 3]);
            writer.write([4, 5, 6, 7]);
            assert_eq!(writer.tell(), 8);
        }
        assert_eq!(buf, [0, 1, 2, 3, 4, 5, 6, 7]);
    }

    #[test]
    fn test_buf_writer_seek() {
        let mut buf = [0 as u8, ..8];
        {
            let mut writer = BufWriter::new(buf);
            assert_eq!(writer.tell(), 0);
            writer.write([1]);
            assert_eq!(writer.tell(), 1);

            writer.seek(2, SeekSet);
            assert_eq!(writer.tell(), 2);
            writer.write([2]);
            assert_eq!(writer.tell(), 3);

            writer.seek(-2, SeekCur);
            assert_eq!(writer.tell(), 1);
            writer.write([3]);
            assert_eq!(writer.tell(), 2);

            writer.seek(-1, SeekEnd);
            assert_eq!(writer.tell(), 7);
            writer.write([4]);
            assert_eq!(writer.tell(), 8);

        }
        assert_eq!(buf, [1, 3, 2, 0, 0, 0, 0, 4]);
    }

    #[test]
    fn test_buf_writer_error() {
        let mut buf = [0 as u8, ..2];
        let mut writer = BufWriter::new(buf);
        writer.write([0]);

        let mut called = false;
        io_error::cond.trap(|err| {
            assert_eq!(err.kind, OtherIoError);
            called = true;
        }).inside(|| {
            writer.write([0, 0]);
        });
        assert!(called);
    }

    #[test]
    fn test_mem_reader() {
        let mut reader = MemReader::new(~[0, 1, 2, 3, 4, 5, 6, 7]);
        let mut buf = [];
        assert_eq!(reader.read(buf), Some(0));
        assert_eq!(reader.tell(), 0);
        let mut buf = [0];
        assert_eq!(reader.read(buf), Some(1));
        assert_eq!(reader.tell(), 1);
        assert_eq!(buf, [0]);
        let mut buf = [0, ..4];
        assert_eq!(reader.read(buf), Some(4));
        assert_eq!(reader.tell(), 5);
        assert_eq!(buf, [1, 2, 3, 4]);
        assert_eq!(reader.read(buf), Some(3));
        assert_eq!(buf.slice(0, 3), [5, 6, 7]);
        assert_eq!(reader.read(buf), None);
    }

    #[test]
    fn test_buf_reader() {
        let in_buf = ~[0, 1, 2, 3, 4, 5, 6, 7];
        let mut reader = BufReader::new(in_buf);
        let mut buf = [];
        assert_eq!(reader.read(buf), Some(0));
        assert_eq!(reader.tell(), 0);
        let mut buf = [0];
        assert_eq!(reader.read(buf), Some(1));
        assert_eq!(reader.tell(), 1);
        assert_eq!(buf, [0]);
        let mut buf = [0, ..4];
        assert_eq!(reader.read(buf), Some(4));
        assert_eq!(reader.tell(), 5);
        assert_eq!(buf, [1, 2, 3, 4]);
        assert_eq!(reader.read(buf), Some(3));
        assert_eq!(buf.slice(0, 3), [5, 6, 7]);
        assert_eq!(reader.read(buf), None);
    }

    #[test]
    fn test_read_char() {
        let b = bytes!("Việt");
        let mut r = BufReader::new(b);
        assert_eq!(r.read_char(), Some('V'));
        assert_eq!(r.read_char(), Some('i'));
        assert_eq!(r.read_char(), Some('ệ'));
        assert_eq!(r.read_char(), Some('t'));
        assert_eq!(r.read_char(), None);
    }

    #[test]
    fn test_read_bad_char() {
        let b = bytes!(0x80);
        let mut r = BufReader::new(b);
        assert_eq!(r.read_char(), None);
    }

    #[test]
    fn test_write_strings() {
        let mut writer = MemWriter::new();
        writer.write_str("testing");
        writer.write_line("testing");
        writer.write_str("testing");
        let mut r = BufReader::new(writer.get_ref());
        assert_eq!(r.read_to_str(), ~"testingtesting\ntesting");
    }

    #[test]
    fn test_write_char() {
        let mut writer = MemWriter::new();
        writer.write_char('a');
        writer.write_char('\n');
        writer.write_char('ệ');
        let mut r = BufReader::new(writer.get_ref());
        assert_eq!(r.read_to_str(), ~"a\nệ");
    }

    #[test]
    fn test_read_whole_string_bad() {
        let buf = [0xff];
        let mut r = BufReader::new(buf);
        match result(|| r.read_to_str()) {
            Ok(..) => fail!(),
            Err(..) => {}
        }
    }
}
