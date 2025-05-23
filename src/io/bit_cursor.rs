use std::{
    fmt::LowerHex,
    io::{Read, Seek, SeekFrom, Write},
};

use crate::prelude::*;

#[derive(Debug, Default, Eq, PartialEq)]
pub struct BitCursor<T> {
    inner: T,
    pos: u64,
}

impl<T> BitCursor<T> {
    /// Creates a new cursor wrapping the provided buffer.
    ///
    /// Cursor initial position is `0` even if the given buffer is not empty.
    pub fn new(inner: T) -> BitCursor<T> {
        BitCursor { inner, pos: 0 }
    }

    /// Gets a mutable reference to the inner value
    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }

    /// Gets a reference to the inner value
    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    /// Consumes the cursor, returning the inner value.
    pub fn into_inner(self) -> T {
        self.inner
    }

    /// Returns the position (in _bits_ since the start) of this cursor.
    pub fn position(&self) -> u64 {
        self.pos
    }

    /// Sets the position of this cursor (in _bits_ since the start)
    pub fn set_position(&mut self, pos: u64) {
        self.pos = pos;
    }
}

impl<T> BitCursor<T>
where
    T: BorrowBits,
{
    /// Splits the underlying slice at the cursor position and returns each half.
    pub fn split(&self) -> (&BitSlice, &BitSlice) {
        let bits = self.inner.borrow_bits();
        bits.split_at(self.pos as usize)
    }
}

impl<T> BitCursor<T>
where
    T: BorrowBitsMut,
{
    /// Splits the underlying slice at the cursor position and returns each half mutably
    /// TODO: should we be re-exporting BitSafeU8 in some other way?
    pub fn split_mut(
        &mut self,
    ) -> (
        &mut BitSlice<bitvec::access::BitSafeU8>,
        &mut BitSlice<bitvec::access::BitSafeU8>,
    ) {
        let bits = self.inner.borrow_bits_mut();
        let (left, right) = bits.split_at_mut(self.pos as usize);
        (left, right)
    }
}

impl<T> Clone for BitCursor<T>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        BitCursor {
            inner: self.inner.clone(),
            pos: self.pos,
        }
    }
}

impl<T> BitSeek for BitCursor<T>
where
    T: BorrowBits,
{
    fn bit_seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let (base_pos, offset) = match pos {
            SeekFrom::Start(n) => {
                self.pos = n;
                return Ok(n);
            }
            SeekFrom::End(n) => (self.inner.borrow_bits().len() as u64, n),
            SeekFrom::Current(n) => (self.pos, n),
        };
        match base_pos.checked_add_signed(offset) {
            Some(n) => {
                self.pos = n;
                Ok(self.pos)
            }
            None => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid seek to a negative or overlfowing position",
            )),
        }
    }
}

impl<T> Seek for BitCursor<T>
where
    T: BorrowBits,
{
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        match pos {
            SeekFrom::Start(n) => self.bit_seek(SeekFrom::Start(n * 8)),
            SeekFrom::End(n) => self.bit_seek(SeekFrom::End(n * 8)),
            SeekFrom::Current(n) => self.bit_seek(SeekFrom::Current(n * 8)),
        }
    }
}

impl<T> Read for BitCursor<T>
where
    T: BorrowBits,
{
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let bits = self.inner.borrow_bits();
        let remaining = &bits[self.pos as usize..];
        let mut bytes_read = 0;

        for (i, chunk) in remaining.chunks(8).take(buf.len()).enumerate() {
            let mut byte = 0u8;
            for (j, bit) in chunk.iter().enumerate() {
                if *bit {
                    byte |= 1 << (7 - j);
                }
            }
            buf[i] = byte;
            bytes_read += 1;
        }

        self.pos += (bytes_read * 8) as u64;
        Ok(bytes_read)
    }
}

impl<T> BitRead for BitCursor<T>
where
    T: BorrowBits,
{
    fn read_bits<O: BitStore>(&mut self, dest: &mut BitSlice<O>) -> std::io::Result<usize> {
        let n = BitRead::read_bits(&mut BitCursor::split(self).1, dest)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl<T> Write for BitCursor<T>
where
    T: BorrowBitsMut,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = Write::write(&mut BitCursor::split_mut(self).1, buf)?;
        self.pos += (n * 8) as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<T> BitWrite for BitCursor<T>
where
    T: BorrowBitsMut,
    BitCursor<T>: std::io::Write,
{
    fn write_bits<O: BitStore>(&mut self, source: &BitSlice<O>) -> std::io::Result<usize> {
        let n = BitWrite::write_bits(&mut BitCursor::split_mut(self).1, source)?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl<T> LowerHex for BitCursor<T>
where
    T: LowerHex,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "buf: {:x}, pos: {}", self.inner, self.pos)
    }
}

#[cfg(test)]
mod test {
    use std::fmt::Debug;
    use std::io::{Seek, SeekFrom};

    use crate::prelude::*;
    use bitvec::{order::Msb0, view::BitView};

    fn test_read_bits_hepler<T: BorrowBits>(buf: T, expected: &[u8]) {
        let expected_bits = expected.view_bits::<Msb0>();
        let mut cursor = BitCursor::new(buf);
        let mut read_buf = bitvec![0; expected_bits.len()];
        assert_eq!(
            cursor.read_bits(read_buf.as_mut_bitslice()).unwrap(),
            expected_bits.len()
        );
        assert_eq!(read_buf, expected_bits);
    }

    #[test]
    fn test_read_bits() {
        let data = [0b11110000, 0b00001111];

        let vec = Vec::from(data);
        test_read_bits_hepler(vec, &data);

        let bitvec = BitVec::from_slice(&data);
        test_read_bits_hepler(bitvec, &data);

        let bitslice: &BitSlice = data.view_bits();
        test_read_bits_hepler(bitslice, &data);

        let u8_slice = &data[..];
        test_read_bits_hepler(u8_slice, &data);
    }

    #[test]
    fn test_read_bytes() {
        let data = BitVec::from_vec(vec![1, 2, 3, 4]);
        let mut cursor = BitCursor::new(data);

        let mut buf = [0u8; 2];
        std::io::Read::read(&mut cursor, &mut buf).expect("valid read");
        assert_eq!(buf, [1, 2]);
        std::io::Read::read(&mut cursor, &mut buf).expect("valid read");
        assert_eq!(buf, [3, 4]);
    }

    #[test]
    fn test_bit_seek() {
        let data = BitVec::from_vec(vec![0b11001100, 0b00110011]);
        let mut cursor = BitCursor::new(data);

        let mut read_buf = bitvec![0; 4];

        cursor.bit_seek(SeekFrom::End(-2)).expect("valid seek");
        // Should now be reading the last 2 bits
        assert_eq!(cursor.read_bits(&mut read_buf).unwrap(), 2);
        assert_eq!(read_buf, bits![1, 1, 0, 0]);
        // We already read to the end
        assert_eq!(cursor.read_bits(&mut read_buf).unwrap(), 0);

        // The read after the seek brought the cursor back to the end.  Now jump back 6 bits.
        cursor.bit_seek(SeekFrom::Current(-6)).expect("valid seek");
        assert_eq!(cursor.read_bits(&mut read_buf).unwrap(), 4);
        assert_eq!(read_buf, bits![1, 1, 0, 0]);

        cursor.bit_seek(SeekFrom::Start(4)).expect("valid seek");
        assert_eq!(cursor.read_bits(&mut read_buf).unwrap(), 4);
        assert_eq!(read_buf, bits![1, 1, 0, 0]);
    }

    #[test]
    fn test_seek() {
        let data = BitVec::from_vec(vec![0b11001100, 0b00110011]);
        let mut cursor = BitCursor::new(data);

        let mut read_buf = bitvec![0; 2];
        cursor.seek(SeekFrom::End(-1)).unwrap();
        // Should now be reading the last byte
        assert_eq!(cursor.read_bits(&mut read_buf).unwrap(), 2);
        assert_eq!(read_buf, bits![0, 0]);
        // Go back one byte
        cursor.seek(SeekFrom::Current(-1)).unwrap();
        // We should now be in bit position 2
        assert_eq!(cursor.read_bits(&mut read_buf).unwrap(), 2);
        assert_eq!(read_buf, bits![0, 0]);
    }

    fn test_write_bits_helper<T>(buf: T)
    where
        T: BorrowBitsMut + Debug,
        BitCursor<T>: std::io::Write,
    {
        let mut cursor = BitCursor::new(buf);
        let data = bits![1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0];
        assert_eq!(cursor.write_bits(data).unwrap(), 16);
        assert_eq!(cursor.into_inner().borrow_bits(), data);
    }

    #[test]
    fn test_write_bits_bitvec() {
        let buf = BitVec::from_vec(vec![0; 2]);
        test_write_bits_helper(buf);
    }

    #[test]
    fn test_write_bits_vec() {
        let buf: Vec<u8> = vec![0, 0];
        test_write_bits_helper(buf);
    }

    #[test]
    fn test_write_bits_bit_slice() {
        let mut buf = bitvec![0; 16];
        test_write_bits_helper(buf.as_mut_bitslice());
    }

    #[test]
    fn test_write_bits_u8_slice() {
        let mut buf = [0u8; 2];
        test_write_bits_helper(&mut buf[..]);
    }

    fn test_split_helper<T: BorrowBits>(buf: T, expected: &[u8]) {
        let expected_bits = expected.view_bits::<Msb0>();
        let mut cursor = BitCursor::new(buf);
        cursor.bit_seek(SeekFrom::Current(4)).unwrap();
        let (before, after) = cursor.split();

        assert_eq!(before, expected_bits[..4]);
        assert_eq!(after, expected_bits[4..]);
    }

    #[test]
    fn test_split() {
        let data = [0b11110011, 0b10101010];

        let vec = Vec::from(data);
        test_split_helper(vec, &data);

        let bitvec = BitVec::from_slice(&data);
        test_split_helper(bitvec, &data);

        let bitslice: &BitSlice = data.view_bits();
        test_split_helper(bitslice, &data);

        let u8_slice = &data[..];
        test_split_helper(u8_slice, &data);
    }

    // Maybe a bit paranoid, but this creates cursors using different inner types, splits the data,
    // then makes sure that cursors can be created from each split and the data read correctly
    #[test]
    fn test_cursors_from_splits() {
        let data = [0b11110011, 0b10101010];

        let vec = Vec::from(data);
        let mut vec_cursor = BitCursor::new(vec);
        vec_cursor.seek(SeekFrom::Start(1)).unwrap();
        let (left, right) = vec_cursor.split();
        test_read_bits_hepler(left, &data[..1]);
        test_read_bits_hepler(right, &data[1..]);

        let bitvec = BitVec::from_slice(&data);
        let mut bitvec_cursor = BitCursor::new(bitvec);
        bitvec_cursor.seek(SeekFrom::Start(1)).unwrap();
        let (left, right) = bitvec_cursor.split();
        test_read_bits_hepler(left, &data[..1]);
        test_read_bits_hepler(right, &data[1..]);

        let bitslice: &BitSlice = data.view_bits();
        let mut bitslice_cursor = BitCursor::new(bitslice);
        bitslice_cursor.seek(SeekFrom::Start(1)).unwrap();
        let (left, right) = bitslice_cursor.split();
        test_read_bits_hepler(left, &data[..1]);
        test_read_bits_hepler(right, &data[1..]);

        let u8_slice = &data[..];
        let mut u8_cursor = BitCursor::new(u8_slice);
        u8_cursor.seek(SeekFrom::Start(1)).unwrap();
        let (left, right) = u8_cursor.split();
        test_read_bits_hepler(left, &data[..1]);
        test_read_bits_hepler(right, &data[1..]);
    }

    // Assumes the given buf is 4 bytes long
    fn test_split_mut_helper<T>(buf: T)
    where
        T: BorrowBitsMut + Debug,
        BitCursor<T>: std::io::Write,
    {
        let mut cursor = BitCursor::new(buf);
        cursor.seek(SeekFrom::Start(2)).unwrap();

        {
            let (mut left, mut right) = cursor.split_mut();

            left.write_bits(bits![1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0])
                .unwrap();
            right
                .write_bits(bits![0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1])
                .unwrap();
        }

        let data = cursor.into_inner();
        assert_eq!(
            data.borrow_bits(),
            bits![
                1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0,
                1, 1, 1, 1
            ]
        );
    }

    #[test]
    fn test_split_mut() {
        let data = [0u8; 4];

        let vec = Vec::from(data);
        test_split_mut_helper(vec);

        let bitvec = BitVec::from_vec(vec![0u8; 4]);
        test_split_mut_helper(bitvec);

        let mut data = [0u8; 4];
        let bitslice: &mut BitSlice = data.view_bits_mut();
        test_split_mut_helper(bitslice);

        let mut data = [0u8; 4];
        let u8_slice = &mut data[..];
        test_split_mut_helper(u8_slice);
    }

    #[test]
    fn test_alignment_reads_writes() {
        for offset in 0..8 {
            let buf = vec![0u8; 4];
            let mut cursor = BitCursor::new(buf);
            cursor.set_position(offset);
            let value = BitVec::from_slice(&[0xDE, 0xAD]);

            cursor.write_bits(value.as_bitslice()).unwrap();

            cursor.set_position(offset);
            let mut read_buf = BitVec::with_capacity(16);
            read_buf.resize(16, false);
            cursor.read_bits(read_buf.as_mut_bitslice()).unwrap();
            assert_eq!(value, read_buf, "offset {offset}");
        }
    }
}
