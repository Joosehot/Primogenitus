# Primogenitus

**A conversational agent with no machine learning in it.** No neural network,
no training data, no GPU. Its knowledge is a word network built in about two
seconds from a public dictionary (WordNet 3.1) and then frozen. Its sentences
come from explicit grammar rules, scored by explicit judges and checked by a
parser. Every reply can explain itself, and the same input always gives the
same reply.

It calls itself Primogenitus, Primo for short.

| | |
|---|---|
| Language | Rust, about 8,300 lines, no ML dependencies |
| Knowledge | 272,137 nodes and 1.45 million lines, frozen to a 43 MB file |
| Build | 2.2 s from the dictionary (plus a one-time download) |
| Reply time | typically 1–50 ms on one CPU core |
| Trained parameters | 0 |
| Tests | 25, including real conversations replayed turn by turn |

## What it looks like

```
Primogenitus: Hello. I am Primogenitus, Primo for short. What is your name?
You: Hi, my name is Joose
Primogenitus: Hello, Joose. It is good to meet you. How are you today?
You: I have a dog called Musti
Primogenitus: Musti, what a nice name, Joose. Does Musti bark?
You: He is a husky
Primogenitus: Your dog is a husky. Is Musti noisy?
You: My knee hurts a bit
Primogenitus: I am sorry your knee is hurt.
You: Should I rest?
Primogenitus: That is your choice to make. You know your knee best. How does it feel now?
You: What do dogs eat?
Primogenitus: Dogs eat flesh. Does your knee buckle?
You: Why?
Primogenitus: Because my word network says so: carnivore -eats-> flesh.
You: What is my dog called?
Primogenitus: Your dog is called Musti.
```

With `--explain`, every reply shows what it stands on: the lines of the
network behind each fact, how many candidate replies were weighed, how many
broke a rule, and what the chosen one was chosen for:

```
Primogenitus: Dogs eat flesh. Does your knee buckle?
   line: carnivore -eats-> flesh
   line: dog -is a-> canid -is a-> carnivore
   line: knee -does-> buckle
   line: mblm -asked-> Does your knee buckle? (p 0.334)
   line: raven -weighed-> 25 replies (12 broke its rules) -chose for-> answers the question,
         takes up what you said, remembers what you told, asks back, keeps the thread
   (0.026 s)
```

## How it works

```
                 build once (2 s)                              every turn (ms)
 WordNet 3.1 ─────────────────────▶ frozen word network ◀──────────────┐
 (words, senses,   build.rs          network.bin: nodes = words and     │
  definitions,     morph.rs          their forms; lines = is-a, has,    │
  example          parser.rs         does, eats, lives in, used for ... │
  sentences)                                                            │
                                                                        │
 user text ─▶ Raven: read the turn ─▶ build candidate replies ─▶ rules ─▶ judges ─▶ reply + why
              (turn.rs)               (mod.rs)                  (hard   (scored,
              name, feeling,          opener + body + question  filter)  best wins)
              question, fact,         body: Primo answers from
              pronoun, topic          the network (chat.rs)
                   │                       │
                   ▼                       ▼
              memory.rs: the whole conversation, compacted when long
```

There are three layers: the **network** (what it knows), the **motor** (how
it says a sentence), and **Raven** (how it talks to a person).

### 1. The frozen word network

`wordnet.rs`, `build.rs`, `morph.rs`, `network.rs`

The build downloads WordNet 3.1 once and turns it into a graph.

- **Nodes** are words and every form of them as its own node: `dog`, `dogs`;
  `bark`, `barks`, `barked`, `barking`. Each node carries what the grammar
  needs:
  - part of speech;
  - familiarity (WordNet's tag counts);
  - a verb's sentence frames;
  - whether a noun is animate or uncountable;
  - whether the form is sure (irregular forms come from WordNet's exception
    lists and a few rules).
- **Lines** carry the meaning. They come from three places:
  - the dictionary's own relations: is a, has part, opposite, similar;
  - its 48,522 example sentences, parsed ("the dog barked" gives
    *dog -does-> bark*);
  - its 155,000 definitions, read for what a thing eats, where it lives,
    what it is used for and what it is made of ("a flesh-eating mammal" gives
    *-eats-> flesh*).
- **Senses:** every line knows which sense of a word it comes from and which
  sense it lands in. A walk of two steps therefore never jumps meanings:
  wolf → canine the animal, never canine the tooth.

After the build the graph is serialized to `data/network.bin` and never
changes. Loading it takes about 0.2 s.

### 2. The motor: making a sentence

`rules/*.rs`, `search.rs`, `rules/judges.rs`, `parser.rs`, `lib.rs`

A sentence is never retrieved or predicted. It is assembled, scored and
checked:

1. **Meaning:** activation spreads from the topic along the lines (`search.rs`).
2. **Rules** propose candidate sentences, one sentence pattern per file:
   `is_a` ("Dogs are canines."), `has_part`, `does`, `does_to` ("Dogs chase
   rabbits."), `is_adj`. A rule may only say what a line says.
3. **Beam search** fills the pattern: heads first, then subject form and
   tense, then the object.
4. **Judges** score the whole sentence (`rules/judges.rs`). The grammar is
   built into them: verb frames, determiner and number, agreement, a/an,
   adjective position, no word twice. Some judges only score: animacy, role.
5. **Check:** the winner is parsed again under a small grammar
   (`S -> NP VP .`). A sentence that does not parse is never said.

Seeds perturb the scores so the motor can give several different sentences
for one topic. The judges still pick, so the output is deterministic for a
given seed.

### 3. Answering questions

`question.rs`, `chat.rs`

`question.rs` reads a question whole, word by word, into a typed question:
- what is / what does X do / have / eat;
- where does X live;
- what is X used for;
- is X a Y, does X have a Y, can X VERB;
- the capital of X, define X;
- pronouns for what was talked about last.

A word left over means "not understood", never a guess. An unknown word gets
a "did you mean".

`chat.rs` answers:
- yes/no questions ("Is a dog an animal?", "Does a dog have a tail?") by
  walking the lines, a part or a deed inherited from a kind;
- what-questions from the lines, inherited down the is-a chain ("What do
  dogs eat?": dog → canid → carnivore -eats-> flesh);
- open questions with the motor, limited to the fitting sentence patterns.

When no line says something, it says so: "Not that I know: no line in my
network says that whales are fishes."

### 4. Raven: the conversation layer

`raven/turn.rs`, `raven/memory.rs`, `raven/feeling.rs`, `raven/judges.rs`, `raven/mod.rs`

The network knows things about the world; Raven decides how to talk to a
person.

- **Reading a turn** (`turn.rs`): clause by clause, in the light of what
  Raven asked last. It reads:
  - a name, a feeling, an age, what someone is, likes, has, or what
    happened to something of theirs;
  - a pet's name, an opinion;
  - "should I…?", "why?", "tell me more", "what were we talking about?";
  - and the question forms above.

  Pronouns ("he", "they") point to the thing talked about last, and a role
  ("I am a programmer") is never taken for "he".
- **Context:** what the last question waits for (a name, a feeling, an age),
  the referent of pronouns, and the *thread*: the topic the person brought up
  last, kept for a few turns so follow-ups stay on it.
- **Memory** (`memory.rs`): the whole conversation is kept as exchanges, each
  with the facts read from it. When the text passes a limit, the oldest
  exchanges are compacted: their words are dropped and their facts kept. So
  "What is my dog called?" and "What did I tell you about my knee?" work at
  any distance.
- **Feelings** (`feeling.rs`): how a word feels is read from the network. A
  few anchor words in `rules.toml` set the scale; similar words take their
  sign and opposites flip it.
- **Candidate replies** (`mod.rs`): every reply is at most one opener, one
  body (an answer or a fact) and one question. All combinations are built.
  Follow-up questions come from MBLM (below) rather than a fixed list.
- **System rules:** 13 rules in `rules.toml` (`[raven.system]`) act as a
  hard filter, not as advice:
  - safety first, with a crisis reply;
  - only what a line says about the world;
  - feelings before facts;
  - at most one question, never more than 45 words;
  - no repeats, no medical, legal or financial advice;
  - forget everything on request.

  A candidate that breaks a rule is never said, and `tests/system.rs` holds
  each rule.
- **Humane judges** (`judges.rs`) score what is left:
  - feelings first (no facts while someone is sad);
  - answer the question;
  - take up what was said;
  - remember what was told;
  - ask back, but not endlessly;
  - keep the thread;
  - do not repeat yourself.

  The best candidate is said. Its reasons become the "chose for" line.

### 5. MBLM: a motor-based language model

`mblm.rs`

The same machinery also works as a language model, with nothing trained. For
any text it gives a probability distribution over the next word:
- **the grammar:** a word-level automaton of the motor's grammar decides
  which words may come next at all; everything else has probability 0;
- **the meaning:** the network's lines decide which of those words fit the
  topic;
- **the probabilities:** a softmax turns the scores into probabilities.

```
$ primogenitus next "A dog is a"
    0.6693  domestic animal   dog -is a-> domestic animal
    0.3015  beast             dog -is a-> domestic animal -is a-> beast
    0.0087  canid             dog -is a-> canid

$ primogenitus complete "A dog is"
A dog is a domestic animal. It snatches a bone. It licks a hand. It chases a rabbit.
```

Raven uses it to write its follow-up questions ("Does Musti bark?"), with
the probability shown in the explanation.

## Design principles

- **Nothing is trained.** All behaviour comes from the dictionary, the
  grammar and explicit rules.
- **Every number is in `rules.toml`:** weights, limits, the reply texts and
  the system rules. Behaviour can be tuned without touching code.
- **One rule per file**, and every judge is named in the explanation.
- **Deterministic:** the same conversation gives the same replies, so it can
  be tested exactly.
- **Explainable:** every fact carries the path of lines it stands on.
- **Honest about not knowing:** an unread sentence or an unknown fact is said
  to be unknown, never guessed.

## Trade-offs and limits

This is not a large language model and does not try to be one. What it
gains:
- predictability and explanations;
- a tiny footprint, no GPU, millisecond replies;
- behaviour a designer can read and edit.

What it gives up is breadth and fluency:
- **Knowledge is dictionary-deep.** It knows what a woman or a husky *is*,
  not much more, so a long talk about one topic soon repeats the definition.
- **Reading is pattern-based.** A sentence shape it has no reader for is
  said to be not understood. Each real conversation that went wrong has been
  turned into a test, so the coverage grows.
- **Some generated sentences are odd** because a dictionary example was
  misread or is odd itself ("Is Musti cowardly?" comes from WordNet listing
  "cowardly" with dogs). They are grammatical, since the parser checks them,
  but not always natural.

## Running it

Rust (stable). The first build downloads WordNet 3.1.

```
cargo build --release
target/release/primogenitus build              # download WordNet once, build and freeze the network
target/release/primogenitus serve              # chat page at http://127.0.0.1:7878/
target/release/primogenitus chat --explain     # chat in the terminal, with reasons
target/release/primogenitus ask "What do dogs eat?"
target/release/primogenitus read "I have a dog called Musti"   # how Raven reads a sentence
target/release/primogenitus next "A dog is a"  # MBLM: next-word distribution
cargo test --release                           # needs the built network
```

## Repository layout

```
src/wordnet.rs build.rs morph.rs network.rs   the frozen word network
src/lexicon.rs parser.rs                      closed words, a/an, the sentence parser (the check)
src/rules/                                    one sentence pattern per file, and the grammar judges
src/search.rs lib.rs input.rs                 spreading, beam search, seeds
src/question.rs chat.rs                       reading and answering questions
src/raven/                                    the conversation layer
src/mblm.rs                                   the motor-based language model
src/serve.rs ui/index.html                    the chat page
rules.toml                                    every number, every reply text, the system rules
tests/                                        sentences, chat, knowledge, MBLM, Raven conversations, system rules
```

## Tests

`cargo test --release` (25 tests):
- **Sentences:** 50 topics generated twice must give identical sentences that
  all parse, and the check must refuse bad grammar ("Dogs barks.",
  "An dog barks.").
- **Chat:** a whole question conversation, with known facts confirmed and
  made-up ones refused.
- **Raven:** real conversations replayed turn by turn, each with assertions
  on the replies: a loss, a breakup, a long twenty-turn conversation, a topic
  change.
- **System rules:** every rule of `[raven.system]`.

## Credits

WordNet 3.1 is © Princeton University. It is downloaded at build time under
its own license and is not part of this repository.

MIT License. See [LICENSE](LICENSE).
