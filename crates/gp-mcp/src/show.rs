//! `show_explain`: what a person showed the agent by playing the original
//! (docs/show.md). A play session in the workspace marks moments ("hit a
//! brick"); `gp serve` records the session and `gp show explain` lists,
//! for each mark, what the original did there that the model didn't. This
//! tool reads those explanations from the session's folder, so an agent
//! can answer the person about the moment they showed. Read only: it runs
//! nothing and changes nothing.

use std::fs;
use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Config, Reply};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShowArgs {
    /// The play session's name (`play-...`), as the workspace shows it.
    /// Default: the newest session.
    #[serde(default)]
    pub session: Option<String>,
    /// Only this mark, by its number.
    #[serde(default)]
    pub mark: Option<u64>,
}

fn sessions(home: &Path) -> Vec<Value> {
    let mut out: Vec<Value> = fs::read_dir(home.join("capture"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| fs::read(e.path().join("play.json")).ok())
        .filter_map(|b| serde_json::from_slice::<Value>(&b).ok())
        .collect();
    out.sort_by_key(|s| std::cmp::Reverse(s["started_ms"].as_u64().unwrap_or(0)));
    out
}

pub fn explain(config: &Config, a: ShowArgs) -> Result<Reply, String> {
    let all = sessions(&config.home);
    let s = match &a.session {
        Some(name) => all
            .iter()
            .find(|s| s["run"] == name.as_str())
            .ok_or_else(|| format!("no play session {name} under $GP_HOME/capture"))?,
        None => all
            .first()
            .ok_or("no play sessions yet: play the original in the workspace (gp serve, the play panel) and mark a moment")?,
    };
    let explanations = &s["explanations"];
    let marks: Vec<Value> = s["marks"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|m| a.mark.is_none_or(|n| m["id"] == n))
        .map(|m| {
            let id = m["id"].as_u64().unwrap_or(0).to_string();
            let e = &explanations[id.as_str()];
            json!({
                "id": m["id"],
                "frames": [m["from"], m["to"]],
                "note": m["note"],
                "annotation": m["annotation"],
                "explained": !e.is_null(),
                "missing": e["missing"],
                "model": e["model"],
                "original": e["original"],
            })
        })
        .collect();
    if a.mark.is_some() && marks.is_empty() {
        return Err(format!(
            "session {} has no mark {}",
            s["run"],
            a.mark.unwrap_or(0)
        ));
    }
    Ok(Reply::ok(json!({
        "session": s["run"],
        "state": s["state"],
        "template": s["template"],
        "recording": s["recording"],
        "store": s["store"],
        "real_store": s["real_store"],
        "frame_hashes_agree": s["digest_matches"],
        "marks": marks,
        "others": all.iter().skip(1).take(10).map(|x| x["run"].clone()).collect::<Vec<_>>(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_newest_session_and_one_mark() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        for (run, ms) in [("play-a-1", 1), ("play-b-2", 2)] {
            let d = home.join("capture").join(run);
            fs::create_dir_all(&d).unwrap();
            fs::write(
                d.join("play.json"),
                json!({
                    "run": run, "started_ms": ms, "state": "done", "template": "mosshop",
                    "recording": "ab", "digest_matches": true,
                    "marks": [{"id": 1, "from": 10, "to": 20, "note": "hit a brick", "by": "key"},
                              {"id": 2, "from": 30, "to": 40, "note": "", "by": "timeline"}],
                    "explanations": {"1": {"missing": ["background: 1 cell changes"], "model": {"events": []}}},
                })
                .to_string(),
            )
            .unwrap();
        }
        let c = Config {
            gp: "gp".into(),
            repo: dir.path().into(),
            work: dir.path().join("work"),
            home: home.clone(),
            rom: None,
            assets: None,
        };
        let r = explain(
            &c,
            ShowArgs {
                session: None,
                mark: None,
            },
        )
        .unwrap();
        assert_eq!(r.json["session"], "play-b-2");
        assert_eq!(
            r.json["marks"][0]["missing"][0],
            "background: 1 cell changes"
        );
        assert_eq!(r.json["marks"][1]["explained"], false);
        assert_eq!(r.json["others"], json!(["play-a-1"]));
        let r = explain(
            &c,
            ShowArgs {
                session: Some("play-a-1".into()),
                mark: Some(2),
            },
        )
        .unwrap();
        assert_eq!(r.json["marks"].as_array().unwrap().len(), 1);
        assert!(
            explain(
                &c,
                ShowArgs {
                    session: Some("nope".into()),
                    mark: None
                }
            )
            .is_err()
        );
    }
}
