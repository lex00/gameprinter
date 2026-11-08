//! Text pixel art (`.pix`), so a theme's small restyled assets diff and
//! review in git like the rest of the repo. A file is:
//!
//! ```text
//! pix 1
//! size 16 16
//! . transparent
//! a #2b1d0e
//! b #f2e6c8
//! ................
//! ...aaaa.........
//! ```
//!
//! after the header, one line per color (a single character other than a
//! space, then `transparent` or `#rrggbb`), then `height` rows of `width`
//! characters. Lines starting with `#` are comments.

use crate::color::Rgb;

/// An RGBA picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[u8; 4]>,
}

impl Rgba {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![[0; 4]; (width * height) as usize],
        }
    }

    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        if x < self.width && y < self.height {
            self.pixels[(y * self.width + x) as usize]
        } else {
            [0; 4]
        }
    }

    pub fn set(&mut self, x: u32, y: u32, c: [u8; 4]) {
        if x < self.width && y < self.height {
            self.pixels[(y * self.width + x) as usize] = c;
        }
    }

    /// The picture as a PNG.
    pub fn png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width, self.height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().expect("PNG header into memory");
            w.write_image_data(self.pixels.as_flattened())
                .expect("PNG data into memory");
            w.finish().expect("PNG end into memory");
        }
        out
    }

    /// Reads an 8-bit RGBA or RGB PNG.
    pub fn from_png(bytes: &[u8]) -> Result<Self, String> {
        let dec = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut r = dec.read_info().map_err(|e| e.to_string())?;
        let mut buf = vec![0; r.output_buffer_size().ok_or("PNG too large")?];
        let info = r.next_frame(&mut buf).map_err(|e| e.to_string())?;
        if info.bit_depth != png::BitDepth::Eight {
            return Err("a theme's PNG is 8 bits a channel".into());
        }
        let px: Vec<[u8; 4]> = match info.color_type {
            png::ColorType::Rgba => buf[..info.buffer_size()].as_chunks::<4>().0.to_vec(),
            png::ColorType::Rgb => buf[..info.buffer_size()]
                .as_chunks::<3>()
                .0
                .iter()
                .map(|c| [c[0], c[1], c[2], 255])
                .collect(),
            other => return Err(format!("a theme's PNG is RGB or RGBA, not {other:?}")),
        };
        Ok(Self {
            width: info.width,
            height: info.height,
            pixels: px,
        })
    }

    /// Reads a `.pix` or `.png` file by its extension.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let r = if path.extension().is_some_and(|e| e == "pix") {
            std::str::from_utf8(&bytes)
                .map_err(|e| e.to_string())
                .and_then(parse)
        } else {
            Self::from_png(&bytes)
        };
        r.map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// Parses `.pix` text.
pub fn parse(text: &str) -> Result<Rgba, String> {
    let mut lines = text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.starts_with('#') && !l.trim().is_empty());
    let (_, head) = lines.next().ok_or("empty pix file")?;
    if head.trim() != "pix 1" {
        return Err("a pix file starts with `pix 1`".into());
    }
    let (n, size) = lines.next().ok_or("no size line")?;
    let dims: Vec<u32> = size
        .strip_prefix("size ")
        .ok_or(format!("line {}: `size W H`", n + 1))?
        .split_whitespace()
        .map(|w| w.parse().map_err(|_| format!("line {}: `{w}`", n + 1)))
        .collect::<Result<_, _>>()?;
    let [w, h] = dims[..] else {
        return Err(format!("line {}: `size W H`", n + 1));
    };
    if w == 0 || h == 0 || w > 512 || h > 512 {
        return Err(format!("size {w} {h} isn't 1 to 512 each way"));
    }
    let mut keys: Vec<(char, [u8; 4])> = Vec::new();
    let mut img = Rgba::new(w, h);
    let mut y = 0;
    for (n, l) in lines {
        let mut chars = l.chars();
        let first = chars.next().expect("not empty");
        let rest = chars.as_str();
        // Rows have no spaces (no key is a space), so a line with one is a color.
        if y == 0 && rest.starts_with(' ') {
            let v = rest.trim();
            let c = if v == "transparent" {
                [0, 0, 0, 0]
            } else {
                let Rgb([r, g, b]) = Rgb::parse(v).map_err(|e| format!("line {}: {e}", n + 1))?;
                [r, g, b, 255]
            };
            if keys.iter().any(|(k, _)| *k == first) {
                return Err(format!("line {}: `{first}` has two colors", n + 1));
            }
            keys.push((first, c));
            continue;
        }
        if y >= h {
            return Err(format!("line {}: more than {h} rows", n + 1));
        }
        if l.chars().count() != w as usize {
            return Err(format!("line {}: a row is {w} characters", n + 1));
        }
        for (x, ch) in l.chars().enumerate() {
            let c = keys
                .iter()
                .find(|(k, _)| *k == ch)
                .ok_or(format!("line {}: `{ch}` has no color", n + 1))?
                .1;
            img.set(x as u32, y, c);
        }
        y += 1;
    }
    if y != h {
        return Err(format!("{y} rows, not {h}"));
    }
    Ok(img)
}

/// Writes a picture as `.pix`: colors by first use, `.` transparent.
pub fn write(img: &Rgba, comment: &str) -> Result<String, String> {
    const KEYS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut colors: Vec<[u8; 4]> = Vec::new();
    for p in &img.pixels {
        let p = if p[3] == 0 { [0; 4] } else { *p };
        if p[3] != 0 && p[3] != 255 {
            return Err("pix holds opaque or transparent pixels only".into());
        }
        if p[3] == 255 && !colors.contains(&p) {
            colors.push(p);
        }
    }
    if colors.len() > KEYS.len() {
        return Err(format!("{} colors; pix holds {}", colors.len(), KEYS.len()));
    }
    let key = |p: [u8; 4]| -> char {
        if p[3] == 0 {
            '.'
        } else {
            let i = colors.iter().position(|c| *c == p).expect("listed");
            KEYS.as_bytes()[i] as char
        }
    };
    let mut out = String::from("pix 1\n");
    for l in comment.lines() {
        out.push_str(&format!("# {l}\n"));
    }
    out.push_str(&format!(
        "size {} {}\n. transparent\n",
        img.width, img.height
    ));
    for (i, c) in colors.iter().enumerate() {
        out.push_str(&format!(
            "{} {}\n",
            KEYS.as_bytes()[i] as char,
            Rgb([c[0], c[1], c[2]])
        ));
    }
    for y in 0..img.height {
        let row: String = (0..img.width).map(|x| key(img.get(x, y))).collect();
        out.push_str(&row);
        out.push('\n');
    }
    Ok(out)
}

/// A `.pix` file as color indices, for a tiles slot's restyle: `.` (or any
/// transparent color) is index 0, and the other colors are 1, 2 and 3 in
/// the order their lines come. Returns the width, the height and a byte
/// a pixel.
pub fn indexed(text: &str) -> Result<(u32, u32, Vec<u8>), String> {
    let mut lines = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty());
    if lines.next().map(str::trim) != Some("pix 1") {
        return Err("a .pix file starts with `pix 1`".into());
    }
    let size = lines.next().ok_or("no size line")?;
    let f: Vec<&str> = size.split_whitespace().collect();
    let (w, h) = match f.as_slice() {
        ["size", w, h] => (
            w.parse::<u32>().map_err(|_| "a bad width")?,
            h.parse::<u32>().map_err(|_| "a bad height")?,
        ),
        _ => return Err("the second line is `size W H`".into()),
    };
    let mut index = std::collections::BTreeMap::new();
    let mut next = 1u8;
    let mut rows = Vec::new();
    for l in lines {
        let mut cs = l.chars();
        let c = cs.next().unwrap_or(' ');
        let rest = cs.as_str();
        if rows.is_empty() && rest.starts_with(' ') {
            let v = rest.trim();
            if v == "transparent" {
                index.insert(c, 0);
            } else {
                if next > 3 {
                    return Err(
                        "a tiles slot's .pix has at most three colors and transparent".into(),
                    );
                }
                index.insert(c, next);
                next += 1;
            }
            continue;
        }
        rows.push(l);
    }
    if rows.len() != h as usize {
        return Err(format!("{} rows, not {h}", rows.len()));
    }
    let mut px = Vec::with_capacity((w * h) as usize);
    for (y, r) in rows.iter().enumerate() {
        if r.chars().count() != w as usize {
            return Err(format!("row {y} isn't {w} wide"));
        }
        for c in r.chars() {
            px.push(
                *index
                    .get(&c)
                    .ok_or(format!("`{c}` isn't a color of the file"))?,
            );
        }
    }
    Ok((w, h, px))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let mut img = Rgba::new(3, 2);
        img.set(0, 0, [255, 0, 0, 255]);
        img.set(2, 1, [0, 0, 255, 255]);
        let text = write(&img, "made up").unwrap();
        assert!(text.starts_with("pix 1\n# made up\nsize 3 2\n"));
        assert_eq!(parse(&text).unwrap(), img);
    }

    #[test]
    fn refuses_bad_rows() {
        assert!(parse("pix 1\nsize 2 1\n. transparent\n...\n").is_err());
        assert!(parse("pix 1\nsize 2 1\n. transparent\nxx\n").is_err());
        assert!(parse("pix 2\nsize 1 1\n").is_err());
    }
}
