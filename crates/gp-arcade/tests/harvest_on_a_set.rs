//! The static harvest on a game's main CPU, against what was found by
//! hand: the game folder $GP_GAME_DIR names keeps it in harvest.toml
//! (the facts, lists, jump tables and state machines the harvest must
//! find, on the set it names). Skipped without the game, its harvest.toml
//! or the set's zip (gp_arcade::romset::test_set).

use gp_arcade::harvest::{Board, Harvest, Value, harvest};

#[derive(serde::Deserialize)]
struct Want {
    set: String,
    #[serde(default)]
    fact: Vec<Fact>,
    #[serde(default)]
    list: Vec<List>,
    #[serde(default)]
    dispatch: Vec<Dispatch>,
    #[serde(default)]
    machine: Vec<Machine>,
}

#[derive(serde::Deserialize)]
struct Fact {
    path: String,
    value: Option<toml::Value>,
}

#[derive(serde::Deserialize)]
struct List {
    path: String,
    starts_with: Vec<String>,
}

#[derive(serde::Deserialize)]
struct Dispatch {
    at: String,
    subject: String,
    cases: usize,
}

#[derive(serde::Deserialize)]
struct Machine {
    byte: String,
    transitions: Vec<[String; 3]>,
}

fn same(v: &Value, want: &toml::Value) -> bool {
    match (v, want) {
        (Value::Int(a), toml::Value::Integer(b)) => a == b,
        (Value::Str(a), toml::Value::String(b)) => a == b,
        (Value::Bool(a), toml::Value::Boolean(b)) => a == b,
        _ => false,
    }
}

fn check(h: &Harvest, want: &Want) {
    for f in &want.fact {
        let found: Vec<&Value> = h
            .facts
            .iter()
            .filter(|x| x.path == f.path)
            .map(|x| &x.value)
            .collect();
        match &f.value {
            None => assert!(!found.is_empty(), "no fact {}", f.path),
            Some(w) => assert!(
                found.iter().any(|v| same(v, w)),
                "{}: {found:?}, not {w}",
                f.path
            ),
        }
    }
    for l in &want.list {
        let Some(Value::List(items)) = h.facts.iter().find(|f| f.path == l.path).map(|f| &f.value)
        else {
            panic!("no list {}", l.path)
        };
        for start in &l.starts_with {
            assert!(
                items
                    .iter()
                    .any(|c| matches!(c, Value::Str(x) if x.starts_with(start.as_str()))),
                "{start} in {items:?}"
            );
        }
    }
    for d in &want.dispatch {
        let table = h
            .dispatch
            .iter()
            .find(|t| t.at == d.at && t.subject.starts_with(d.subject.as_str()))
            .unwrap_or_else(|| panic!("no jump table at {} on {}", d.at, d.subject));
        assert_eq!(table.cases.len(), d.cases, "the table at {}", d.at);
    }
    for m in &want.machine {
        let found = h
            .machines
            .iter()
            .find(|x| x.byte == m.byte)
            .unwrap_or_else(|| panic!("no state machine on {}", m.byte));
        let t: Vec<[&str; 3]> = found
            .transitions
            .iter()
            .map(|t| [t.from.as_str(), t.to.as_str(), t.routine.as_str()])
            .collect();
        for w in &m.transitions {
            let w = [w[0].as_str(), w[1].as_str(), w[2].as_str()];
            assert!(t.contains(&w), "{w:?} in {t:?}");
        }
    }
}

#[test]
fn the_harvest_finds_what_the_game_found_by_hand() {
    if gp_arcade::skip_slow_test() {
        return;
    }
    let game = match gp_arcade::romset::test_game() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("skipped: {e}");
            return;
        }
    };
    let Ok(text) = std::fs::read_to_string(game.join("harvest.toml")) else {
        eprintln!("skipped: {} has no harvest.toml", game.display());
        return;
    };
    let want: Want = toml::from_str(&text).expect("harvest.toml reads");
    let set = match gp_arcade::romset::test_set(&want.set) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skipped: {e}");
            return;
        }
    };
    check(&harvest(&Board::mcr_main(&set.maincpu)), &want);
}
