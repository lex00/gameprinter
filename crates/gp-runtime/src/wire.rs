//! The bytes that carry [`Runtime`] calls between `gp` and a game's module.
//!
//! A call is an op number and one byte string each way, so the module's
//! exports stay fixed as calls are added. Integers are little-endian,
//! strings and lists start with a u32 count.

use crate::{Entry, Event, InputFrame, LoadHow, LoadNote, Rect, Until, Value};

pub const LOAD: u32 = 1;
pub const INFO: u32 = 2;
pub const FORK: u32 = 3;
pub const DROP: u32 = 4;
pub const STEP: u32 = 5;
pub const RUN_UNTIL: u32 = 6;
pub const FRAME: u32 = 7;
pub const CANONICAL_STATE: u32 = 8;
pub const STATE_HASH: u32 = 9;
pub const LOAD_STATE: u32 = 10;
pub const QUERY: u32 = 11;
pub const SAMPLE: u32 = 12;
pub const ENTITIES_AT: u32 = 13;
pub const SET: u32 = 14;
pub const EXPLAIN: u32 = 15;
pub const SET_DRAWS: u32 = 16;
pub const REPORT_DRAWS: u32 = 17;
pub const SET_APU_EVENTS: u32 = 18;
pub const SPEC_DIGEST: u32 = 19;
pub const WORLD_HASH: u32 = 20;
pub const START_READS: u32 = 21;
pub const TAKE_READS: u32 = 22;
pub const RESTORE: u32 = 23;

#[derive(Default)]
pub struct Writer(pub Vec<u8>);

impl Writer {
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    pub fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }
    pub fn u32(&mut self, v: u32) {
        self.0.extend(v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.0.extend(v.to_le_bytes());
    }
    pub fn i64(&mut self, v: i64) {
        self.0.extend(v.to_le_bytes());
    }
    pub fn count(&mut self, n: usize) {
        self.u32(u32::try_from(n).expect("a list under 4G"));
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.count(b.len());
        self.0.extend(b);
    }
    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
    pub fn strs(&mut self, list: &[String]) {
        self.count(list.len());
        for s in list {
            self.str(s);
        }
    }
    pub fn opt_u64(&mut self, v: Option<u64>) {
        self.bool(v.is_some());
        if let Some(x) = v {
            self.u64(x);
        }
    }
    pub fn value(&mut self, v: &Value) {
        match v {
            Value::Int(x) => {
                self.u8(0);
                self.i64(*x);
            }
            Value::Fixed(x) => {
                self.u8(1);
                self.i64(*x);
            }
            Value::Flag(x) => {
                self.u8(2);
                self.bool(*x);
            }
            Value::Text(x) => {
                self.u8(3);
                self.str(x);
            }
            Value::Entity(x) => {
                self.u8(4);
                self.u64(*x);
            }
            Value::Vec2(x, y) => {
                self.u8(5);
                self.i64(*x);
                self.i64(*y);
            }
        }
    }
    pub fn entry(&mut self, e: &Entry) {
        self.str(&e.path);
        self.value(&e.value);
    }
    pub fn entries(&mut self, list: &[Entry]) {
        self.count(list.len());
        for e in list {
            self.entry(e);
        }
    }
    pub fn event(&mut self, e: &Event) {
        self.u64(e.frame);
        self.str(&e.kind);
        self.opt_u64(e.entity);
        self.entries(&e.data);
    }
    pub fn events(&mut self, list: &[Event]) {
        self.count(list.len());
        for e in list {
            self.event(e);
        }
    }
    pub fn input(&mut self, i: &InputFrame) {
        self.u32(i.buttons);
        self.bool(i.reset);
        self.count(i.axes.len());
        for a in &i.axes {
            self.0.extend(a.to_le_bytes());
        }
    }
    pub fn inputs(&mut self, list: &[InputFrame]) {
        self.count(list.len());
        for i in list {
            self.input(i);
        }
    }
    pub fn rect(&mut self, r: Rect) {
        self.i64(r.x);
        self.i64(r.y);
        self.u64(r.w);
        self.u64(r.h);
    }
    pub fn until(&mut self, u: &Until) {
        self.bool(u.met);
        self.events(&u.events);
    }
    pub fn notes(&mut self, list: &[LoadNote]) {
        self.count(list.len());
        for n in list {
            self.str(&n.path);
            self.u8(match n.how {
                LoadHow::Loaded => 0,
                LoadHow::Approximated => 1,
                LoadHow::Missing => 2,
            });
            self.str(&n.why);
        }
    }
}

pub struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

type R<T> = Result<T, String>;

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Self { b, at: 0 }
    }
    fn take(&mut self, n: usize) -> R<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.b.len());
        let end = end.ok_or("a runtime call's bytes ended early")?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    pub fn u8(&mut self) -> R<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn bool(&mut self) -> R<bool> {
        Ok(self.u8()? != 0)
    }
    pub fn u32(&mut self) -> R<u32> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("4 bytes"),
        ))
    }
    pub fn u64(&mut self) -> R<u64> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }
    pub fn i64(&mut self) -> R<i64> {
        Ok(i64::from_le_bytes(
            self.take(8)?.try_into().expect("8 bytes"),
        ))
    }
    pub fn count(&mut self) -> R<usize> {
        Ok(self.u32()? as usize)
    }
    pub fn bytes(&mut self) -> R<&'a [u8]> {
        let n = self.count()?;
        self.take(n)
    }
    pub fn str(&mut self) -> R<String> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|e| e.to_string())
    }
    pub fn strs(&mut self) -> R<Vec<String>> {
        (0..self.count()?).map(|_| self.str()).collect()
    }
    pub fn opt_u64(&mut self) -> R<Option<u64>> {
        Ok(if self.bool()? {
            Some(self.u64()?)
        } else {
            None
        })
    }
    pub fn value(&mut self) -> R<Value> {
        Ok(match self.u8()? {
            0 => Value::Int(self.i64()?),
            1 => Value::Fixed(self.i64()?),
            2 => Value::Flag(self.bool()?),
            3 => Value::Text(self.str()?),
            4 => Value::Entity(self.u64()?),
            5 => Value::Vec2(self.i64()?, self.i64()?),
            k => return Err(format!("a value of kind {k}")),
        })
    }
    pub fn entry(&mut self) -> R<Entry> {
        Ok(Entry {
            path: self.str()?,
            value: self.value()?,
        })
    }
    pub fn entries(&mut self) -> R<Vec<Entry>> {
        (0..self.count()?).map(|_| self.entry()).collect()
    }
    pub fn event(&mut self) -> R<Event> {
        Ok(Event {
            frame: self.u64()?,
            kind: self.str()?,
            entity: self.opt_u64()?,
            data: self.entries()?,
        })
    }
    pub fn events(&mut self) -> R<Vec<Event>> {
        (0..self.count()?).map(|_| self.event()).collect()
    }
    pub fn input(&mut self) -> R<InputFrame> {
        let buttons = self.u32()?;
        let reset = self.bool()?;
        let n = self.count()?;
        let axes = self
            .take(n * 2)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes(*c))
            .collect();
        Ok(InputFrame {
            buttons,
            axes,
            reset,
        })
    }
    pub fn inputs(&mut self) -> R<Vec<InputFrame>> {
        (0..self.count()?).map(|_| self.input()).collect()
    }
    pub fn rect(&mut self) -> R<Rect> {
        Ok(Rect {
            x: self.i64()?,
            y: self.i64()?,
            w: self.u64()?,
            h: self.u64()?,
        })
    }
    pub fn until(&mut self) -> R<Until> {
        Ok(Until {
            met: self.bool()?,
            events: self.events()?,
        })
    }
    pub fn notes(&mut self) -> R<Vec<LoadNote>> {
        (0..self.count()?)
            .map(|_| {
                Ok(LoadNote {
                    path: self.str()?,
                    how: match self.u8()? {
                        0 => LoadHow::Loaded,
                        1 => LoadHow::Approximated,
                        2 => LoadHow::Missing,
                        k => return Err(format!("a load note of kind {k}")),
                    },
                    why: self.str()?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_written_reads_back() {
        let event = Event {
            frame: 9,
            kind: "k".into(),
            entity: Some(3),
            data: vec![
                Entry {
                    path: "a".into(),
                    value: Value::Fixed(-5),
                },
                Entry {
                    path: "b".into(),
                    value: Value::Vec2(1, -2),
                },
                Entry {
                    path: "c".into(),
                    value: Value::Text("t".into()),
                },
                Entry {
                    path: "d".into(),
                    value: Value::Flag(true),
                },
                Entry {
                    path: "e".into(),
                    value: Value::Entity(7),
                },
                Entry {
                    path: "f".into(),
                    value: Value::Int(i64::MIN),
                },
            ],
        };
        let input = InputFrame {
            buttons: 0x0281,
            axes: vec![-1, 2],
            reset: true,
        };
        let note = LoadNote {
            path: "p".into(),
            how: LoadHow::Approximated,
            why: "w".into(),
        };
        let mut w = Writer::default();
        w.events(std::slice::from_ref(&event));
        w.inputs(std::slice::from_ref(&input));
        w.notes(std::slice::from_ref(&note));
        w.until(&Until {
            met: true,
            events: vec![],
        });
        let mut r = Reader::new(&w.0);
        assert_eq!(r.events().unwrap(), vec![event]);
        assert_eq!(r.inputs().unwrap(), vec![input]);
        assert_eq!(r.notes().unwrap(), vec![note]);
        assert!(r.until().unwrap().met);
    }

    #[test]
    fn a_short_call_is_an_error() {
        let mut w = Writer::default();
        w.str("hello");
        assert!(Reader::new(&w.0[..3]).str().is_err());
    }
}
