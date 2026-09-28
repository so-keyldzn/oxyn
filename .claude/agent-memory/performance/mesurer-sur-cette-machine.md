---
name: mesurer-sur-cette-machine
description: "Tooling traps for measuring on oxyn's dev Mac: the load average is unusable, and criterion 0.8 has two behaviors that make a campaign look successful when it measured nothing"
metadata:
  type: project
---

Three traps observed on 2026-09-10 while setting up the first measurement
campaign.

**This machine's load average is not an indicator of quietness.**
Apple M1 Max, 10 cores: `loadavg` structurally stays between 15 and 50 there
(busy graphical session, ~59 user sessions, heavy applications open) while
`top` reports 55 to 71 % idle CPU time. Waiting for `loadavg < 4` before
measuring means waiting forever.

**Why:** a first waiting script looped without ever starting the measurement.

**How to apply:** the quietness gate that works here is
`pgrep -x rustc` and `pgrep -x cargo` empty **and**
`top -l 2 -n 0 -s 1 | grep "CPU usage" | tail -1` above ~45 % idle.
Under these conditions, two successive campaigns give medians within ≤ 4 % —
so figures valid to ±5 %, enough for budgets in milliseconds, not to arbitrate
an optimization that would promise 3 %.

**`criterion` launched without `--bench` measures nothing and does not say so.**
The executable switches to test mode and only prints `Testing …` / `Success`. A
campaign launched this way looks like a successful campaign.

**`BenchmarkGroup::sample_size` overrides `--sample-size` from the command line.**
A value written in the code cannot be tuned at invocation; you have to
recompile.

**Linear sampling is unusable beyond ~100 ms per iteration.**
`criterion` then asks for `n(n+1)/2` iterations for `n` samples. On a bench
that reads a one-million-row table (~210 ms per iteration),
`SamplingMode::Flat` + `sample_size(50)` gives 50 measurements in ~11 s, with
±1 % confidence intervals instead of ±10 % with ten linear samples.

See [[perimetre-cargo-fmt]] for the risk of collateral formatting when several
agents work in parallel.
