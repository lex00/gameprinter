//! A plain binary form for snapshots: little-endian numbers and
//! length-prefixed byte runs, written and read in the same order.

use crate::attotime::Attotime;

#[derive(Default)]
pub struct Writer {
    pub out: Vec<u8>,
}

impl Writer {
    pub fn u8(&mut self, v: u8) {
        self.out.push(v);
    }
    pub fn bool(&mut self, v: bool) {
        self.out.push(u8::from(v));
    }
    pub fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i32(&mut self, v: i32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    pub fn i64(&mut self, v: i64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    pub fn time(&mut self, t: Attotime) {
        self.i32(t.seconds);
        self.i64(t.attoseconds);
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.out.extend_from_slice(b);
    }
}

pub struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

pub type Result<T> = std::result::Result<T, String>;

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self
            .data
            .get(self.at..self.at + n)
            .ok_or_else(|| format!("snapshot ends early, at byte {}", self.at))?;
        self.at += n;
        Ok(s)
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }
    pub fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }
    pub fn time(&mut self) -> Result<Attotime> {
        Ok(Attotime::new(self.i32()?, self.i64()?))
    }
    pub fn bytes(&mut self) -> Result<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    /// A byte run that must be exactly `out.len()` long.
    pub fn fill(&mut self, out: &mut [u8]) -> Result<()> {
        let b = self.bytes()?;
        if b.len() != out.len() {
            return Err(format!(
                "snapshot holds {} bytes where {} belong",
                b.len(),
                out.len()
            ));
        }
        out.copy_from_slice(b);
        Ok(())
    }
    pub fn done(&self) -> Result<()> {
        if self.at == self.data.len() {
            Ok(())
        } else {
            Err(format!(
                "{} bytes left over in the snapshot",
                self.data.len() - self.at
            ))
        }
    }
}
