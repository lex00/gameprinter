//! Palette files: GIMP `.gpl`, JASC-PAL `.pal`, `.hex` and Pixelorama's
//! JSON palette, written and read. Each holds the colors in order; the
//! reader gives them back in that order.

/// `#rrggbb` or `rrggbb` as a color.
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 || !h.is_ascii() {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([b(0)?, b(2)?, b(4)?])
}

pub fn hex(c: [u8; 3]) -> String {
    format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

pub fn write(kind: crate::config::PaletteFile, name: &str, colors: &[[u8; 3]]) -> String {
    use crate::config::PaletteFile as P;
    match kind {
        P::None => String::new(),
        P::Gpl => {
            let mut s = format!("GIMP Palette\nName: {name}\nColumns: 4\n#\n");
            for (i, c) in colors.iter().enumerate() {
                s.push_str(&format!("{:3} {:3} {:3}\tIndex {i}\n", c[0], c[1], c[2]));
            }
            s
        }
        P::Jasc => {
            let mut s = format!("JASC-PAL\n0100\n{}\n", colors.len());
            for c in colors {
                s.push_str(&format!("{} {} {}\n", c[0], c[1], c[2]));
            }
            s
        }
        P::Hex => colors.iter().map(|c| hex(*c) + "\n").collect(),
        P::Pixelorama => {
            // Pixelorama's palette resource: a JSON object with the colors
            // as "(r, g, b, a)" strings of 0 to 1, by index, on a grid of
            // `width` by `height`.
            let entries: Vec<serde_json::Value> = colors
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let f = |v: u8| format!("{}", f64::from(v) / 255.0);
                    serde_json::json!({
                        "color": format!("({}, {}, {}, 1)", f(c[0]), f(c[1]), f(c[2])),
                        "index": i,
                    })
                })
                .collect();
            let width = 4.min(colors.len().max(1));
            let v = serde_json::json!({
                "name": name,
                "comment": "Written by gp sprites export (docs/formats/image-config.md)",
                "width": width,
                "height": colors.len().div_ceil(width),
                "colors": entries,
            });
            serde_json::to_string_pretty(&v).expect("json") + "\n"
        }
    }
}

/// Reads any of the four, by its contents.
pub fn read(text: &str) -> Result<Vec<[u8; 3]>, String> {
    let t = text.trim_start_matches('\u{feff}');
    let nums = |line: &str| -> Option<[u8; 3]> {
        let v: Vec<u8> = line
            .split_whitespace()
            .take(3)
            .filter_map(|x| x.parse().ok())
            .collect();
        (v.len() == 3).then(|| [v[0], v[1], v[2]])
    };
    if t.starts_with("GIMP Palette") {
        return Ok(t
            .lines()
            .skip(1)
            .filter(|l| !l.starts_with('#') && !l.contains(':'))
            .filter_map(nums)
            .collect());
    }
    if t.starts_with("JASC-PAL") {
        return Ok(t.lines().skip(3).filter_map(nums).collect());
    }
    if t.trim_start().starts_with('{') {
        let v: serde_json::Value = serde_json::from_str(t).map_err(|e| e.to_string())?;
        let mut out: Vec<(u64, [u8; 3])> = Vec::new();
        for e in v["colors"]
            .as_array()
            .ok_or("a JSON palette needs `colors`")?
        {
            let s = e["color"].as_str().ok_or("a color needs `color`")?;
            let f: Vec<f64> = s
                .trim_matches(|c| c == '(' || c == ')')
                .split(',')
                .filter_map(|x| x.trim().parse().ok())
                .collect();
            if f.len() < 3 {
                return Err(format!("color `{s}` isn't (r, g, b, a)"));
            }
            let b = |x: f64| (x * 255.0).round().clamp(0.0, 255.0) as u8;
            out.push((
                e["index"].as_u64().unwrap_or(out.len() as u64),
                [b(f[0]), b(f[1]), b(f[2])],
            ));
        }
        out.sort_by_key(|(i, _)| *i);
        return Ok(out.into_iter().map(|(_, c)| c).collect());
    }
    t.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with(';'))
        .map(|l| parse_hex(l).ok_or(format!("`{l}` isn't a hex color")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PaletteFile as P;

    #[test]
    fn every_palette_file_round_trips() {
        let colors = vec![
            [0, 0, 0],
            [255, 128, 7],
            [12, 200, 99],
            [1, 2, 3],
            [250, 250, 250],
        ];
        for k in [P::Gpl, P::Jasc, P::Hex, P::Pixelorama] {
            let text = write(k, "made up", &colors);
            assert_eq!(read(&text).unwrap(), colors, "{k:?}: {text}");
        }
        assert_eq!(parse_hex("#ff00ff"), Some([255, 0, 255]));
        assert_eq!(parse_hex("#ff00f"), None);
    }
}
