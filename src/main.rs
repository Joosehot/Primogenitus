//! primogenitus build                 build the word network once and freeze it
//! primogenitus chat [--explain]      talk (one question per line)
//! primogenitus ask "question"        one question, one answer
//! primogenitus <noun> [--seeds N] [--explain] [--rules rules.toml]

use anyhow::{bail, Result};
use primogenitus::config::Config;
use primogenitus::input::Input;
use primogenitus::network::Network;
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut take = |flag: &str| -> Option<String> {
        let k = args.iter().position(|a| a == flag)?;
        args.remove(k);
        (k < args.len()).then(|| args.remove(k))
    };
    let rules = take("--rules").map(PathBuf::from).unwrap_or_else(|| root.join("rules.toml"));
    let seeds: Option<usize> = take("--seeds").map(|s| s.parse()).transpose()?;
    let explain = match args.iter().position(|a| a == "--explain") {
        Some(k) => {
            args.remove(k);
            true
        }
        None => false,
    };
    let cfg = Config::load(&rules)?;
    let frozen = root.join("data").join("network.bin");

    if args.first().map(|s| s.as_str()) == Some("build") {
        let t = Instant::now();
        let (net, report) = primogenitus::build::build(&root, &cfg, &mut |m| eprintln!("  {m}"))?;
        for (what, sec) in &report.steps {
            println!("  {sec:7.2} s  {what}");
        }
        let tf = Instant::now();
        net.save(&frozen)?;
        let size = std::fs::metadata(&frozen)?.len();
        println!("  {:7.2} s  frozen to {} ({:.1} MB)", tf.elapsed().as_secs_f64(), frozen.display(), size as f64 / 1e6);
        println!(
            "built in {:.1} s: {} nodes ({} words, {} variations), {} lines ({} from {} parsed example sentences)",
            t.elapsed().as_secs_f64(),
            report.nodes,
            report.base,
            report.nodes - report.base,
            report.lines,
            report.example_lines,
            report.examples
        );
        return Ok(());
    }
    if args.is_empty() {
        bail!("usage: primogenitus build | chat | ask \"question\" | <noun> [--seeds N] [--explain]");
    }
    if matches!(args[0].as_str(), "chat" | "ask") {
        let net = Network::load(&frozen)?;
        let mut chat = primogenitus::chat::Chat::new(&net, &cfg);
        let mut answer = |q: &str| {
            let t = Instant::now();
            let r = chat.reply(q);
            println!("Primo: {}", r.text);
            if explain {
                for w in &r.why {
                    println!("       line: {w}");
                }
                println!("       ({:.3} s)", t.elapsed().as_secs_f64());
            }
        };
        if args[0] == "ask" {
            answer(&args[1..].join(" "));
            return Ok(());
        }
        println!("Primo: {}", cfg.chat.greeting);
        let stdin = std::io::stdin();
        let mut line = String::new();
        loop {
            use std::io::Write;
            print!("You: ");
            std::io::stdout().flush()?;
            line.clear();
            if stdin.read_line(&mut line)? == 0 {
                break;
            }
            let q = line.trim();
            if q.is_empty() {
                continue;
            }
            answer(q);
            if matches!(primogenitus::question::read(&net, q), primogenitus::question::Question::Goodbye) {
                break;
            }
        }
        return Ok(());
    }

    let t = Instant::now();
    let net = Network::load(&frozen)?;
    let load = t.elapsed().as_secs_f64();
    let input = Input::from_args(&args);
    let topic = input.start(&net)?;
    let t = Instant::now();
    let (seeds, winner) = primogenitus::generate(&net, &cfg, topic, seeds.unwrap_or(cfg.search.seeds));
    let gen = t.elapsed().as_secs_f64();
    println!("input: {}", input.describe());
    println!("seeds (the judges' score; the check parses each under the grammar):");
    for (k, s) in seeds.iter().enumerate() {
        let mark = if Some(k) == winner { "->" } else { "  " };
        let check = match &s.check {
            Ok(p) => format!("parses: {p}"),
            Err(e) => format!("FAILS the check: {e}"),
        };
        println!("{mark} seed {}  {:+6.2}  {:<40} [{}; {}]", s.seed, s.score, s.text, s.cand.pattern, check);
        if explain {
            for (what, v) in &s.cand.parts {
                println!("        {v:+6.2}  {what}");
            }
            for w in &s.cand.why {
                println!("        line: {w}");
            }
        }
    }
    match winner {
        Some(k) => println!("\n{}", seeds[k].text),
        None => println!("\nno seed passed the check: nothing is said"),
    }
    eprintln!("(network loaded in {load:.2} s, {} seeds generated and judged in {:.3} s)", seeds.len(), gen);
    Ok(())
}
