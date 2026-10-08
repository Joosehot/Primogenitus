//! The proof, on the frozen network (run `primogenitus build` first):
//! every winning sentence parses under the grammar, and the same input gives
//! the same sentences every time.

use primogenitus::config::Config;
use primogenitus::input::Input;
use primogenitus::network::Network;
use std::path::PathBuf;

const TOPICS: &[&str] = &[
    "dog", "cat", "car", "tree", "teacher", "bread", "house", "bird", "horse", "river", "child", "doctor", "ship",
    "book", "flower", "king", "soldier", "farmer", "fish", "boat", "door", "window", "road", "city", "student",
    "mother", "baby", "snake", "bee", "wolf", "train", "bridge", "garden", "storm", "fire", "rain", "computer",
    "knife", "hammer", "chair", "table", "apple", "egg", "cow", "lion", "eagle", "nurse", "pilot", "judge", "thief",
];

fn setup() -> Option<(Network, Config)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let frozen = root.join("data").join("network.bin");
    if !frozen.exists() {
        eprintln!("no frozen network: run `primogenitus build` first");
        return None;
    }
    Some((Network::load(&frozen).unwrap(), Config::load(&root.join("rules.toml")).unwrap()))
}

#[test]
fn every_winner_parses_and_repeats() {
    let Some((net, cfg)) = setup() else { return };
    let mut said = 0;
    for t in TOPICS {
        let topic = Input::Topic(t.to_string()).start(&net).unwrap();
        let (a, wa) = primogenitus::generate(&net, &cfg, topic, cfg.search.seeds);
        let (b, wb) = primogenitus::generate(&net, &cfg, topic, cfg.search.seeds);
        assert_eq!(a.iter().map(|s| &s.text).collect::<Vec<_>>(), b.iter().map(|s| &s.text).collect::<Vec<_>>(), "{t}");
        assert_eq!(wa, wb);
        for s in &a {
            assert!(s.check.is_ok(), "{t}: \"{}\" failed the check: {:?}", s.text, s.check);
        }
        if let Some(w) = wa {
            said += 1;
            println!("{t:>10}: {}", a[w].text);
        }
    }
    println!("{said} of {} topics gave a sentence", TOPICS.len());
}

#[test]
fn the_check_refuses_bad_grammar() {
    let Some((net, cfg)) = setup() else { return };
    for bad in ["Dogs barks.", "The dog bark.", "An dog barks.", "A dogs bark.", "Dog barks.", "the dog barks.",
                "The dog barks", "Dogs chase.", "A dog are a canine.", "Dogs is loyal."] {
        assert!(primogenitus::parser::check(&net, &cfg, bad).is_err(), "{bad} passed");
    }
    for good in ["Dogs bark.", "The dog barks.", "A dog is a canine.", "Dogs chase a rabbit.", "An apple is a fruit."] {
        assert!(primogenitus::parser::check(&net, &cfg, good).is_ok(), "{good} failed");
    }
}
