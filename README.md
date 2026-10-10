# Primogenitus

A chatbot that is not trained. A frozen word network is built once from a
downloaded dictionary (WordNet 3.1), sentences come from the HotMotor
pipeline (lexicon, parser, rules, beam search with judges, checks), and
Raven, its conversation layer, makes the talk humane: it hears feelings,
remembers the person and keeps the thread. Nothing is trained and nothing
updates. It calls itself Primogenitus, Primo for short.

## Getting started

Rust (stable) and an internet connection for the one-time dictionary download.

```
cargo build --release
target/release/primogenitus build      # download WordNet 3.1 once, build and freeze the network (~6 s)
target/release/primogenitus serve      # the chat page: http://127.0.0.1:7878/
target/release/primogenitus chat       # or talk in the terminal
target/release/primogenitus ask "What is a dog?"   # Primo alone: one question, one fact
cargo test --release                   # the proof (needs the built network)
```

WordNet 3.1 is (c) Princeton University and is downloaded at build time under
its own license; it is not part of this repository.

## The word network and the sentence motor

```
$ primogenitus build
     4.72 s  the dictionary downloaded and unpacked (or found)
     ...
built in 1.5 s: 271207 nodes (155467 words, 115740 variations), 1411759 lines (29295 from 48530 parsed example sentences)

$ primogenitus dog
input: the topic "dog" (one noun; the sentence is about it)
seeds (the judges' score; the check parses each under the grammar):
-> seed 0   +3.74  Dogs snatch a bone.          [does_to; parses: NP Verb NP]
   seed 3   +3.59  Dogs lick a hand.            [does_to; parses: NP Verb NP]
   seed 5   +3.18  Dogs chase a rabbit.         [does_to; parses: NP Verb NP]
   ...
Dogs snatch a bone.
```

`--explain` shows every score term and the line each sentence stands on.

## Raven: the conversation, made humane

`primogenitus chat` and the page talk through Raven. Primo knows what is in
the world; Raven decides how to say it to a person.

```
You: Hi, my name is Joose
Primogenitus: Hello, Joose. It is good to meet you. How are you today?
You: My dog died
Primogenitus: That is very sad. I am sorry, Joose. What was your dog like?
You: Her name was Musti
Primogenitus: You must miss Musti. What do you remember most about Musti?
You: I have a cat too
Primogenitus: Oh, you have a cat, Joose. What is your cat like?
You: Is a cat an animal?
Primogenitus: Yes. Cats are animals. Is that true of your cat too?
```

- `raven/turn.rs` reads a turn whole, clause by clause: questions go to
  Primo's parser; statements are read too (a name, a feeling, what they are,
  like, dislike, have, what happened to something of theirs, a pet's name,
  an opinion, yes / no / thanks / haha).
- `raven/feeling.rs` tells how a word feels from the network: an anchor word
  in rules.toml, a similar one, an opposite (flipped), or the feeling it names
  up the is-a lines.
- `raven/mod.rs` remembers the person (name, mood, likes, what they have,
  what they lost, their pets' names) and makes every reply of at most one
  opener, one body (Primo's answer or a fact) and one question back.
- `raven/judges.rs`, the humane judges, score each whole reply: feelings
  first (sympathy before anything, no facts while someone hurts; joy shared),
  answers, listens (takes up what was said, greets a name, no fact nobody
  asked for), remembers (brings back what was told, the name now and then),
  keeps going (asks back, but not a fourth question in a row, not at
  goodbye), natural (nothing twice, not the same opening, not too long).
- The search is the old Raven's: every reply weighed, MAX picks, the judges
  as the evaluator instead of a trained model. Raven's words are in
  `[raven.say]`, the weights in `[raven.judges]`, the feeling anchors in
  `[raven.feeling]`. Nothing is trained.

`primogenitus ask "..."` is Primo alone (one question, one fact).

## MBLM: a motor based language model

Primo as a language model, word by word, with nothing trained. For any text
so far it gives a distribution over the next word, and continues text by
choosing from it:

- **the grammar**: a word-level automaton of Primo's sentence grammar says
  which words may come next at all (a determiner, an adjective, the noun, a
  verb in the form that agrees, "is", "has", a full stop); everything else
  has probability 0;
- **the meaning**: the frozen network's lines say which of those words fit
  the topic (what it is, does, has, is like; what that verb is done to);
- **the judges**: a word said already in the paragraph, or its opposite, is
  held back; a sentence is chosen by its words' mean log-probability, so a
  short one is not better for being short.

The scores become probabilities by a softmax (`[mblm]` in rules.toml), each
with the line it stands on.

```
$ primogenitus complete "A dog is"
A dog is a domestic animal. It snatches a bone. It licks a hand. It chases a rabbit.

$ primogenitus next "A dog is a"
    0.6693  domestic animal dog -is a-> domestic animal
    0.3015  beast          dog -is a-> domestic animal -is a-> beast
    0.0087  canid          dog -is a-> canid
    ...

$ primogenitus score "Dogs bark. They chase rabbits."
perplexity 2.97 over the words inside the motor; coverage 100%
```

`score` gives every word its probability, the perplexity over the words the
motor has a place for, and the coverage: how much of the text lies inside
the motor at all (the same words scrambled: 0%). It is said plainly: words
outside the motor's grammar and lines are not given a made-up probability.

## The chatbot (Primo alone)

```
$ primogenitus chat
Primogenitus: Hello. Ask me about a thing, an animal or a person: ...
You: Is a dog an animal?
Primogenitus: Yes. Dogs are animals.
You: Does it have a tail?
Primogenitus: Yes. Dogs have tails.
You: What does a dog do?
Primogenitus: Dogs snatch a bone.
You: What else?
Primogenitus: Dogs lick a hand.
You: Is a whale a fish?
Primogenitus: Not that I know: no line in my network says that whales are fishes.
```

`primogenitus ask "question"` answers one question; `--explain` shows the
lines each answer stands on.

- `question.rs` reads the question whole, word for word: what is / who is /
  what does ... do / have / what is ... like / what do dogs eat / what eats
  fish / which animals swim / what animals have wings / what kind of animal is
  a dog / how big is a whale / is X a Y / is X ADJ / does X have a Y / can X
  VERB / tell me about X / who are you / what else, with pronouns (it, they)
  for what was talked about. Where, when, why and how are answered plainly:
  the network has no lines for places, times, reasons or ways yet. A word that
  reads two ways ("wings": the plural, or the pilot's badge) is read the way
  whose number is sure and fits its determiner. A word left over means not understood, never a
  guess; an unknown word gets "did you mean".
- `chat.rs` answers: open questions by the generator limited to the fitting
  patterns, yes/no questions by walking the lines (dog -> domestic animal ->
  animal, never jumping senses; a part or a deed inherited from a kind). The
  conversation remembers its topic and what was said, so "what else" gives
  something new until "That is all I know".
- Every sentence about the world passes the grammar judges and the check.
  No line means "Not that I know", never a guess.

## The frozen word network

- **Source**: WordNet 3.1, downloaded once into `data/wordnet/`.
- **Nodes**: every word, and every variation of a word as its own node
  (`dogs`, `barks`, `barked`, `barking`), with a line to its base. A node
  carries its familiarity (WordNet's tag counts), a verb's sentence frames
  (from its most common senses), a noun's animacy, an adjective's position
  marker, whether a noun may be uncountable, and whether its form is sure.
- **Lines** carry the meaning: the dictionary's own (same as, is a, has part,
  opposite, similar, derived, ...), and the ones the parser reads from the
  dictionary's example sentences ("the dog barked": *dog does bark*; "toast
  the bread": *toast is done to bread*). Every line knows which sense of its
  word it comes from and which sense of the target it lands in, so a walk of
  two steps never jumps senses (wolf -> canine the animal, not the tooth).
- **Frozen** to `data/network.bin`; never updated after the build.

## Generation

| Step | File | What it does |
|---|---|---|
| input | `input.rs` | one noun (the simplest option; swap here) |
| lexicon | `lexicon.rs` | closed words, the a/an rule |
| meaning | `search.rs` | spreads from the topic along the lines |
| rules | `rules/*.rs` | one sentence pattern per file: `is_a`, `has_part`, `does`, `does_to`, `is_adj`; each says only what a line says |
| beam | `search.rs` | heads, then subject and tense, then object |
| judges | `rules/judges.rs` | the grammar, built in: frames, determiner and number, agreement, sure forms only, adjective place, no word twice, a/an; scoring only: animacy, role |
| seeds | `lib.rs` | each seed shakes the scores and gives one sentence; the judges pick |
| check | `parser.rs` | every sentence is parsed again under `S -> NP VP .`; one that fails is never said |

Every number is in `rules.toml`.

## The proof

`cargo test`: the spelling rules, the article rule, and on the frozen network
(`tests/sentences.rs`) 50 topics, each generated twice: the same sentences
both times, every seed's sentence parses, and the check refuses bad grammar
("Dogs barks.", "An dog barks.", "A dog are a canine."); `tests/chat.rs`
holds a whole conversation (known facts yes, made-up ones not, pronouns,
"what else" to its end, every sentence checked); `tests/raven.rs` holds
real conversations as they went, every turn of them read and answered in
context (a name, a loss, a breakup, a sad day).

## Known limits

The example-sentence parser is simple; a misread example gives an odd but
grammatical sentence ("Birds land a lot."). Some odd ones are the
dictionary's own ("Houses bearded the top of the heights").

## License

MIT. See [LICENSE](LICENSE).
