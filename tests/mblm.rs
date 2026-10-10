//! MBLM on the frozen network (run `primogenitus build` first): every
//! distribution sums to one, every sentence it writes passes the grammar
//! check, it is the same every time, and it scores its own kind of text far
//! above words in a scrambled order.

use primogenitus::config::Config;
use primogenitus::mblm::{Mblm, St};
use primogenitus::network::Network;
use std::path::PathBuf;

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
fn a_language_model_by_the_motor() {
    let Some((net, cfg)) = setup() else { return };
    let m = Mblm::new(&net, &cfg);
    for prompt in ["Dogs", "A dog is", "The tree", "Bread", "Tell me about whales", "Cats", "A teacher", "Horses"] {
        let (text, toks) = m.complete(prompt, 4).unwrap();
        println!("{prompt}|{text}");
        assert!(!toks.is_empty(), "{prompt}: nothing");
        assert_eq!(m.complete(prompt, 4).unwrap().0, text, "{prompt}: not the same twice");
        // every whole sentence passes the grammar check
        let full = format!("{prompt}{text}");
        for s in full.split_inclusive('.').map(|s| s.trim()).filter(|s| s.ends_with('.')) {
            let s = if s.starts_with("Tell me about whales") { s.trim_start_matches("Tell me about whales").trim() } else { s };
            assert!(primogenitus::parser::check(&net, &cfg, s).is_ok(), "{prompt}: \"{s}\" fails the check");
        }
    }
    // every distribution sums to one
    let topic = m.topic_of("dogs").unwrap();
    let p = m.para(topic);
    for st in [St::Start, St::Subj(primogenitus::search::Num::Pl), St::Cop(primogenitus::search::Num::Sg)] {
        let sum: f64 = m.next(st, &p).iter().map(|t| t.p).sum();
        assert!((sum - 1.0).abs() < 1e-9, "{st:?}: {sum}");
    }
    // its own kind of text far above the same words scrambled
    let good = m.score("Dogs bark. They chase rabbits.").unwrap();
    let bad = m.score("Bark dogs. Rabbits chase they.").unwrap();
    println!("good: perplexity {:.2}, coverage {:.2}; scrambled: coverage {:.2}", good.perplexity, good.coverage, bad.coverage);
    assert!(good.coverage == 1.0 && bad.coverage < 0.5);
}
