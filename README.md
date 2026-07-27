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

Units are typed and closed. `Ticks` and `Cycles` carry their rate, so two
differently-clocked quantities cannot be confused for one another.
`Iterations` and `BusReads` have no conversion to time at all. Supplying one
requires a declared `Conversion` carrying its own provenance, and every result
that used it inherits that provenance.

A provenance is `Derived`, `Extracted`, `Measured`, or `Assumed`. The governing
rule is that a result carries the weakest provenance of anything in its
derivation. A single assumed input makes the whole answer assumed, and it is
labelled that way wherever it is printed. This is the property the name
promises, and it is the reason the tool exists.

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

Evaluation returns a cost together with the input at which that cost is
attained. The witness is the point in the latency domain that produces the
maximum, and reporting it matters: a maximum can sit one unit inside a budget
rather than out at the boundary, where a sweep of the extremes would miss it. A
tool that returned only a number would conceal exactly the thing worth knowing.

Monotonicity is never assumed. Where the tool can establish it, it reports it as
a proved property. Where it cannot, it searches the domain.

## Proof obligations

The arithmetic is verified with [Kani](https://model-checking.github.io/kani/).

| | |
|---|---|
| K1 | termination: every wait's measure is well-founded and strictly decreasing |
| K2 | the declared budget fits the counter type that holds it |
| K3 | every reachable composition fits its accumulator type |
| K4 | attainment: the reported maximum is a true maximum over the declared domain |
| K5 | unit soundness: combining units requires a declared conversion |
| K6 | provenance monotonicity: a result is at most as strong as its weakest input |
| K7 | interval soundness: the result interval is conservative |

K2, K5, K6 and K7 are small bounded harnesses. K1 and K4 want loop contracts,
because realistic budgets run to thousands of iterations and unrolling them is
not viable.

## Subcommands

Following the `calx-mill` idiom.

| | |
|---|---|
| `census` | inventory waits and windows from an ELF; emit a skeleton register |
| `check` | run the proof obligations against a register |
| `project` | worst-case cost of each declared composition |
| `attain` | report the witness: where each maximum occurs |
| `deadline` | compare projections against declared deadlines, by armed interval |
| `diff` | register against register, for regression across builds |

The register itself is a hand-authorable, diffable file declaring waits,
windows, deadlines, compositions and conversions. An ELF adapter emits a
skeleton register with `Extracted` provenance and blanks wherever a human must
still supply a budget or a measure.

## Relationship to calx-mill

[calx-mill](https://github.com/slartibardfast/calx-mill) is a sibling crate, not
a parent. It computes steady-state throughput and occupancy. Its arithmetic
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

## Status

Greenfield. The design is settled and the implementation is not yet written. The
milestones, decisions and verification lanes are governed by the host meta-repo
at [agentic-calx-telltale](https://github.com/slartibardfast/agentic-calx-telltale).

## Licence

Released into the public domain under the [Unlicense](LICENSE).
