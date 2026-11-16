use super::*;

#[test]
fn a_github_remote_by_any_of_its_spellings() {
    let want = Some(("acme".to_owned(), "harbor-registry".to_owned()));
    for url in [
        "https://github.com/acme/harbor-registry",
        "https://github.com/acme/harbor-registry.git",
        "https://github.com/acme/harbor-registry/",
        "git@github.com:acme/harbor-registry.git",
        "ssh://git@github.com/acme/harbor-registry.git",
    ] {
        assert_eq!(github_of(url), want, "{url}");
    }
    for url in [
        "/tmp/acme-registry.git",
        "https://gitlab.com/acme/harbor-registry",
        "https://github.com/acme",
        "https://github.com/acme/harbor/registry",
        "https://github.com/acme/<x>",
    ] {
        assert_eq!(github_of(url), None, "{url}");
    }
}

#[test]
fn the_raw_address() {
    assert_eq!(
        raw_of("acme", "harbor-registry", "main"),
        "https://raw.githubusercontent.com/acme/harbor-registry/main"
    );
}

#[test]
fn with_no_registry_yet_the_first_step_is_the_repo() {
    let o = host(&HostArgs {
        dir: None,
        github: None,
        theme: Some("acme/harbor-lights"),
        theme_dir: None,
        game: Some("acme/rooftop"),
    })
    .unwrap();
    assert_eq!(o.json["stage"]["n"], 1, "{}", o.text);
    assert_eq!(o.json["next"]["who"], "person");
    assert_eq!(o.json["raw"], Value::Null);
    assert_eq!(o.json["steps"].as_array().unwrap().len(), 6);
    // Every step that writes to the outside world says to ask first.
    for s in o.json["steps"].as_array().unwrap() {
        let writes = s["cli"].as_array().unwrap().iter().any(|c| {
            let c = c.as_str().unwrap();
            [
                "init",
                "grant",
                "copy",
                "publish",
                "reindex",
                "deploy.sh",
                "gh repo create",
            ]
            .iter()
            .any(|w| c.contains(w))
        });
        if writes {
            assert_eq!(s["asks_first"], true, "{s}");
        }
    }
}
