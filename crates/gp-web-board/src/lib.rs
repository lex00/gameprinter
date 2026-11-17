//! The page's board drawing for an arcade model (docs/web.md, "Arcade
//! games"): [`gp_look::board::Companion`] behind a handful of plain exports
//! over linear memory ([`abi`], wasm32 only), and the packing the page
//! hands the lifted files over in.

use std::collections::BTreeMap;

use gp_look::board::{Companion, HEIGHT, Picture, WIDTH};

/// Files packed as the page hands them over: each name and its bytes, each
/// preceded by its length as a little-endian u32.
pub fn pack_files<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in files {
        for part in [name.as_bytes(), data] {
            out.extend((part.len() as u32).to_le_bytes());
            out.extend(part);
        }
    }
    out
}

/// [`pack_files`]' bytes read back, by name.
pub fn unpack_files(bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut out = BTreeMap::new();
    let mut at = 0;
    let next = |at: &mut usize| -> Result<Vec<u8>, String> {
        let n = bytes
            .get(*at..*at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
            .ok_or("the packed files end inside a length")?;
        let v = bytes
            .get(*at + 4..*at + 4 + n)
            .ok_or("the packed files end inside a file")?
            .to_vec();
        *at += 4 + n;
        Ok(v)
    };
    while at < bytes.len() {
        let name = String::from_utf8(next(&mut at)?).map_err(|e| e.to_string())?;
        let data = next(&mut at)?;
        out.insert(name, data);
    }
    Ok(out)
}

/// Opens the board art from the bundle, the presentation text and the
/// packed lifted files: None for a model that names none.
pub fn open(bundle: &[u8], present: &[u8], packed: &[u8]) -> Result<Option<Companion>, String> {
    let files = unpack_files(packed)?;
    let present = std::str::from_utf8(present).map_err(|e| format!("present.txt: {e}"))?;
    Companion::open(bundle, present, &|n: &str| {
        files
            .get(n)
            .cloned()
            .ok_or_else(|| format!("{n}: the page didn't hand over this lifted file"))
    })
}

/// A frame's picture as the page reads it: its width and height as
/// little-endian u16s, each pixel's pen (0 to 63) row by row, then the 64
/// colours as R, G, B, 255.
pub fn picture_bytes(pic: &Picture) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + pic.pens.len() + 256);
    for n in [WIDTH, HEIGHT] {
        out.extend((n as u16).to_le_bytes());
    }
    out.extend(pic.pens.iter().map(|p| p & 0x3f));
    for c in pic.palette {
        out.extend([(c >> 16) as u8, (c >> 8) as u8, c as u8, 255]);
    }
    out
}

/// [`picture_bytes`] of a themed frame's picture: the same, but a pen the
/// board leaves for the theme ([`gp_look::board::TRANSPARENT`]) is 64,
/// which the page paints as nothing.
pub fn themed_picture_bytes(pic: &Picture) -> Vec<u8> {
    let mut out = picture_bytes(pic);
    for (o, p) in out[4..4 + pic.pens.len()].iter_mut().zip(&pic.pens) {
        if *p == gp_look::board::TRANSPARENT {
            *o = 64;
        }
    }
    out
}

/// A themed frame as the page reads it: [`themed_picture_bytes`]'s length
/// as a little-endian u32, those bytes, then the theme's draws as JSON in
/// the game module's `gp_art_frame` shape (`size`, `scale`, `clear`,
/// `draws`, `backdrop`), with the board's picture as the draw
/// `[["board"], 0, 0, W, H, false, false, 255, [], null]` where it goes.
pub fn themed_bytes(t: &gp_look::board::Themed) -> Vec<u8> {
    let pic = themed_picture_bytes(&t.picture);
    let mut out = Vec::with_capacity(4 + pic.len() + 256);
    out.extend((pic.len() as u32).to_le_bytes());
    out.extend(pic);
    out.extend(art_json(t).into_bytes());
    out
}

fn ints(v: &[i64]) -> String {
    let s: Vec<String> = v.iter().map(i64::to_string).collect();
    format!("[{}]", s.join(","))
}

fn art_json(t: &gp_look::board::Themed) -> String {
    use gp_look::board::{HEIGHT, WIDTH};
    use gp_present::art::{ART_SUB, Src};
    let f = &t.frame;
    let rgb = |c: [u8; 3]| format!("[{},{},{}]", c[0], c[1], c[2]);
    let mut draws: Vec<String> = Vec::with_capacity(f.draws.len() + 1);
    let board = format!(
        "[[\"board\"],0,0,{},{},false,false,255,[],null]",
        WIDTH as i64 * ART_SUB,
        HEIGHT as i64 * ART_SUB
    );
    for (i, d) in f.draws.iter().enumerate() {
        if i == t.board_at {
            draws.push(board.clone());
        }
        let src = match d.src {
            Src::Own(i) => format!("[\"own\",{i}]"),
            Src::Art(i) => format!("[\"art\",{i}]"),
            Src::Tint(i, c) => format!("[\"tint\",{i},{}]", rgb(c)),
            Src::Fill(c) => format!("[\"fill\",{}]", rgb(c)),
            Src::Stage(l) => format!("[\"stage\",{l}]"),
            Src::Backdrop => "[\"backdrop\"]".to_owned(),
        };
        let cut: Vec<String> = d.cut.iter().map(|c| ints(c)).collect();
        let clip = d.clip.map_or("null".to_owned(), |c| ints(&c));
        draws.push(format!(
            "[{src},{},{},{},{},{},{},{},[{}],{clip}]",
            d.x,
            d.y,
            d.w,
            d.h,
            d.flip_x,
            d.flip_y,
            d.alpha,
            cut.join(",")
        ));
    }
    if t.board_at >= f.draws.len() {
        draws.push(board);
    }
    format!(
        "{{\"size\":[{},{}],\"scale\":{},\"clear\":{},\"draws\":[{}],\"backdrop\":null}}",
        f.size[0],
        f.size[1],
        f.scale,
        rgb(f.clear),
        draws.join(",")
    )
}

/// The files a theme's rules name, as JSON in `gp_load_art`'s shape
/// (`{"files": [...], "stages": []}`: the board builds no stages over
/// pictures the page hands it), and the rooms it built (`#+ room`) over
/// the board's screens from the player's lift: `"rooms": [[pic, stage],
/// ...]`, each stage as the game module's `gp_art_stage` gives one, which
/// the page paints once and draws where the frame's draws hold
/// `["stage", pic]`.
pub fn files_json(theme: &gp_look::board::BoardTheme) -> String {
    use gp_present::art::stage::StageSrc;
    let files: Vec<String> = theme
        .art
        .files
        .iter()
        .map(|f| format!("\"{}\"", f.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    let rgb = |c: [u8; 3]| format!("[{},{},{}]", c[0], c[1], c[2]);
    let mut rooms = Vec::new();
    for r in &theme.rooms {
        let Some(st) = theme.art.stage(r.pic) else {
            continue;
        };
        let draws: Vec<String> = st
            .draws
            .iter()
            .map(|d| {
                let src = match d.src {
                    StageSrc::Piece(i) => format!("[\"piece\",{i}]"),
                    StageSrc::Backdrop => "[\"backdrop\"]".to_owned(),
                };
                let clip = d.clip.map_or("null".to_owned(), |c| ints(&c));
                format!(
                    "[{src},{},{},{},{},{},{},{},{clip}]",
                    ints(&d.part),
                    d.x,
                    d.y,
                    d.w,
                    d.h,
                    d.flip_x,
                    d.shade
                )
            })
            .collect();
        rooms.push(format!(
            "[{},{{\"size\":{},\"scale\":{},\"backdrop\":[{},{}],\"draws\":[{}]}}]",
            r.pic,
            ints(&st.size),
            st.scale,
            rgb(st.backdrop[0]),
            rgb(st.backdrop[1]),
            draws.join(",")
        ));
    }
    format!(
        "{{\"files\":[{}],\"stages\":[],\"rooms\":[{}]}}",
        files.join(","),
        rooms.join(",")
    )
}

#[cfg(target_arch = "wasm32")]
pub mod abi;

#[cfg(test)]
mod tests {
    use super::*;
    use gp_look::board::Themed;

    /// Files packed for the page come back as they went, and a packing cut
    /// short is an error, not files.
    #[test]
    fn packed_files_come_back_whole() {
        let files = [
            ("assets.toml", &b"schema = 1\n"[..]),
            ("tiles.png", &[0u8, 1, 2][..]),
        ];
        let packed = pack_files(files);
        let back = unpack_files(&packed).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back["tiles.png"], [0, 1, 2]);
        assert_eq!(back["assets.toml"], b"schema = 1\n");
        assert!(unpack_files(&packed[..packed.len() - 1]).is_err());
        assert!(unpack_files(&packed[..2]).is_err());
    }

    /// The page's picture: size first, pens without the sprite bit, then
    /// the colours.
    #[test]
    fn a_picture_is_its_size_its_pens_and_its_colours() {
        let mut pens = vec![0u8; WIDTH * HEIGHT];
        pens[1] = 5 | 0x80;
        let mut palette = [0u32; 64];
        palette[5] = 0x123456;
        let b = picture_bytes(&Picture {
            pens,
            palette,
            drawn: Vec::new(),
            screen: None,
            cells: Vec::new(),
        });
        assert_eq!(u16::from_le_bytes([b[0], b[1]]) as usize, WIDTH);
        assert_eq!(u16::from_le_bytes([b[2], b[3]]) as usize, HEIGHT);
        assert_eq!(b[4 + 1], 5);
        let pal = &b[4 + WIDTH * HEIGHT..];
        assert_eq!(&pal[20..24], &[0x12, 0x34, 0x56, 255]);
        assert_eq!(b.len(), 4 + WIDTH * HEIGHT + 256);
    }

    /// A themed frame's picture is the plain one's bytes but for the pens
    /// left to the theme (64), and its draws put the board's picture where
    /// the theme says.
    #[test]
    fn a_themed_frame_is_the_boards_picture_and_the_themes_draws() {
        let mut pens = vec![3u8 | 0x80; WIDTH * HEIGHT];
        let pic = |pens: Vec<u8>| Picture {
            pens,
            palette: [0x102030; 64],
            drawn: Vec::new(),
            screen: Some("stage".into()),
            cells: Vec::new(),
        };
        let mut d = gp_present::art::ArtFrame {
            size: [512, 480],
            scale: 2,
            ..Default::default()
        };
        let plain = Themed {
            picture: pic(pens.clone()),
            frame: d.clone(),
            board_at: 0,
        };
        assert_eq!(
            themed_picture_bytes(&plain.picture),
            picture_bytes(&plain.picture)
        );
        let b = themed_bytes(&plain);
        let n = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
        assert_eq!(&b[4..4 + n], &picture_bytes(&plain.picture)[..]);
        let json = std::str::from_utf8(&b[4 + n..]).unwrap();
        assert_eq!(
            json,
            r#"{"size":[512,480],"scale":2,"clear":[0,0,0],"draws":[[["board"],0,0,8192,7680,false,false,255,[],null]],"backdrop":null}"#
        );
        pens[0] = gp_look::board::TRANSPARENT;
        let mut s = gp_present::art::ArtFrame::default().draws;
        s.push(gp_present::art::ArtDraw {
            src: gp_present::art::Src::Art(0),
            x: 0,
            y: 0,
            w: 16,
            h: 16,
            flip_x: true,
            flip_y: false,
            alpha: 255,
            cut: Vec::new(),
            clip: Some([0, 0, 8, 8]),
            of: gp_present::art::Of::Level,
        });
        d.draws = s;
        let t = Themed {
            picture: pic(pens),
            frame: d,
            board_at: 1,
        };
        let b = themed_bytes(&t);
        assert_eq!(b[4 + 4], 64);
        assert_eq!(b[4 + 5], 3);
        let n = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
        let json = std::str::from_utf8(&b[4 + n..]).unwrap();
        assert!(
            json.contains(
                r#""draws":[[["art",0],0,0,16,16,true,false,255,[],[0,0,8,8]],[["board"],"#
            ),
            "{json}"
        );
    }
}
