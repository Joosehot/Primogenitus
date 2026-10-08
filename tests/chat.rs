//! The chatbot on the frozen network (run `primogenitus build` first): known
//! facts answered yes, made-up ones not, pronouns followed, "what else" runs
//! to its end, and every sentence about the world passes the grammar check.

use primogenitus::chat::Chat;
use primogenitus::config::Config;
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

/// The sentence about the world inside a reply ("Yes. Dogs bark." -> "Dogs bark.").
fn world(reply: &str) -> Option<&str> {
    let r = reply.strip_prefix("Yes. ").unwrap_or(reply);
    let framing = ["Not that I know", "That is all", "I know of nothing", "I do not know", "I could not", "Who or what",
                   "Hello", "Goodbye", "You are welcome", "Ask me", "I know the word"];
    (!framing.iter().any(|f| r.starts_with(f))).then_some(r)
}

#[test]
fn a_conversation() {
    let Some((net, cfg)) = setup() else { return };
    let mut chat = Chat::new(&net, &cfg);
    let mut said = Vec::new();
    let mut ask = |q: &str| {
        let r = chat.reply(q).text;
        println!("You: {q}\nPrimo: {r}");
        if let Some(s) = world(&r) {
            assert!(primogenitus::parser::check(&net, &cfg, s).is_ok(), "\"{s}\" fails the check");
        }
        said.push(r.clone());
        r
    };
    assert!(ask("Hello").starts_with("Hello"));
    assert!(ask("Is a dog an animal?").starts_with("Yes."));
    assert!(ask("Is a dog a cat?").starts_with("Not that I know"));
    assert!(ask("Does a dog have a tail?").starts_with("Yes."));
    assert!(ask("Can dogs bark?").starts_with("Yes."));
    assert!(ask("Can it fly?").starts_with("Not that I know"));
    assert!(ask("Is an apple a fruit?").starts_with("Yes."));
    assert!(ask("Is a whale a mammal?").starts_with("Yes."));
    assert!(ask("Is a whale a fish?").starts_with("Not that I know"));
    // pronouns follow the topic
    ask("What is a tree?");
    assert!(!ask("What does it do?").starts_with("Who or what"));
    // "what else" never repeats and runs to its end
    let mut seen = std::collections::HashSet::new();
    let first = ask("What does a dog do?");
    seen.insert(first);
    let mut ended = false;
    for _ in 0..30 {
        let r = ask("What else?");
        if r.starts_with("That is all") {
            ended = true;
            break;
        }
        assert!(seen.insert(r.clone()), "\"{r}\" said twice");
    }
    assert!(ended, "\"what else\" never ran out");
    assert!(ask("What is a blorft?").starts_with("I do not know the word"));
    assert!(ask("What is a dog doing in the garden?").starts_with("I could not"));
}
