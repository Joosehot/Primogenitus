//! Raven on the frozen network (run `primogenitus build` first): a
//! conversation where the humane judges must show.

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
fn a_humane_conversation() {
    let Some((net, cfg)) = setup() else { return };
    let mut raven = Raven::new(&net, &cfg).unwrap();
    let mut said: Vec<String> = Vec::new();
    let mut talk = |q: &str| {
        let r = raven.reply(q).text;
        println!("You: {q}\nPrimogenitus: {r}");
        said.push(r.clone());
        r
    };
    // the name is taken and used, once
    let r = talk("Hi, my name is Joose");
    assert!(r.contains("meet you") && r.matches("Joose").count() == 1, "{r}");
    // pain first: sympathy opens, no facts
    let r = talk("My dog died");
    assert!(r.contains("sorry") || r.contains("sad"), "{r}");
    assert!(!r.contains("Dogs "), "a fact while someone hurts: {r}");
    // a pet's name is remembered with the loss
    let r = talk("Her name was Musti");
    assert!(r.contains("Musti") && r.contains("miss"), "{r}");
    // joy is shared
    let r = talk("I am happy now");
    assert!(r.contains("glad") || r.contains("good") || r.contains("nice"), "{r}");
    // a question gets its answer, and what they told comes back
    let r = talk("I have a cat too");
    assert!(r.contains("cat") && !r.starts_with("Cats "), "{r}");
    let r = talk("Is a cat an animal?");
    assert!(r.starts_with("Yes. Cats are animals."), "{r}");
    assert!(r.contains("your cat"), "does not remember the cat: {r}");
    // thanks and goodbye
    let r = talk("Thanks for listening");
    assert!(r.contains("welcome") || r.contains("Any time"), "{r}");
    let r = talk("bye");
    assert!(!r.contains('?'), "a question at goodbye: {r}");
}
