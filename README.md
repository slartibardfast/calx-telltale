# calx-telltale

A verified calculator for waits, interrupts-off windows, and the compositions
built out of them.

The name is the watchman's tell-tale clock: an instrument fitted because a
report cannot be trusted. It records whether the rounds were walked, and when.

## What it is

A calculator over a declared register of waits, windows and deadlines. The
arithmetic is formally verified. The units are typed. The provenance of every
value is carried through to every result that depends on it.

## What it is not

It is not a worst-case-execution-time tool. It models no caches and no
pipelines, and on a part that executes in place over serial flash it will not
convert a code span into a time. It computes in the units it is given, and it
refuses to invent a conversion it has no evidence for.

It is not a firmware prover either. It proves properties of the model, and the
model is a declaration written by a human or emitted by an adapter.

## Quantity, unit, provenance

Every number is a `Quantity`: an interval, a unit, and a provenance. The model
holds no bare integers.

Units are typed and closed. `Ticks` and `Cycles` name the clock they were
counted against rather than carrying a rate, so two differently-clocked
quantities cannot be added and a rate never enters the model as a bare integer.
`Iterations` and `BusReads` have no conversion to time at all. Supplying one
requires a declared `Conversion` carrying its own provenance, and every result
that used it inherits that provenance.

A provenance is `Derived`, `Extracted`, `Measured`, or `Assumed`. The governing
rule is that a result carries the weakest provenance of anything in its
derivation. A single assumed input makes the whole answer assumed, and it is
labelled that way wherever it is printed. This is the property the name
promises, and it is the reason the tool exists.

## Timing sources

The core knows no particular clock. An interval timer, an event timer and a
part-specific timer an adopter alone has are all declarations of the same shape,
referenced by identifier. A source declares a nominal frequency, a tolerance, a
counter width, what one read of it costs, and the span over which it can be
trusted.

**A frequency is an exact rational.** A common interval timer runs at `105/88`
MHz, and no integer count of hertz represents it. Storing a rounded frequency
would put an error into the one value that turns a count into a time, so nothing
here rounds. The figures usually quoted for such parts are themselves roundings,
and a base derived from them is a base for a clock that does not exist.

**A register composes in its own base.** Where several clocks are declared, the
tool derives the frequency in which every declared period is a whole number of
ticks, and composition there is exact integer arithmetic that introduces no
rounding at all. This is the idea behind Facebook's `flicks` applied at the scope
where the set of rates is actually finite, which is one register rather than the
world. Two clocks at `105/88` MHz and four times the colour burst share a base of
157500000 Hz, in which their periods are exactly 132 and 11 ticks.

The derivation is guarded, and the tool reports the span its base can represent.
A finer base buys resolution and spends range, so a register whose windows do not
fit is told at the point the base is derived.

**Clocks form a tree.** Real parts rarely have independent clocks. A core clock
comes off a synthesiser, the only counter runs at the core rate, a kernel tick
comes off a compare on that counter, and a sleep primitive rides the tick.
Declaring the tree buys three things: the derived rates are exact, a declared
frequency that disagrees with its parent and ratio is caught, and clocks sharing
a root are known to fail together rather than being worst-cased as though they
were independent.

**Trust is span-scoped, and it propagates.** A clock is no more trustworthy than
the clock it hangs off. Where a synthesiser is reconfigured while the part runs
on it, nothing beneath it can measure anything at all, and that includes the
reconfiguration. A conversion across such a span is refused rather than taken against a
stale rate, which is why a window measured there is honestly denominated in loop
passes.

## Delays

A delay is what binds a wait to a clock, and the register tells three kinds
apart by how tightly they are bound.

A timer-backed sleep names its clock, the granularity of one tick, and the
rounding it applies. A calibrated busy loop names the clock its calibration was
taken against, which matters more than it sounds: the loop reads no counter
while it runs, so it holds only while that clock stays at its nominal rate. A
bare spin names nothing, and its cost stays a count.

Rounding is part of the model because it can dominate. A sleep on a one
kilohertz tick that rounds up has a floor of one millisecond, so a caller asking
for a microsecond gets a millisecond and pays a thousand times what it asked
for. That is structural rather than a defect, and a model that ignored the
rounding would understate such a wait by three orders of magnitude.

## Composition

Sequential waits sum. Retry loops multiply. Branches take a maximum. A
short-circuit is subtler than any of those, and it is the operator that
hand-analysis gets wrong. Its worst case is

```
max( cost(guard fails), cost(guard succeeds) + cost(then) )
```

which is not a product. Where the guard fails the `then` never runs, so the
expensive case is the one where the guard succeeds expensively. A tool without
this operator will report the cheap branch and call it the worst.

## Attainment

A wait costs what it costs because of something outside the part: how long the
peripheral takes to answer. That is one parameter, every wait in a composition
sees the same world, and a cost is therefore a function of it rather than a
constant.

Evaluation at one latency is cheap. The question worth asking is which latency
is worst, and the answer is not reliably at either end. A guard that fails costs
its budget and unwinds. A guard that answers on its last permitted attempt costs
almost as much and lets everything above it run, so the expensive case can sit
one step inside the boundary where a sweep of the extremes will not find it.

So evaluation returns a cost together with the latency attaining it, and says
whether that latency was interior. The search is bounded by the widest budget in
the composition rather than by any cost, which keeps it small even where the
costs run to millions: past that bound every wait has already given up.

Monotonicity is never assumed. Where the tool can establish it, it reports it as
a proved property. Where it cannot, it searches the domain.

## Proof obligations

The arithmetic is verified with [Kani](https://model-checking.github.io/kani/).

| | |
|---|---|
| termination | every wait's measure is well-founded and strictly decreasing |
| counter fit | the declared budget fits the counter type that holds it |
| accumulator fit | every reachable composition fits its accumulator type |
| attainment | the reported maximum is a true maximum over the declared domain |
| unit soundness | combining units requires a declared conversion |
| provenance monotonicity | a result is at most as strong as its weakest input |
| interval soundness | the result interval is conservative |

Units, provenance and refusal are quantified over everything their types admit,
and they discharge in seconds. The obligations that turn on multiplication are
quantified over boundary values instead: the identities, the values either side
of the point where a product stops fitting the stored width, and the extremes.
Multiplication over the whole domain defeats a solver that bit-blasts it, at any
stored width and with either a SAT or an SMT back end, and the property itself
is textbook interval arithmetic rather than anything this crate invents. What
the proof really checks is the transcription, meaning whether the endpoints are
paired correctly and the refusal fires, and that shows at the boundaries.

Termination and attainment are quantified over a whole declared domain, so they
are proved with loop contracts rather than by unrolling a budget that runs to
thousands of iterations.

Every stored count is a `u128` behind a single alias, and arithmetic refuses
rather than wraps. The width is deliberately generous: the register composes in
the finest base it can derive, and a narrow store would put a ceiling on how
fine that base is allowed to be.

## Subcommands

Following the `calx-mill` idiom.

| | |
|---|---|
| `census` | inventory waits and windows from an ELF; emit a skeleton register |
| `check` | run the proof obligations against a register |
| `project` | worst-case cost of each declared composition |
| `attain` | report the witness: where each maximum occurs |
| `deadline` | compare projections against declared deadlines, by armed interval |
| `overrun` | whether a blackout delivers more arrivals than a buffer holds |
| `diff` | register against register, for regression across builds |

The register itself is a hand-authorable, diffable file declaring waits,
windows, deadlines, compositions and conversions. An ELF adapter emits a
skeleton register with `Extracted` provenance and blanks wherever a human must
still supply a budget or a measure.

## Relationship to calx-mill

[calx-mill](https://github.com/slartibardfast/calx-mill) is a sibling crate, not
a parent. It computes steady-state throughput and overrun. Its arithmetic
assumes abundant independent work, and asks which resource saturates first.

This problem inverts every axis of that one. There is one work unit where
calx-mill has many. Concurrency is 1 by construction, on a single core with
interrupts off. The binding constraint is an external agent that may never
answer, where calx-mill's is a pipe. The arithmetic is max-plus over dependency
chains, where calx-mill's is min-of-limits over ratios.

What transfers is the design: declare the axes, keep the arithmetic universal
over the declaration, verify it, and let adapters populate it from tool output.
Whether the two can share a `Substrate` abstraction is an open question, and it
turns on whether its axes are hard-wired to registers and pipes.

## Interrupts

A register measures how long interrupts are off, which on its own says nothing
about whether any interrupt missed anything. An interrupt therefore declares how
often it can arrive, what latency it can absorb, and how deep the buffer behind
it is.

Latency asks whether the blackout outlasts the deadline. Overrun asks whether
more arrivals land during the blackout than the buffer holds, and it is the
question that bites: a handler that drops entries when its ring fills and only
logs the fact fails without ever missing a latency figure anyone was watching.

Either verdict can be withheld, and a withheld verdict is a verdict rather than
an error. A blackout counted in loop passes, because its clock was being
reconfigured while it ran, cannot be compared against a deadline in nanoseconds.
The tool says which piece is missing and declines, and a run reports how many
comparisons it withheld so that an exclusion never reads as a clean sweep.

## Keeping up

An interrupt also declares how long its handler runs, what priority it holds, and
what a dropped arrival costs. Priority is numbered the way the hardware numbers
it, so a lower number preempts a higher one.

Utilisation is the first-order check: `Σ C/T` over the declared set, carried in
parts per million so it stays exact. A set demanding more time than exists is
reported as such without iterating, because no ordering rescues it.

Response time is then the fixed point of

```
R = C + B + Σ over higher priorities of ceil(R / T) * C
```

which is the standard fixed-priority analysis rather than anything invented here.
The blocking term is the interrupts-off window this project already computes, so
the two halves of the tool meet at that symbol. Every verdict carries its margin,
since a response inside its deadline by a hair and one inside by an order of
magnitude are different facts.

The handler cost is **declared, not derived**. This tool computes no worst-case
execution time. It also declines to pretend the quantity is absent, because a
model missing the term cannot say whether anything keeps up at all, so it takes
the number it is given and carries the provenance. A guessed cost makes every
verdict resting on it a guess.

## What it does not model

The exclusions are enumerated in the crate and stated on every run, because an
exclusion that is not stated reads as coverage. They include execute-in-place
instruction fetch, hangs that are not loops, unresolved indirect calls, windows
that cross stack frames, correlated clock failure, nested budgets, release
jitter, nesting within a priority level, and worst-case execution time itself.

## Status

The verified core is built: quantities, exact rates, the register base, the tree
of derived clocks, the delay forms, and the interrupt verdicts. The register
file format and the command line are still ahead. The milestones, decisions and
verification lanes are governed by the host meta-repo at
[agentic-calx-telltale](https://github.com/slartibardfast/agentic-calx-telltale).

## Licence

Released into the public domain under the [Unlicense](LICENSE).
