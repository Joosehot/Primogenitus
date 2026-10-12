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

/// A conversation as it really went (2026-10-08), every turn of it read.
#[test]
fn the_conversation_that_went_wrong() {
    let Some((net, cfg)) = setup() else { return };
    let mut raven = Raven::new(&net, &cfg).unwrap();
    raven.greeting();
    let mut talk = |q: &str| {
        let r = raven.reply(q).text;
        println!("You: {q}\nPrimogenitus: {r}");
        assert!(!r.contains("did not") && !r.contains("do not know the word"), "not followed: {q} -> {r}");
        r
    };
    let r = talk("I am Joose, your original creator");
    assert!(r.contains("Joose") && (r.contains("owe") || r.contains("made me") || r.contains("know me")), "{r}");
    let r = talk("I am fine, i guess");
    assert!(!r.contains("made you feel"), "{r}");
    let r = talk("The helpless search for purpose");
    assert!(r.contains("heavy") || r.contains("hard") || r.contains("hear you"), "{r}");
    talk("I am an artificial intellegence architech");
    let r = talk("Do you know that you are not an artificial intellegence but you can still talk?");
    assert!(r.contains("program"), "{r}");
    let r = talk("Do you know what's AI?");
    assert!(r.to_lowercase().contains("intelligence") || r.contains("AI") || r.contains("ai "), "{r}");
    talk("Good");
    let r = talk("Nothing really");
    assert!(!r.contains("glad you told me"), "{r}");
    let r = talk("Can you tell me something about yourself");
    assert!(r.contains("Primo"), "{r}");
    let r = talk("i meant you");
    assert!(r.contains("Primogenitus") || r.contains("program"), "{r}");
}

/// The second conversation as it really went (2026-10-08): the thread kept.
#[test]
fn the_thread_is_kept() {
    let Some((net, cfg)) = setup() else { return };
    let mut raven = Raven::new(&net, &cfg).unwrap();
    raven.greeting();
    let mut talk = |q: &str| {
        let r = raven.reply(q).text;
        println!("You: {q}\nPrimogenitus: {r}");
        assert!(!r.contains("did not"), "not followed: {q} -> {r}");
        r
    };
    talk("I'm Joose");
    let r = talk("I'm feeling a little sad.");
    assert!(r.contains("sorry") || r.contains("hard") || r.contains("hear"), "{r}");
    let r = talk("Yes I want to.");
    assert!(r.contains("listening") || r.contains("Take your time"), "\"yes, I want to talk\" not heard: {r}");
    assert!(!r.contains("about yourself"), "{r}");
    let r = talk("I am a 22-year old male human and I like to code");
    assert!(r.contains("cod") || r.contains("feel"), "{r}");
    let r = talk("Every day");
    assert!(!r.contains("Dull"), "an answer taken for a new topic: {r}");
    let r = talk("What do you mean by that?");
    assert!(r.contains("Sorry"), "{r}");
}

/// The third conversation as it really went (2026-10-08): a breakup.
#[test]
fn a_breakup() {
    let Some((net, cfg)) = setup() else { return };
    let mut raven = Raven::new(&net, &cfg).unwrap();
    raven.greeting();
    let mut talk = |q: &str| {
        let r = raven.reply(q).text;
        println!("You: {q}\nPrimogenitus: {r}");
        assert!(!r.contains("did not") && !r.contains("glad"), "{q} -> {r}");
        r
    };
    talk("Joose");
    talk("Sad");
    let r = talk("I broke up");
    assert!(r.contains("sorry") || r.contains("hard"), "{r}");
    talk("Broken");
    talk("Yeah it is");
    talk("Yes it is");
}

/// The fourth conversation as it really went (2026-10-08): the context held.
#[test]
fn the_context_holds() {
    let Some((net, cfg)) = setup() else { return };
    let mut raven = Raven::new(&net, &cfg).unwrap();
    raven.greeting();
    let mut talk = |q: &str| {
        let r = raven.reply(q).text;
        println!("You: {q}\nPrimogenitus: {r}");
        assert!(!r.contains("did not") && !r.contains("glad"), "{q} -> {r}");
        r
    };
    talk("Joose");
    talk("Sad");
    let r = talk("I broke up with my fiancee");
    assert!(r.contains("sorry") || r.contains("hard"), "{r}");
    let asked_to_talk = r.contains("talk about it");
    let r = talk("Yes");
    // a yes to "do you want to talk about it?" is heard as wanting to talk
    assert!(!asked_to_talk || r.contains("listening") || r.contains("Take your time"), "{r}");
    talk("So she threw shit at me and then just left");
    talk("So that's about it");
    talk("Depressed");
    talk("Maybe talking to my family");
    let r = talk("Goodbye!");
    assert!(!r.contains('?'), "{r}");
}

/// The fifth conversation as it really went (2026-10-11): talking about it.
#[test]
fn talking_about_it() {
    let Some((net, cfg)) = setup() else { return };
    let mut raven = Raven::new(&net, &cfg).unwrap();
    raven.greeting();
    let mut talk = |q: &str| {
        let r = raven.reply(q).text;
        println!("You: {q}
Primogenitus: {r}");
        assert!(!r.contains("did not") && !r.contains("told me"), "{q} -> {r}");
        r
    };
    talk("Joose");
    talk("I'm fine");
    talk("Pretty well");
    talk("Yeah");
    talk("Are you a robot?");
    talk("I am your creator.");
    let r = talk("Let's talk about you");
    assert!(r.contains("Primogenitus") || r.contains("myself"), "{r}");
    let r = talk("Let’s talk");
    assert!(r.contains("Gladly"), "{r}");
    let r = talk("Can we talk about dogs?");
    assert!(r.contains("dog") || r.contains("Dog"), "{r}");
}

/// A long conversation: the whole of it is remembered, compacting runs, and
/// what was told at the start is still known at the end.
#[test]
fn the_whole_conversation_is_remembered() {
    let Some((net, mut cfg)) = setup() else { return };
    cfg.raven.memory.max_words = 300;
    cfg.raven.memory.keep_words = 150;
    let mut raven = Raven::new(&net, &cfg).unwrap();
    raven.greeting();
    raven.reply("Joose");
    raven.reply("I have a dog");
    raven.reply("Rex");
    raven.reply("I like to code");
    for q in ["What is a cat?", "What does a bird do?", "Is a whale a mammal?", "What is a tree?", "What do dogs chase?",
              "Which animals swim?", "What is bread?", "Is an apple a fruit?", "What animals have wings?", "What is music?"]
        .iter()
        .cycle()
        .take(60)
    {
        raven.reply(q);
    }
    assert!(raven.transcript.compactions > 0, "never compacted");
    assert!(raven.transcript.words() <= 300, "kept text past the limit: {}", raven.transcript.words());
    assert!(raven.transcript.turns() >= 64);
    let r = raven.reply("What is my name?").text;
    println!("Primogenitus: {r}");
    assert!(r.contains("Joose"), "{r}");
    let r = raven.reply("What is my dog's name?").text;
    println!("Primogenitus: {r}");
    assert!(r.contains("Rex"), "{r}");
    let r = raven.reply("What do you remember about me?").text;
    println!("Primogenitus: {r}");
    assert!(r.contains("Joose") && r.contains("dog") && r.contains("Rex") && r.contains("cod"), "{r}");
}

/// The sixth conversation as it really went (2026-10-11): the thread kept.
#[test]
fn motors_not_robots() {
    let Some((net, cfg)) = setup() else { return };
    let mut raven = Raven::new(&net, &cfg).unwrap();
    raven.greeting();
    let mut talk = |q: &str| {
        let r = raven.reply(q).text;
        println!("You: {q}\nPrimogenitus: {r}");
        assert!(!r.contains("did not") && !r.contains("told me"), "{q} -> {r}");
        r
    };
    talk("Patrik");
    let r = talk("I'm fine, how about you?");
    assert!(r.matches('?').count() <= 1 && !r.contains("Ask me about something"), "{r}");
    let r = talk("What are you? A robot, an AI or a frozen neural net?");
    assert!(r.contains("chatbot") && !r.contains("Robots"), "{r}");
    let r = talk("Who made you?");
    assert!(r.contains("Joose") && !r.contains("robot"), "an old topic brought back: {r}");
    let r = talk("I like motors, not robots");
    assert!(r.contains("motor"), "{r}");
    let r = talk("I have plenty of motors");
    assert!(r.contains("motor"), "the thread lost: {r}");
}

#[test]
fn a_long_conversation_holds() {
    // the owner's report (2026-10-12): Primo did not keep longer conversations up.
    // Twenty turns that build on each other: what is told is kept, "he" is the
    // dog, a short follow-up goes on with the thread, and it is all recalled.
    let Some((net, cfg)) = setup() else { return };
    let mut raven = Raven::new(&net, &cfg).unwrap();
    let mut talk = |q: &str| {
        let r = raven.reply(q).text;
        println!("You: {q}\nPrimogenitus: {r}");
        r
    };
    talk("Hi");
    talk("My name is Joose");
    talk("I am a programmer");
    let r = talk("I have a dog called Musti");
    assert!(r.contains("Musti"), "the dog's name is taken: {r}");
    let r = talk("He is a husky");
    assert!(!r.contains("programmer"), "\"he\" is the dog, not the role: {r}");
    let r = talk("I like to go running with him");
    assert!(!r.contains("programmer"), "{r}");
    let r = talk("We ran ten kilometers yesterday");
    assert!(!r.contains("sad") && !r.contains("sorry"), "running is no loss: {r}");
    let r = talk("My knee hurts a bit now");
    assert!(!r.contains("hurted"), "{r}");
    let r = talk("Should I rest?");
    assert!(r.contains("knee") && !r.contains("did not"), "a choice that is theirs, the knee kept in mind: {r}");
    talk("I think I will rest tomorrow");
    let r = talk("What do you think about huskies?");
    assert!(r.to_lowercase().contains("husk") && !r.contains("did not"), "{r}");
    let r = talk("They need a lot of exercise");
    assert!(!r.contains("What would you like to talk about"), "the thread is kept: {r}");
    let r = talk("Tell me more");
    assert!(!r.contains("Ask me something first"), "more of the thread: {r}");
    let r = talk("Why?");
    assert!(r.starts_with("Because") || r.contains("do not know why"), "{r}");
    assert!(talk("What is my name?").contains("Joose"));
    let r = talk("What do I do for work?");
    assert!(r.contains("programmer"), "{r}");
    let r = talk("What is my dog called?");
    assert!(r.contains("Musti"), "{r}");
    let r = talk("What did I tell you about my knee?");
    assert!(r.contains("knee") && r.contains("hurt") && !r.contains("not told"), "{r}");
    let r = talk("What were we talking about?");
    assert!(r.contains("We were talking about"), "{r}");
}

#[test]
fn talking_about_women() {
    // the owner's conversation (2026-10-12): a topic asked for three ways, "women"
    // unknown, "do you have a man?", "do you like men?" after "not", a curse in a no
    let Some((net, cfg)) = setup() else { return };
    let mut raven = Raven::new(&net, &cfg).unwrap();
    let mut talk = |q: &str| {
        let r = raven.reply(q).text;
        println!("You: {q}\nPrimogenitus: {r}");
        r
    };
    talk("I am Joose, your creator");
    talk("I am fine, how about you?");
    for ask in ["I would like to talk about women", "But lets talk about women", "Can we talk about women"] {
        let r = talk(ask);
        assert!(r.to_lowercase().contains("woman") && !r.contains("did not") && !r.contains("do not know the word"), "{ask}: {r}");
        assert!(!r.contains("Do you have a woman"), "a person is not had: {r}");
    }
    let r = talk("I am a handsome man, 22 years old");
    assert!(!r.contains("Do you have a man") && !r.contains("Do you like men"), "{r}");
    let r = talk("I am a man heterosexual so not");
    assert!(!r.contains("Do you like men") && !r.contains("Do you have a man"), "{r}");
    let r = talk("No I fucking dont");
    assert!(!r.contains("did not"), "a curse changes nothing read: {r}");
}
