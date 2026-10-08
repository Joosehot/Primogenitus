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
    // answers read in the light of the question asked
    let mut raven2 = Raven::new(&net, &cfg).unwrap();
    let g = raven2.greeting();
    assert!(g.ends_with("What is your name?"), "{g}");
    let r = raven2.reply("Joose").text;
    println!("You: Joose\nPrimogenitus: {r}");
    assert!(r.contains("Joose") && r.contains("meet"), "a bare name not read as a name: {r}");
    let r = raven2.reply("I have a dog").text;
    println!("You: I have a dog\nPrimogenitus: {r}");
    let r2 = raven2.reply(if r.contains("name?") { "Rex" } else if r.contains("How old") { "3 years" } else { "very playful" }).text;
    println!("Primogenitus: {r2}");
    assert!(!r2.contains("did not"), "the answer to its own question not read: {r2}");
    let r3 = raven2.reply("she is very friendly").text;
    println!("You: she is very friendly\nPrimogenitus: {r3}");
    assert!(!r3.contains("did not"), "\"she\" not read as the dog: {r3}");
    // a story asked for is heard; "playful and friendly" stays about the dog
    let r = raven2.reply("she is playful and friendly").text;
    println!("You: she is playful and friendly\nPrimogenitus: {r}");
    assert!(!r.to_lowercase().contains("friendlies"), "{r}");
    let r = raven2.reply("My old dog died last year").text;
    println!("You: My old dog died last year\nPrimogenitus: {r}");
    assert!(r.contains("sorry") || r.contains("sad"), "{r}");
    let r = raven2.reply("we used to walk together every morning").text;
    println!("You: we used to walk together every morning\nPrimogenitus: {r}");
    assert!(!r.contains("did not"), "a story not heard: {r}");
    let r = talk("bye");
    assert!(!r.contains('?'), "a question at goodbye: {r}");
}
