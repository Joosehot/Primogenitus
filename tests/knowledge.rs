//! What Primo knows from the dictionary's definitions (run `primogenitus build`
//! first): what things eat, where they live, what they are for and made of,
//! the capitals, the definitions; and a question it cannot read is said to be
//! unread, never heard as a story.

use primogenitus::chat::Chat;
use primogenitus::config::Config;
use primogenitus::network::Network;
use primogenitus::raven::Raven;
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
fn from_the_definitions() {
    let Some((net, cfg)) = setup() else { return };
    let mut chat = Chat::new(&net, &cfg);
    for (q, want) in [
        ("What do dogs eat?", "flesh"),
        ("Where do fish live?", "water"),
        ("Where do frogs live?", "land"),
        ("What is a knife used for?", "cutting"),
        ("What is a pen used for?", "writing"),
        ("What is bread made of?", "dough"),
        ("What is the capital of Finland?", "Helsinki"),
        ("What is the capital of Japan?", "Tokyo"),
        ("What is the capital of France?", "Paris"),
        ("Define love", "emotion"),
        ("What does happiness mean?", "well-being"),
    ] {
        let r = chat.reply(q);
        println!("{q} -> {}", r.text);
        assert!(r.text.contains(want), "{q}: {}", r.text);
        // every sentence about the world it writes passes the grammar check (a quote is a quote)
        if !r.text.starts_with("My dictionary says") {
            assert!(primogenitus::parser::check(&net, &cfg, &r.text).is_ok(), "{q}: \"{}\" fails the check", r.text);
        }
    }
}

#[test]
fn a_question_not_read_is_not_a_story() {
    let Some((net, cfg)) = setup() else { return };
    let mut raven = Raven::new(&net, &cfg).unwrap();
    raven.greeting();
    raven.reply("Joose");
    for q in ["How many legs does a spider have?", "What is the square root of a carpet?", "Who won the war of the roses?"] {
        let r = raven.reply(q).text;
        println!("{q} -> {r}");
        assert!(!r.contains("told me") && !r.contains("telling me") && !r.contains("sharing"), "{q}: heard as a story: {r}");
    }
}
