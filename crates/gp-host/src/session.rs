//! Interpreter sessions: a model loaded in the interpreter, kept between
//! calls, so an agent can step a copy, change it and step again without
//! paying the load each time.

use gp_interp::{Interp, Lifted};
use gp_mcp::Config;
use gp_mcp::tools;
use gp_runtime::{InputFrame, Runtime};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::StepRequest;

/// The most events a step returns.
const MAX_EVENTS: usize = 200;

pub struct Session {
    pub id: String,
    pub copy: String,
    stamp: String,
    rt: Interp,
    /// Every frame run so far, to replay after a reload.
    input: Vec<InputFrame>,
}

fn stamp(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn load(config: &Config, copy: &str) -> Result<(String, Interp), String> {
    let dir = config.existing_copy(copy)?;
    let (text, model) = tools::load_model(&dir)?;
    let assets = config
        .assets_of(&dir, model.model.source.as_deref())
        .ok_or("the model names no source, so start with --assets")?;
    let lifted = Lifted::read_dir(&assets, &model)
        .map_err(|e| format!("{e} (lift the model's assets with gp assets lift)"))?;
    Ok((stamp(&text), Interp::load(&model, &lifted)?))
}

impl Session {
    pub fn open(config: &Config, id: String, copy: &str) -> Result<Self, String> {
        let (stamp, rt) = load(config, copy)?;
        Ok(Self {
            id,
            copy: copy.to_owned(),
            stamp,
            rt,
            input: Vec::new(),
        })
    }

    pub fn info(&self) -> Value {
        json!({
            "session": self.id,
            "copy": self.copy,
            "frame": self.rt.frame(),
            "state_hash": format!("{:016x}", self.rt.state_hash()),
            "model": self.stamp,
        })
    }

    /// Reloads from the copy if its model changed, replaying the input so
    /// far. Returns whether it did.
    fn refresh(&mut self, config: &Config) -> Result<bool, String> {
        let dir = config.existing_copy(&self.copy)?;
        let (text, _) = tools::load_model(&dir)?;
        if stamp(&text) == self.stamp {
            return Ok(false);
        }
        let (stamp, mut rt) = load(config, &self.copy)?;
        for f in &self.input {
            rt.step(std::slice::from_ref(f))?;
        }
        self.stamp = stamp;
        self.rt = rt;
        Ok(true)
    }

    pub fn step(&mut self, config: &Config, req: &StepRequest) -> Result<Value, String> {
        let reloaded = self.refresh(config)?;
        let frames = req.frames.unwrap_or(1);
        if frames > tools::MAX_PREVIEW_FRAMES {
            return Err(format!(
                "a step runs at most {} frames",
                tools::MAX_PREVIEW_FRAMES
            ));
        }
        let mut holds = req.holds.clone();
        if !req.buttons.is_empty() {
            holds.push(format!("{}@0-", req.buttons.join("+")));
        }
        let spans: Vec<&str> = holds.iter().map(String::as_str).collect();
        let log = gp_evidence::input_log::generate(frames, &spans).map_err(|e| e.to_string())?;
        let mut events = Vec::new();
        let mut dropped = 0usize;
        for f in &log.frames {
            let frame = InputFrame {
                buttons: f.buttons_word(),
                axes: Vec::new(),
                reset: f.reset,
            };
            for e in self.rt.step(std::slice::from_ref(&frame))? {
                if e.kind == gp_interp::runtime::FRAME_KIND {
                    continue;
                }
                if events.len() < MAX_EVENTS {
                    events.push(json!({ "frame": e.frame, "kind": e.kind, "entity": e.entity }));
                } else {
                    dropped += 1;
                }
            }
            self.input.push(frame);
        }
        let mut out = self.info();
        out["reloaded"] = json!(reloaded);
        out["events"] = json!(events);
        if dropped > 0 {
            out["events_dropped"] = json!(dropped);
        }
        if !req.paths.is_empty() {
            out["state"] = tools::read_paths(&self.rt, &req.paths)?;
        }
        Ok(out)
    }

    pub fn query(&mut self, config: &Config, paths: &[String]) -> Result<Value, String> {
        let reloaded = self.refresh(config)?;
        let mut out = self.info();
        out["reloaded"] = json!(reloaded);
        out["state"] = tools::read_paths(&self.rt, paths)?;
        Ok(out)
    }
}
