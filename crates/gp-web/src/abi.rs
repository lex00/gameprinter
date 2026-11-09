//! The preview's exports to JavaScript, wasm32 only. Every function works
//! on one preview held by this module. Bytes go in through a buffer from
//! `gp_alloc`; text comes back in the output buffer, read with
//! `gp_out_ptr` and `gp_out_len`. A function returning `i32` returns 0 when
//! it worked and 1 when it didn't, with the error's text in the output
//! buffer.
//!
//! The browser runs a module on one thread, so the cells below are never
//! reached from two places at once.

#![allow(unsafe_code)]

use std::cell::RefCell;

use crate::Preview;

/// A save slot the page handed over: its number, the record kept and when
/// it was saved.
type Handed = (usize, Option<Vec<u8>>, Option<u64>);

thread_local! {
    static PREVIEW: RefCell<Option<Preview>> = const { RefCell::new(None) };
    static SETTINGS: RefCell<Option<gp_present::settings::Screen>> = const { RefCell::new(None) };
    static OUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static HASH: RefCell<[u8; 32]> = const { RefCell::new([0; 32]) };
    static REMOVED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// The slots handed over for the Saves line, until gp_settings_saves.
    static SLOTS: RefCell<Vec<Handed>> = const { RefCell::new(Vec::new()) };
    /// The shell's settings the last screen closed with.
    static SHELL: RefCell<Option<gp_present::shell::Settings>> = const { RefCell::new(None) };
}

fn set_out(bytes: &[u8]) {
    OUT.with(|o| {
        let mut o = o.borrow_mut();
        o.clear();
        o.extend_from_slice(bytes);
    });
}

fn result(r: Result<String, String>) -> i32 {
    match r {
        Ok(text) => {
            set_out(text.as_bytes());
            0
        }
        Err(e) => {
            set_out(e.as_bytes());
            1
        }
    }
}

fn with_preview<T>(f: impl FnOnce(&mut Preview) -> Result<T, String>) -> Result<T, String> {
    PREVIEW.with(|p| match p.borrow_mut().as_mut() {
        Some(p) => f(p),
        None => Err("no model loaded: call gp_load first".into()),
    })
}

/// Takes back the `len` bytes JavaScript wrote at `ptr`, a buffer from
/// `gp_alloc`.
fn input(ptr: *mut u8, len: usize) -> Box<[u8]> {
    // SAFETY: `ptr` and `len` are a boxed slice `gp_alloc` leaked, which
    // JavaScript filled and hands back here exactly once.
    unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) }
}

/// A buffer of `len` bytes for JavaScript to fill. The function it's passed
/// to frees it.
#[unsafe(no_mangle)]
pub extern "C" fn gp_alloc(len: usize) -> *mut u8 {
    Box::into_raw(vec![0u8; len].into_boxed_slice()) as *mut u8
}

#[unsafe(no_mangle)]
pub extern "C" fn gp_out_ptr() -> *const u8 {
    OUT.with(|o| o.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn gp_out_len() -> usize {
    OUT.with(|o| o.borrow().len())
}

thread_local! {
    static SESSIONS: std::cell::RefCell<crate::rt::Sessions> = std::cell::RefCell::default();
}

/// One runtime protocol call from `gp`'s host: an op number and its bytes
/// (`gp_runtime::wire`). 0 with the answer in the output buffer, 1 with the
/// error's words there.
#[unsafe(no_mangle)]
pub extern "C" fn gp_rt(op: u32, ptr: *mut u8, len: usize) -> i32 {
    let bytes = input(ptr, len);
    let mut w = gp_runtime::wire::Writer::default();
    match SESSIONS.with(|s| s.borrow_mut().call(op, &bytes, &mut w)) {
        Ok(()) => {
            set_out(&w.0);
            0
        }
        Err(e) => {
            set_out(e.as_bytes());
            1
        }
    }
}

/// A model bundle in a profile ([`crate::profile_bundle`]): the bundle,
/// the text of `profiles.toml` and the profile's name, each from
/// `gp_alloc`. The new bundle's bytes are in the output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn gp_profile_bundle(
    bundle: *mut u8,
    bundle_len: usize,
    profiles: *mut u8,
    profiles_len: usize,
    name: *mut u8,
    name_len: usize,
) -> i32 {
    let (b, p, n) = (
        input(bundle, bundle_len),
        input(profiles, profiles_len),
        input(name, name_len),
    );
    let made = String::from_utf8(p.to_vec())
        .map_err(|e| e.to_string())
        .and_then(|p| {
            let n = String::from_utf8(n.to_vec()).map_err(|e| e.to_string())?;
            crate::profile_bundle(&b, &p, &n)
        });
    match made {
        Ok(bytes) => {
            set_out(&bytes);
            0
        }
        Err(e) => {
            set_out(e.as_bytes());
            1
        }
    }
}

/// Loads a model bundle (JSON bytes) and powers it on with its model's
/// `ext` hooks. The output buffer holds the hooks' label, empty for a model
/// without hooks; a model naming a hook the module has no crate for is
/// refused ([`Preview::load`]).
#[unsafe(no_mangle)]
pub extern "C" fn gp_load(ptr: *mut u8, len: usize) -> i32 {
    let bytes = input(ptr, len);
    PREVIEW.with(|c| *c.borrow_mut() = None);
    result(Preview::load(&bytes).map(|p| {
        let hooks = p.hooks().to_owned();
        PREVIEW.with(|c| *c.borrow_mut() = Some(p));
        hooks
    }))
}

/// Runs one frame: controller 1's bits in the low byte of `buttons`,
/// controller 2's in the next.
#[unsafe(no_mangle)]
pub extern "C" fn gp_step(buttons: u32, reset: u32) -> i32 {
    result(with_preview(|p| p.step_pads(buttons as u16, reset != 0)).map(|_| String::new()))
}

/// Loads a canonical model state (`Preview::load_state`).
#[unsafe(no_mangle)]
pub extern "C" fn gp_load_state(ptr: *mut u8, len: usize) -> i32 {
    let bytes = input(ptr, len);
    result(with_preview(|p| p.load_state(&bytes)).map(|()| String::new()))
}

/// The canonical model state's bytes (`Preview::state`), in the output
/// buffer, for the page's save slots and rewind.
#[unsafe(no_mangle)]
pub extern "C" fn gp_state() -> i32 {
    match with_preview(|p| Ok(p.state())) {
        Ok(b) => {
            set_out(&b);
            0
        }
        Err(e) => {
            set_out(e.as_bytes());
            1
        }
    }
}

/// The game as a save record (`Preview::save_record`) in the output
/// buffer: the profile's name, from `gp_alloc`, is the one the page runs
/// the game in.
#[unsafe(no_mangle)]
pub extern "C" fn gp_save_record(ptr: *mut u8, len: usize) -> i32 {
    let name = input(ptr, len);
    match std::str::from_utf8(&name)
        .map_err(|e| e.to_string())
        .and_then(|n| with_preview(|p| Ok(p.save_record(n))))
    {
        Ok(b) => {
            set_out(&b);
            0
        }
        Err(e) => {
            set_out(e.as_bytes());
            1
        }
    }
}

/// Loads a save record (`Preview::load_record`), the record and the
/// profile the page runs the game in each from `gp_alloc`. Returns 0 when
/// it loaded; 1 when it was refused, with the sentence for the player in
/// the output buffer; 2 when it was saved in another profile, with that
/// profile's name in the output buffer and nothing loaded.
#[unsafe(no_mangle)]
pub extern "C" fn gp_load_record(ptr: *mut u8, len: usize, name: *mut u8, name_len: usize) -> i32 {
    let (bytes, name) = (input(ptr, len), input(name, name_len));
    // Another game or an older record is refused before the profile is
    // looked at: switching profile wouldn't make it load.
    use gp_sim::save::{OTHER_BUILD, OTHER_GAME, Record, VERSION};
    let record = Record::read(&bytes).and_then(|r| {
        if r.model != with_preview(|p| Ok(p.model().to_owned()))? {
            Err(OTHER_GAME.to_owned())
        } else if r.version != VERSION {
            Err(OTHER_BUILD.to_owned())
        } else {
            Ok(r)
        }
    });
    let record = match record {
        Ok(r) => r,
        Err(e) => return result(Err(e)),
    };
    if record.profile.as_bytes() != &name[..] {
        set_out(record.profile.as_bytes());
        return 2;
    }
    result(with_preview(|p| p.load_record(&bytes)).map(|()| String::new()))
}

/// Sizes and empties the rewind ring (`Preview::ring_reset`) for
/// `num`/`den` frames a second.
#[unsafe(no_mangle)]
pub extern "C" fn gp_ring_reset(num: u32, den: u32) -> i32 {
    result(with_preview(|p| {
        p.ring_reset(f64::from(num) / f64::from(den.max(1)));
        Ok(String::new())
    }))
}

/// Empties the rewind ring (`Preview::ring_clear`).
#[unsafe(no_mangle)]
pub extern "C" fn gp_ring_clear() -> i32 {
    result(with_preview(|p| {
        p.ring_clear();
        Ok(String::new())
    }))
}

/// Keeps the state in the rewind ring when its step says so
/// (`Preview::ring_note`), after a frame.
#[unsafe(no_mangle)]
pub extern "C" fn gp_ring_note() -> i32 {
    result(with_preview(|p| {
        p.ring_note();
        Ok(String::new())
    }))
}

/// One step of held rewind (`Preview::ring_back`): 0 when a state was put
/// back, 2 when the ring has none, 1 on an error.
#[unsafe(no_mangle)]
pub extern "C" fn gp_ring_back() -> i32 {
    match with_preview(|p| p.ring_back()) {
        Ok(true) => 0,
        Ok(false) => 2,
        Err(e) => {
            set_out(e.as_bytes());
            1
        }
    }
}

/// The state hash's 32 bytes, at a pointer valid until the next call.
#[unsafe(no_mangle)]
pub extern "C" fn gp_hash() -> *const u8 {
    let h = with_preview(|p| Ok(p.hash())).unwrap_or([0; 32]);
    HASH.with(|c| {
        *c.borrow_mut() = h;
        c.as_ptr() as *const u8
    })
}

/// The spec digest in hex, in the output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn gp_spec_digest() -> i32 {
    result(with_preview(|p| Ok(p.spec_digest())))
}

/// The draw state as JSON, in the output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn gp_draw_state() -> i32 {
    result(with_preview(|p| Ok(p.draw_state())))
}

/// Runs a frames file and writes its hash lines to the output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn gp_run_frames(ptr: *mut u8, len: usize) -> i32 {
    let bytes = input(ptr, len);
    result(
        String::from_utf8(bytes.into_vec())
            .map_err(|e| e.to_string())
            .and_then(|text| with_preview(|p| p.run_frames(&text))),
    )
}

/// Loads the presentation (`present.txt`, UTF-8 bytes) for `gp_scene`.
#[unsafe(no_mangle)]
pub extern "C" fn gp_load_present(ptr: *mut u8, len: usize) -> i32 {
    let bytes = input(ptr, len);
    result(
        String::from_utf8(bytes.into_vec())
            .map_err(|e| e.to_string())
            .and_then(|text| with_preview(|p| p.load_present(&text)))
            .map(|_| String::new()),
    )
}

/// What to draw this frame as JSON (`Preview::scene`), in the output
/// buffer.
#[unsafe(no_mangle)]
pub extern "C" fn gp_scene() -> i32 {
    result(with_preview(|p| p.scene()))
}

/// Sets the picture and sound options (`Preview::set_view`, UTF-8 bytes).
#[unsafe(no_mangle)]
pub extern "C" fn gp_view(ptr: *mut u8, len: usize) -> i32 {
    let bytes = input(ptr, len);
    result(with_preview(|p| {
        let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        p.set_view(text).map(|()| String::new())
    }))
}

/// Loads `assets/sound.json`; then each `gp_step` makes a frame of sound.
#[unsafe(no_mangle)]
pub extern "C" fn gp_load_sound(ptr: *mut u8, len: usize) -> i32 {
    let bytes = input(ptr, len);
    result(with_preview(|p| {
        let text = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
        p.load_sound(text).map(|()| String::new())
    }))
}

/// The last frame's samples (16-bit little-endian mono at 44,100 Hz) in
/// the output buffer; returns how many samples.
#[unsafe(no_mangle)]
pub extern "C" fn gp_audio() -> i32 {
    match with_preview(|p| {
        let mut b = Vec::with_capacity(p.audio().len() * 2);
        for s in p.audio() {
            b.extend_from_slice(&s.to_le_bytes());
        }
        Ok(b)
    }) {
        Ok(b) => {
            let n = (b.len() / 2) as i32;
            set_out(&b);
            n
        }
        Err(_) => -1,
    }
}

/// Opens the standard settings screen (docs/settings.md) over the game:
/// the text of the model's `profiles.toml` and the profile the game runs
/// in, by the name the player chose (`retro`, `normal`, or switches joined
/// by `+`), each from `gp_alloc`.
#[unsafe(no_mangle)]
pub extern "C" fn gp_settings_open(
    profiles: *mut u8,
    profiles_len: usize,
    name: *mut u8,
    name_len: usize,
) -> i32 {
    let (p, n) = (input(profiles, profiles_len), input(name, name_len));
    result((|| {
        let p = std::str::from_utf8(&p).map_err(|e| e.to_string())?;
        let n = std::str::from_utf8(&n).map_err(|e| e.to_string())?;
        let screen = gp_present::settings::Screen::open(crate::settings_catalog(p)?, n);
        SETTINGS.with(|s| *s.borrow_mut() = Some(screen));
        Ok(String::new())
    })())
}

/// Gives the open screen its Theme line: the looks found, a line each as
/// `id<TAB>title`, the build's own first, and the one shown by id, each
/// from `gp_alloc`. One look or none shows no line. A published theme's
/// line goes on as `<TAB>author<TAB>state<TAB>size`, the state
/// `downloaded` or `download` (docs/theme-publishing.md, "In the game").
#[unsafe(no_mangle)]
pub extern "C" fn gp_settings_themes(
    list: *mut u8,
    list_len: usize,
    current: *mut u8,
    current_len: usize,
) -> i32 {
    let (l, c) = (input(list, list_len), input(current, current_len));
    result((|| {
        let l = std::str::from_utf8(&l).map_err(|e| e.to_string())?;
        let c = std::str::from_utf8(&c).map_err(|e| e.to_string())?;
        use gp_present::settings::{Kept, ThemeMark};
        let mut themes: Vec<(String, String)> = Vec::new();
        let mut marks = std::collections::BTreeMap::new();
        for x in l.lines().filter(|x| !x.is_empty()) {
            let f: Vec<&str> = x.split('\t').collect();
            let id = f[0].to_owned();
            themes.push((id.clone(), f.get(1).unwrap_or(&f[0]).to_string()));
            let kept = match f.get(3) {
                Some(&"downloaded") => Some(Kept::Downloaded),
                Some(&"download") => Some(Kept::Download),
                _ => None,
            };
            if let Some(kept) = kept {
                let size = f.get(4).and_then(|n| n.parse().ok()).unwrap_or(0);
                let author = f.get(2).unwrap_or(&"").to_string();
                marks.insert(id, ThemeMark { author, kept, size });
            }
        }
        SETTINGS.with(|s| {
            let mut s = s.borrow_mut();
            let screen = s.take().ok_or("no settings screen is open")?;
            *s = Some(screen.with_themes(themes, c).with_marks(marks));
            Ok::<_, String>(())
        })?;
        Ok(String::new())
    })())
}

/// Sets a theme's sound for a role: its name and a 16-bit PCM WAV file,
/// each from `gp_alloc`. 0 when set, 1 with the reason in the output
/// buffer (no sound loaded, no such role, not a WAV).
#[unsafe(no_mangle)]
pub extern "C" fn gp_sound_role(
    name: *mut u8,
    name_len: usize,
    wav: *mut u8,
    wav_len: usize,
) -> i32 {
    let (n, w) = (input(name, name_len), input(wav, wav_len));
    result((|| {
        let n = std::str::from_utf8(&n).map_err(|e| e.to_string())?;
        if with_preview(|p| p.set_sound_role(n, &w))? {
            Ok(String::new())
        } else {
            Err(format!("the model has no sound role `{n}`"))
        }
    })())
}

/// Takes every role's sound away: the model's own sound plays again.
#[unsafe(no_mangle)]
pub extern "C" fn gp_sound_roles_clear() -> i32 {
    match with_preview(|p| {
        p.clear_sound_roles();
        Ok(())
    }) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

/// Whether the last frame started a level, so the page autosaves into slot
/// 0 (`gp_present::shell::autosaves`): 1 or 0.
#[unsafe(no_mangle)]
pub extern "C" fn gp_autosave_due() -> i32 {
    with_preview(|p| Ok(i32::from(p.autosave_due()))).unwrap_or(0)
}

/// Gives the open screen the shell's settings the game runs in, as their
/// file holds them (`gp_present::shell::Settings::parse`), from
/// `gp_alloc`: the Console only and Quick save buttons lines.
#[unsafe(no_mangle)]
pub extern "C" fn gp_settings_shell(ptr: *mut u8, len: usize) -> i32 {
    let text = input(ptr, len);
    result((|| {
        let t = std::str::from_utf8(&text).map_err(|e| e.to_string())?;
        let shell = gp_present::shell::Settings::parse(t);
        SETTINGS.with(|s| {
            let mut s = s.borrow_mut();
            let screen = s.take().ok_or("no settings screen is open")?;
            *s = Some(screen.with_shell(shell));
            Ok(String::new())
        })
    })())
}

/// The shell's settings the last screen closed with, as their file holds
/// them, in the output buffer; 1 when no screen offered them.
#[unsafe(no_mangle)]
pub extern "C" fn gp_settings_shell_chosen() -> i32 {
    match SHELL.with(|s| *s.borrow()) {
        Some(x) => result(Ok(x.to_text())),
        None => result(Err("no screen closed with the shell's settings".into())),
    }
}

/// Hands over one save slot for the Saves line: its number, the record
/// kept from `gp_alloc` (length 0 for an empty slot) and when it was saved
/// in seconds since 1970 (negative for no stamp). gp_settings_saves lists
/// the slots handed over.
#[unsafe(no_mangle)]
pub extern "C" fn gp_settings_slot(slot: u32, ptr: *mut u8, len: usize, when: f64) -> i32 {
    let bytes = (len > 0).then(|| input(ptr, len).into_vec());
    let when = (when >= 0.0).then_some(when as u64);
    SLOTS.with(|s| s.borrow_mut().push((slot as usize, bytes, when)));
    0
}

/// Lists the slots handed over on the open screen's Saves line, each read
/// by the preview (`Preview::slot_line`): `profile` (from `gp_alloc`) is
/// the one the page runs the game in, `now` the page's clock in seconds
/// since 1970, `num`/`den` the frame rate, and `files` whether a save can
/// be loaded from a file.
#[unsafe(no_mangle)]
pub extern "C" fn gp_settings_saves(
    profile: *mut u8,
    profile_len: usize,
    now: f64,
    num: u32,
    den: u32,
    files: u32,
) -> i32 {
    let profile = input(profile, profile_len);
    let slots = SLOTS.with(|s| std::mem::take(&mut *s.borrow_mut()));
    result((|| {
        let profile = std::str::from_utf8(&profile).map_err(|e| e.to_string())?;
        let lines = with_preview(|p| {
            Ok(slots
                .iter()
                .map(|(n, b, w)| p.slot_line(*n, b.as_deref(), *w, profile))
                .collect())
        })?;
        let saves = gp_present::settings::Saves {
            lines,
            now: (now >= 0.0).then_some(now as u64),
            rate: f64::from(num) / f64::from(den.max(1)),
            files: files != 0,
        };
        SETTINGS.with(|s| {
            let mut s = s.borrow_mut();
            let screen = s.as_mut().ok_or("no settings screen is open")?;
            screen.set_saves(saves);
            Ok(String::new())
        })
    })())
}

/// The open screen as text (`gp_present::settings::Screen::lines`), in
/// the output buffer: a line each, the cursor's marked, and what is shown
/// under the list last. For the page's checks.
#[unsafe(no_mangle)]
pub extern "C" fn gp_settings_lines() -> i32 {
    match SETTINGS.with(|s| s.borrow().as_ref().map(|x| x.lines().join("\n"))) {
        Some(t) => result(Ok(t)),
        None => result(Err("no settings screen is open".into())),
    }
}

/// Shows a sentence on the open screen, under the list, until the cursor
/// moves: what the page did with a slot. From `gp_alloc`.
#[unsafe(no_mangle)]
pub extern "C" fn gp_settings_say(ptr: *mut u8, len: usize) -> i32 {
    let text = input(ptr, len);
    result((|| {
        let t = std::str::from_utf8(&text).map_err(|e| e.to_string())?;
        SETTINGS.with(|s| {
            s.borrow_mut()
                .as_mut()
                .ok_or("no settings screen is open")?
                .say(t);
            Ok(String::new())
        })
    })())
}

/// Closes the open screen after a load from it
/// (`gp_present::settings::Screen::close_loaded`): returns 2 with the
/// setting to keep (and the look, on a second line), as a key that closes
/// it does; -1 when no screen is open.
#[unsafe(no_mangle)]
pub extern "C" fn gp_settings_close_loaded() -> i32 {
    let out = SETTINGS.with(|s| s.borrow_mut().as_mut().map(|screen| screen.close_loaded()));
    closed(out)
}

/// A key on the open screen ([`crate::settings_key`]'s numbers). Returns 0
/// when nothing changed, 1 to draw it again, 2 when it closed with the
/// game's switches as they were and 3 when it closed with new ones, so the
/// game starts again from power-on; on a close the output buffer holds the
/// setting to keep. 4 when the player asked for something done with a save
/// slot, the screen staying open: the output buffer says what, as `save N`,
/// `load N`, `delete N`, `download N` or `file`. -1 when no screen is open
/// or the key isn't one.
#[unsafe(no_mangle)]
pub extern "C" fn gp_settings_key(code: u32) -> i32 {
    let Some(key) = crate::settings_key(code) else {
        return -1;
    };
    let out = SETTINGS.with(|s| {
        s.borrow_mut().as_mut().map(|screen| {
            let o = screen.key(key);
            REMOVED.with(|r| r.borrow_mut().append(&mut screen.removed));
            o
        })
    });
    closed(out)
}

fn closed(out: Option<gp_present::settings::Outcome>) -> i32 {
    use gp_present::settings::{Outcome, SlotAction};
    match out {
        None => -1,
        Some(Outcome::Nothing) => 0,
        Some(Outcome::Redraw) => 1,
        Some(Outcome::Slot(a)) => {
            let text = match a {
                SlotAction::Save(n) => format!("save {n}"),
                SlotAction::Load(n) => format!("load {n}"),
                SlotAction::Delete(n) => format!("delete {n}"),
                SlotAction::Download(n) => format!("download {n}"),
                SlotAction::LoadFile => "file".to_owned(),
            };
            set_out(text.as_bytes());
            4
        }
        Some(Outcome::Closed {
            profile,
            restart,
            theme,
        }) => {
            let shell = SETTINGS.with(|s| s.borrow().as_ref().and_then(|x| x.shell));
            SHELL.with(|s| *s.borrow_mut() = shell);
            SETTINGS.with(|s| *s.borrow_mut() = None);
            // The look chosen follows on a second line when the screen
            // listed looks (gp_settings_themes).
            let out = match theme {
                Some(t) => format!("{profile}\n{t}"),
                None => profile,
            };
            set_out(out.as_bytes());
            if restart { 3 } else { 2 }
        }
    }
}

/// The open screen as RGBA pixels, `w` by `h`, in the output buffer.
#[unsafe(no_mangle)]
pub extern "C" fn gp_settings_draw(w: u32, h: u32) -> i32 {
    match SETTINGS.with(|s| {
        s.borrow()
            .as_ref()
            .map(|screen| screen.draw(w as usize, h as usize))
    }) {
        Some(px) => {
            set_out(&px);
            0
        }
        None => {
            set_out(b"no settings screen is open");
            1
        }
    }
}

/// The themes the player removed from the cache on the settings screen
/// since the last call, by id, a line each in the output buffer; returns
/// how many. The page deletes their files.
#[unsafe(no_mangle)]
pub extern "C" fn gp_settings_removed() -> i32 {
    let ids = REMOVED.with(|r| std::mem::take(&mut *r.borrow_mut()));
    set_out(ids.join("\n").as_bytes());
    ids.len() as i32
}

/// The BLAKE3 hash of some bytes from `gp_alloc`, as 64 hex digits in the
/// output buffer: how the page checks each downloaded file.
#[unsafe(no_mangle)]
pub extern "C" fn gp_blake3(ptr: *mut u8, len: usize) -> i32 {
    let b = input(ptr, len);
    set_out(gp_package::hash(&b).as_bytes());
    0
}

/// The themes of a game's index (its text) that this build can draw, by
/// its picture file names (a line each, in order): a line a theme in the
/// output buffer, `id<TAB>title<TAB>author<TAB>package<TAB>size<TAB>preview`.
/// 1 with the reason when the index doesn't read.
#[unsafe(no_mangle)]
pub extern "C" fn gp_catalogue(
    index: *mut u8,
    index_len: usize,
    pictures: *mut u8,
    pictures_len: usize,
) -> i32 {
    let (ix, pics) = (input(index, index_len), input(pictures, pictures_len));
    result((|| {
        let ix = std::str::from_utf8(&ix).map_err(|e| e.to_string())?;
        let pics = std::str::from_utf8(&pics).map_err(|e| e.to_string())?;
        let names: Vec<&str> = pics.lines().collect();
        let themes = gp_present::catalogue::offered(ix, &names)?;
        let clean = |s: &str| s.replace(['\t', '\n'], " ");
        Ok(themes
            .iter()
            .map(|t| {
                format!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    t.id,
                    clean(&t.title),
                    clean(&t.author),
                    t.package,
                    t.size,
                    t.preview
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    })())
}

/// A package's look files, from its manifest's bytes checked against the
/// package id (both from `gp_alloc`): a line each, `hash<TAB>size<TAB>path`,
/// the path in the look's folder. 1 with the reason on a mismatch.
#[unsafe(no_mangle)]
pub extern "C" fn gp_package_look(
    id: *mut u8,
    id_len: usize,
    manifest: *mut u8,
    manifest_len: usize,
) -> i32 {
    let (id, m) = (input(id, id_len), input(manifest, manifest_len));
    result((|| {
        let id = std::str::from_utf8(&id).map_err(|e| e.to_string())?;
        let files = gp_present::catalogue::look_files(id, &m)?;
        Ok(files
            .iter()
            .map(|(rel, f)| format!("{}\t{}\t{rel}", f.hash, f.size))
            .collect::<Vec<_>>()
            .join("\n"))
    })())
}
