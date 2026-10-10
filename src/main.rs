//! primogenitus build                 build the word network once and freeze it
//! primogenitus chat [--explain]      talk (one question per line)
//! primogenitus ask "question"        one question, one answer
//! primogenitus serve [--port 7878]   the chat page: http://127.0.0.1:7878/
//! primogenitus complete "prompt" [--sentences N] [--explain]   MBLM: the prompt continued
//! primogenitus next "prompt" [--top K]                         MBLM: the next word's distribution
//! primogenitus score "text"                                    MBLM: every word's probability, perplexity, coverage
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
    let port: u16 = take("--port").map(|s| s.parse()).transpose()?.unwrap_or(7878);
    let sentences: Option<usize> = take("--sentences").map(|s| s.parse()).transpose()?;
    let top: usize = take("--top").map(|s| s.parse()).transpose()?.unwrap_or(10);
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
        bail!("usage: primogenitus build | chat | ask \"question\" | serve [--port N] | <noun> [--seeds N] [--explain]");
    }
    if matches!(args[0].as_str(), "complete" | "next" | "score") {
        let net = Network::load(&frozen)?;
        let m = primogenitus::mblm::Mblm::new(&net, &cfg);
        let text = args[1..].join(" ");
        let t = Instant::now();
        match args[0].as_str() {
            "complete" => match m.complete(&text, sentences.unwrap_or(cfg.mblm.sentences)) {
                Ok((more, toks)) => {
                    println!("{text}\x1b[1m{more}\x1b[0m");
                    if explain {
                        for tk in &toks {
                            println!("  {:>8.4}  {:<14} {}", tk.p, tk.text, tk.why);
                        }
                    }
                    let lp: f64 = toks.iter().map(|t| t.p.ln()).sum();
                    eprintln!("({} words, mean log-probability {:.3}, {:.3} s)", toks.len(), lp / toks.len().max(1) as f64, t.elapsed().as_secs_f64());
                }
                Err(e) => println!("{e}"),
            },
            "next" => {
                let Some(topic) = m.topic_of(&text) else { bail!("no noun in the prompt that I know well enough") };
                let mut p = m.para(topic);
                let (read, st) = m.read(&text, &mut p);
                let lost = read.iter().any(|(_, pr, _)| pr.is_none());
                let st = if lost { primogenitus::mblm::St::Start } else { st };
                if lost {
                    println!("(the motor could not follow the prompt to its end; a new sentence about \"{}\")", net.node(topic).text);
                }
                for tk in m.next(st, &p).into_iter().take(top) {
                    println!("  {:>8.4}  {:<14} {}", tk.p, tk.text, tk.why);
                }
            }
            _ => match m.score(&text) {
                Some(s) => {
                    println!("topic: {}", s.topic);
                    for (w, pr, alts) in &s.words {
                        let alts: Vec<String> = alts.iter().map(|(a, p)| format!("{a} {p:.3}")).collect();
                        match pr {
                            Some(p) => println!("  {:>8.4}  {:<14} (likeliest: {})", p, w, alts.join(", ")),
                            None => println!("  {:>8}  {:<14} (no place for it in the motor)", "-", w),
                        }
                    }
                    println!("perplexity {:.2} over the words inside the motor; coverage {:.0}%", s.perplexity, s.coverage * 100.0);
                }
                None => println!("I found no noun in the text that I know well enough to read it by."),
            },
        }
        return Ok(());
    }
    if args[0] == "serve" {
        let net = Network::load(&frozen)?;
        return primogenitus::serve::serve(&net, &cfg, port);
    }
    if matches!(args[0].as_str(), "chat" | "ask") {
        let net = Network::load(&frozen)?;
        // "ask" is Primo alone (one question, one fact); "chat" talks through Raven
        let mut chat = primogenitus::chat::Chat::new(&net, &cfg);
        let mut raven = primogenitus::raven::Raven::new(&net, &cfg)?;
        let hello = raven.greeting();
        let is_ask = args[0] == "ask";
        let mut answer = |q: &str| {
            let t = Instant::now();
            let r = if is_ask { chat.reply(q) } else { raven.reply(q) };
            println!("Primogenitus: {}", r.text);
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
        println!("Primogenitus: {hello}");
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
